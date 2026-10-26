mod common;

use anyhow::{Result, bail};
use azure_blob::{
	config::Request,
	dispatch,
	storage::{Storage, VersionedBlob},
};
use common::FileStorage;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, fs, path::Path, sync::Mutex};

struct Store {
	inner: FileStorage,
	writes: Mutex<Vec<String>>,
	fail: Mutex<Option<String>>,
	races: Mutex<HashMap<String, Value>>,
}

impl Store {
	fn new(root: &Path) -> Self {
		Self {
			inner: FileStorage(root.join("storage")),
			writes: Mutex::new(Vec::new()),
			fail: Mutex::new(None),
			races: Mutex::new(HashMap::new()),
		}
	}
	fn maybe_fail(&self, name: &str) -> Result<()> {
		let mut fail = self.fail.lock().expect("test mutex");
		if fail.as_deref() == Some(name) {
			*fail = None;
			bail!("Injected write failure");
		}
		Ok(())
	}
	fn put(&self, name: &str, value: &Value) -> Result<()> {
		let path = self.inner.0.join(name);
		fs::create_dir_all(path.parent().expect("parent"))?;
		fs::write(path, serde_json::to_vec(value)?)?;
		Ok(())
	}
	async fn document(&self, name: &str) -> Result<Value> {
		Ok(serde_json::from_slice(&self.inner.read(name).await?)?)
	}
}

#[async_trait::async_trait]
impl Storage for Store {
	async fn read(&self, name: &str) -> Result<Vec<u8>> {
		self.inner.read(name).await
	}
	async fn download(&self, name: &str, file: &Path, max: Option<u64>) -> Result<()> {
		self.inner.download(name, file, max).await
	}
	async fn upload(&self, name: &str, file: &Path, overwrite: bool) -> Result<()> {
		assert!(!overwrite, "Publishing must never overwrite artifacts");
		self.maybe_fail("artifact")?;
		self.inner.upload(name, file, overwrite).await?;
		self.writes.lock().expect("test mutex").push(name.into());
		Ok(())
	}
	async fn list(&self, prefix: &str, limit: usize) -> Result<Value> {
		self.inner.list(prefix, limit).await
	}
	async fn read_versioned(&self, name: &str) -> Result<Option<VersionedBlob>> {
		self.inner.read_versioned(name).await
	}
	async fn write_metadata(&self, name: &str, bytes: &[u8], etag: Option<&str>) -> Result<()> {
		self.maybe_fail(name)?;
		let raced = self.races.lock().expect("test mutex").remove(name);
		if let Some(value) = raced {
			// Simulate another publisher committing after our read, before our write.
			self.put(name, &value)?;
		}
		self.inner.write_metadata(name, bytes, etag).await?;
		self.writes.lock().expect("test mutex").push(name.into());
		Ok(())
	}
}

fn request(root: &Path, platform: &str) -> Result<Request> {
	let file = root.join(if platform == "windows-amd64" {
		"hello.exe"
	} else {
		"hello"
	});
	fs::write(&file, b"test binary bytes")?;
	Ok(serde_json::from_value(json!({
		"action": "publish", "tool": "hello", "version": "1.0.0", "platform": platform,
		"file": file, "options": {"storage_url": "https://account.blob.core.windows.net/tools"}
	}))?)
}

#[tokio::test]
async fn published_binaries_round_trip_through_install() -> Result<()> {
	for platform in ["linux-amd64", "windows-amd64"] {
		let root = tempfile::tempdir()?;
		let store = Store::new(root.path());
		let mut request = request(root.path(), platform)?;
		let published = dispatch(&store, &request).await?;
		assert_eq!(published["published"], "hello");
		assert_eq!(published["platform"], platform);
		let release = store.document("hello/1.0.0/release.json").await?;
		let asset = &release["platforms"][platform];
		let artifact = format!("hello/1.0.0/{}", asset["file"].as_str().expect("filename"));
		let bytes = store.read(&artifact).await?;
		assert_eq!(asset["size"], bytes.len() as u64);
		assert_eq!(asset["sha256"], hex::encode(Sha256::digest(&bytes)));
		assert_eq!(
			store.document("hello/index.json").await?,
			json!({"versions": ["1.0.0"]})
		);
		assert_eq!(
			*store.writes.lock().expect("test mutex"),
			vec![
				artifact,
				"hello/1.0.0/release.json".into(),
				"hello/index.json".into()
			]
		);
		request.action = "install".into();
		request.download_path = Some(root.path().join("download"));
		request.install_path = Some(root.path().join("install"));
		dispatch(&store, &request).await?;
		let binary = root.path().join(if platform == "windows-amd64" {
			"install/bin/hello.exe"
		} else {
			"install/bin/hello"
		});
		assert_eq!(fs::read(&binary)?, b"test binary bytes");
		#[cfg(unix)]
		{
			use std::os::unix::fs::PermissionsExt;
			assert_ne!(fs::metadata(binary)?.permissions().mode() & 0o111, 0);
		}
	}
	Ok(())
}

#[tokio::test]
async fn adding_platform_preserves_release_and_index_metadata() -> Result<()> {
	let root = tempfile::tempdir()?;
	let store = Store::new(root.path());
	let linux = request(root.path(), "linux-amd64")?;
	dispatch(&store, &linux).await?;
	let mut release = store.document("hello/1.0.0/release.json").await?;
	let original = release["platforms"]["linux-amd64"].clone();
	release["notes"] = json!("keep release notes");
	store.put("hello/1.0.0/release.json", &release)?;
	let index = json!({"versions": [{"version": "0.9.0", "metadata": "keep"}, "1.0.0"], "notes": "keep index notes"});
	store.put("hello/index.json", &index)?;
	dispatch(&store, &request(root.path(), "windows-amd64")?).await?;
	let release = store.document("hello/1.0.0/release.json").await?;
	assert_eq!(release["platforms"]["linux-amd64"], original);
	assert!(release["platforms"]["windows-amd64"].is_object());
	assert_eq!(release["notes"], "keep release notes");
	assert_eq!(store.document("hello/index.json").await?, index);
	Ok(())
}

#[tokio::test]
async fn identical_publication_is_idempotent_but_changed_binary_is_rejected() -> Result<()> {
	let root = tempfile::tempdir()?;
	let store = Store::new(root.path());
	let request = request(root.path(), "linux-amd64")?;
	let first = dispatch(&store, &request).await?;
	assert_eq!(dispatch(&store, &request).await?, first);
	assert_eq!(store.writes.lock().expect("test mutex").len(), 3);
	fs::write(request.file.as_ref().expect("binary"), b"different binary")?;
	let error = dispatch(&store, &request)
		.await
		.expect_err("existing platform");
	assert!(error.to_string().contains("use a new version"));
	assert_eq!(store.writes.lock().expect("test mutex").len(), 3);
	Ok(())
}

#[tokio::test]
async fn failed_writes_do_not_advertise_incomplete_releases_and_can_resume() -> Result<()> {
	for failure in ["artifact", "hello/1.0.0/release.json", "hello/index.json"] {
		let root = tempfile::tempdir()?;
		let store = Store::new(root.path());
		let request = request(root.path(), "linux-amd64")?;
		*store.fail.lock().expect("test mutex") = Some(failure.into());
		assert!(dispatch(&store, &request).await.is_err());
		assert!(!store.inner.0.join("hello/index.json").exists());
		if failure != "hello/index.json" {
			assert!(!store.inner.0.join("hello/1.0.0/release.json").exists());
		}
		dispatch(&store, &request).await?;
		assert_eq!(
			store.document("hello/index.json").await?,
			json!({"versions": ["1.0.0"]})
		);
	}
	Ok(())
}

#[tokio::test]
async fn concurrent_metadata_changes_are_merged_instead_of_lost() -> Result<()> {
	let root = tempfile::tempdir()?;
	let store = Store::new(root.path());
	let request = request(root.path(), "linux-amd64")?;
	let windows = json!({"file": "existing.zip", "sha256": "ab".repeat(32), "size": 123});
	store.races.lock().expect("test mutex").extend([
		(
			"hello/1.0.0/release.json".into(),
			json!({"platforms": {"windows-amd64": windows}, "notes": "concurrent"}),
		),
		(
			"hello/index.json".into(),
			json!({"versions": [{"version": "2.0.0", "notes": "concurrent"}]}),
		),
	]);
	dispatch(&store, &request).await?;
	let release = store.document("hello/1.0.0/release.json").await?;
	assert_eq!(release["platforms"]["windows-amd64"], windows);
	assert!(release["platforms"]["linux-amd64"].is_object());
	assert_eq!(release["notes"], "concurrent");
	assert_eq!(
		store.document("hello/index.json").await?,
		json!({"versions": ["1.0.0", {"version": "2.0.0", "notes": "concurrent"}]})
	);
	Ok(())
}

#[tokio::test]
async fn concurrent_conflicting_platform_is_not_overwritten_or_indexed() -> Result<()> {
	let root = tempfile::tempdir()?;
	let store = Store::new(root.path());
	let request = request(root.path(), "linux-amd64")?;
	let competing = json!({"platforms": {"linux-amd64": {"file": "other.zip"}}});
	store
		.races
		.lock()
		.expect("test mutex")
		.insert("hello/1.0.0/release.json".into(), competing.clone());
	assert!(dispatch(&store, &request).await.is_err());
	assert_eq!(store.document("hello/1.0.0/release.json").await?, competing);
	assert!(!store.inner.0.join("hello/index.json").exists());
	Ok(())
}

#[tokio::test]
async fn malformed_metadata_fails_before_any_write() -> Result<()> {
	for (blob, value) in [
		("hello/1.0.0/release.json", json!(null)),
		("hello/1.0.0/release.json", json!({"platforms": []})),
		("hello/index.json", json!({"versions": null})),
		("hello/index.json", json!({"versions": ["1.0.0", "1.0.0"]})),
		("hello/index.json", json!({"versions": ["../escape"]})),
		("hello/index.json", json!({"versions": [42]})),
	] {
		let root = tempfile::tempdir()?;
		let store = Store::new(root.path());
		let request = request(root.path(), "linux-amd64")?;
		store.put(blob, &value)?;
		assert!(
			dispatch(&store, &request).await.is_err(),
			"accepted {value}"
		);
		assert!(store.writes.lock().expect("test mutex").is_empty());
		assert_eq!(store.document(blob).await?, value);
	}
	Ok(())
}

#[tokio::test]
async fn invalid_publication_inputs_never_write() -> Result<()> {
	let root = tempfile::tempdir()?;
	let store = Store::new(root.path());
	for (field, invalid) in [
		("tool", "../escape"),
		("version", "../escape"),
		("platform", "darwin-arm64"),
	] {
		let request = request(root.path(), "linux-amd64")?;
		let mut value = serde_json::to_value(request)?;
		value[field] = json!(invalid);
		assert!(
			dispatch(&store, &serde_json::from_value(value)?)
				.await
				.is_err()
		);
	}
	let mut request = request(root.path(), "linux-amd64")?;
	request.overwrite = true;
	assert!(dispatch(&store, &request).await.is_err());
	request.overwrite = false;
	request.file = Some(root.path().join("missing"));
	assert!(dispatch(&store, &request).await.is_err());
	request.file = Some(root.path().to_owned());
	assert!(dispatch(&store, &request).await.is_err());
	let reserved = root.path().join("NUL.exe");
	if fs::write(&reserved, b"not portable").is_ok() {
		request.file = Some(reserved);
		assert!(dispatch(&store, &request).await.is_err());
	}
	assert!(store.writes.lock().expect("test mutex").is_empty());
	Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn publishing_rejects_symlinks_and_oversized_binaries() -> Result<()> {
	let root = tempfile::tempdir()?;
	let store = Store::new(root.path());
	let mut request = request(root.path(), "linux-amd64")?;
	let link = root.path().join("link");
	std::os::unix::fs::symlink(request.file.as_ref().expect("binary"), &link)?;
	request.file = Some(link);
	assert!(dispatch(&store, &request).await.is_err());
	let huge = root.path().join("huge");
	fs::File::create(&huge)?.set_len(4 * 1024 * 1024 * 1024 + 1)?;
	request.file = Some(huge);
	assert!(dispatch(&store, &request).await.is_err());
	assert!(store.writes.lock().expect("test mutex").is_empty());
	Ok(())
}

#[tokio::test]
async fn corrupt_existing_artifact_is_not_trusted_on_retry() -> Result<()> {
	let root = tempfile::tempdir()?;
	let store = Store::new(root.path());
	let request = request(root.path(), "linux-amd64")?;
	*store.fail.lock().expect("test mutex") = Some("hello/1.0.0/release.json".into());
	assert!(dispatch(&store, &request).await.is_err());
	let artifact = store.writes.lock().expect("test mutex")[0].clone();
	fs::write(store.inner.0.join(artifact), b"corrupt")?;
	let error = dispatch(&store, &request)
		.await
		.expect_err("corrupt artifact");
	assert!(error.to_string().contains("checksum"));
	assert!(!store.inner.0.join("hello/index.json").exists());
	Ok(())
}
