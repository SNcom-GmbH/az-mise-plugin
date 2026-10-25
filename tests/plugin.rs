mod common;

use anyhow::Result;
use azure_ac::{
	archive,
	config::{Options, Request, destination},
	dispatch,
	storage::Storage,
};
use common::FileStorage;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
	fs,
	io::{Cursor, Write},
	path::Path,
};

fn options() -> Options {
	serde_json::from_value(json!({"storage_url": "https://account.blob.core.windows.net/tools"}))
		.expect("fixture")
}

fn request(root: &Path, action: &str) -> Request {
	serde_json::from_value(json!({
		"action": action, "options": options(), "tool": "hello", "version": "1.0.0",
		"platform": "linux-amd64", "download_path": root.join("downloads with spaces"),
		"install_path": root.join("install with spaces")
	}))
	.expect("fixture")
}

fn zip(name: &str, link: bool) -> Vec<u8> {
	let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
	let options = zip::write::SimpleFileOptions::default()
		.compression_method(zip::CompressionMethod::Stored)
		.unix_permissions(0o755);
	if link {
		writer
			.add_symlink(name, "../../escape", options)
			.expect("fixture");
	} else {
		writer.start_file(name, options).expect("fixture");
		writer.write_all(b"hello world").expect("fixture");
	}
	writer.finish().expect("fixture").into_inner()
}

fn zip_without_unix_mode(name: &str) -> Vec<u8> {
	let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
	let options =
		zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
	writer.start_file(name, options).expect("fixture");
	writer.write_all(b"hello world").expect("fixture");
	writer.finish().expect("fixture").into_inner()
}

fn tar_entry(name: &str, data: &[u8]) -> Result<Vec<u8>> {
	let mut builder = tar::Builder::new(Vec::new());
	let mut header = tar::Header::new_gnu();
	header.set_size(data.len() as u64);
	header.set_mode(0o755);
	header.set_cksum();
	builder.append_data(&mut header, name, Cursor::new(data))?;
	Ok(builder.into_inner()?)
}

// The tar crate refuses to build paths containing `..`, so write the raw
// header ourselves to exercise the extractor's traversal checks.
fn raw_tar_entry(name: &str, data: &[u8]) -> Result<Vec<u8>> {
	assert!(name.len() <= 100, "fixture name too long");
	let mut builder = tar::Builder::new(Vec::new());
	let mut header = tar::Header::new_gnu();
	header.set_size(data.len() as u64);
	header.set_mode(0o755);
	header.set_entry_type(tar::EntryType::Regular);
	header.as_mut_bytes()[..name.len()].copy_from_slice(name.as_bytes());
	header.set_cksum();
	builder.append(&header, data)?;
	Ok(builder.into_inner()?)
}

fn put(root: &Path, name: &str, bytes: &[u8]) {
	let path = root.join(name);
	fs::create_dir_all(path.parent().expect("fixture")).expect("fixture");
	fs::write(path, bytes).expect("fixture");
}

fn release(root: &Path, bytes: &[u8], checksum: Option<&str>) {
	put(root, "hello/1.0.0/hello.zip", bytes);
	put(root, "hello/1.0.0/release.json", json!({"platforms": {"linux-amd64": {
		"file": "hello.zip", "sha256": checksum.map(str::to_owned).unwrap_or_else(|| hex::encode(Sha256::digest(bytes)))
	}}}).to_string().as_bytes());
}

#[test]
fn validate_storage_endpoints_before_authentication() {
	assert!(options().validate().is_ok());
	for url in [
		"http://account.blob.core.windows.net/tools",
		"https://evil.invalid/tools",
		"https://account.blob.core.windows.net.evil.invalid/tools",
		"https://user@account.blob.core.windows.net/tools",
		"https://account.blob.core.windows.net/tools?sig=secret",
		"https://account.blob.core.windows.net/tools/path",
		"https://account.blob.core.windows.net/a/../tools",
		"https://account.blob.core.windows.net/t--ools",
		"https://account.blob.core.windows.net/x",
	] {
		let mut o = options();
		o.storage_url = url.into();
		assert!(o.validate().is_err(), "accepted {url}");
	}
}

#[test]
fn browser_and_device_modes_are_removed() {
	for auth in ["browser", "device"] {
		let mut o = options();
		o.auth = auth.into();
		assert!(o.validate().is_err());
	}
}

#[test]
fn reject_unsafe_and_nonportable_paths() {
	for name in [
		"../escape",
		"/absolute",
		"bin/../escape",
		"bin\\escape",
		"bin/NUL",
		"bin/com1.exe",
		"bin/file:ads",
		"bin/end.",
		"bin/end ",
		"bin/%2e%2e",
		"bin/a?b",
		"bin/a\nb",
	] {
		assert!(
			destination(Path::new("/tmp/test"), name).is_err(),
			"accepted {name}"
		);
	}
}

#[tokio::test]
async fn versions_preserve_order_and_metadata() -> Result<()> {
	let root = tempfile::tempdir()?;
	let versions = json!(["1.9.0", {"version": "1.10.0", "created_at": "2026-10-01T00:00:00Z"}]);
	put(
		root.path(),
		"hello/index.json",
		json!({"versions": versions}).to_string().as_bytes(),
	);
	assert_eq!(
		dispatch(
			&FileStorage(root.path().into()),
			&request(root.path(), "versions")
		)
		.await?,
		json!({"versions": versions})
	);
	Ok(())
}

#[tokio::test]
async fn invalid_versions_are_rejected() -> Result<()> {
	let root = tempfile::tempdir()?;
	for versions in [
		json!([]),
		json!(["../escape"]),
		json!(["1", "1"]),
		json!(["a/b"]),
	] {
		put(
			root.path(),
			"hello/index.json",
			json!({"versions": versions}).to_string().as_bytes(),
		);
		assert!(
			dispatch(
				&FileStorage(root.path().into()),
				&request(root.path(), "versions")
			)
			.await
			.is_err()
		);
	}
	Ok(())
}

#[tokio::test]
async fn install_zip_with_checksum_and_executable_mode() -> Result<()> {
	let root = tempfile::tempdir()?;
	release(root.path(), &zip("bin/hello", false), None);
	let r = request(root.path(), "install");
	assert_eq!(
		dispatch(&FileStorage(root.path().into()), &r).await?,
		json!({})
	);
	let binary = r.install_path.as_ref().expect("fixture").join("bin/hello");
	assert_eq!(fs::read(&binary)?, b"hello world");
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt;
		assert_ne!(fs::metadata(binary)?.permissions().mode() & 0o111, 0);
	}
	Ok(())
}

#[tokio::test]
async fn install_zip_without_unix_mode_makes_bin_executable() -> Result<()> {
	let root = tempfile::tempdir()?;
	release(root.path(), &zip_without_unix_mode("bin/hello"), None);
	let r = request(root.path(), "install");
	dispatch(&FileStorage(root.path().into()), &r).await?;
	let binary = r.install_path.as_ref().expect("fixture").join("bin/hello");
	assert_eq!(fs::read(&binary)?, b"hello world");
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt;
		assert_ne!(
			fs::metadata(binary)?.permissions().mode() & 0o111,
			0,
			"zip entries without a Unix mode must still be executable"
		);
	}
	Ok(())
}

#[tokio::test]
async fn corrupt_artifact_installs_nothing() -> Result<()> {
	let root = tempfile::tempdir()?;
	release(root.path(), &zip("bin/hello", false), Some(&"0".repeat(64)));
	let r = request(root.path(), "install");
	let error = dispatch(&FileStorage(root.path().into()), &r)
		.await
		.expect_err("must fail");
	assert!(error.to_string().contains("SHA-256 mismatch"));
	assert!(!r.install_path.expect("fixture").exists());
	Ok(())
}

#[tokio::test]
async fn missing_platform_and_bin_are_rejected() -> Result<()> {
	let root = tempfile::tempdir()?;
	release(root.path(), &zip("hello", false), None);
	let r = request(root.path(), "install");
	let error = dispatch(&FileStorage(root.path().into()), &r)
		.await
		.expect_err("must fail");
	assert!(error.to_string().contains("bin/"));
	let mut r = r;
	r.platform = Some("unsupported-platform".into());
	assert!(
		dispatch(&FileStorage(root.path().into()), &r)
			.await
			.expect_err("must fail")
			.to_string()
			.contains("no artifact")
	);
	Ok(())
}

#[tokio::test]
async fn install_rejects_a_file_at_the_destination_before_download() -> Result<()> {
	let root = tempfile::tempdir()?;
	release(root.path(), &zip("bin/hello", false), None);
	let r = request(root.path(), "install");
	let install_path = r.install_path.clone().expect("fixture");
	let download_path = r.download_path.clone().expect("fixture");
	fs::write(&install_path, b"not a directory")?;
	assert!(
		dispatch(&FileStorage(root.path().into()), &r)
			.await
			.is_err()
	);
	assert!(!download_path.exists());
	assert_eq!(fs::read(install_path)?, b"not a directory");
	Ok(())
}

#[tokio::test]
async fn installation_never_overwrites_existing_content() -> Result<()> {
	let root = tempfile::tempdir()?;
	release(root.path(), &zip("bin/hello", false), None);
	let r = request(root.path(), "install");
	let download_path = r.download_path.clone().expect("fixture");
	put(
		r.install_path.as_ref().expect("fixture"),
		"user.txt",
		b"keep me",
	);
	assert!(
		dispatch(&FileStorage(root.path().into()), &r)
			.await
			.is_err()
	);
	assert!(
		!download_path.exists(),
		"destination validation must happen before the artifact download"
	);
	assert_eq!(
		fs::read(r.install_path.expect("fixture").join("user.txt"))?,
		b"keep me"
	);
	Ok(())
}

#[test]
fn zip_traversal_and_links_are_rejected() -> Result<()> {
	let root = tempfile::tempdir()?;
	for (name, link) in [
		("../escape", false),
		("/absolute", false),
		("bin/NUL", false),
		("bin/link", true),
	] {
		let archive = root.path().join("bad.zip");
		fs::write(&archive, zip(name, link))?;
		assert!(archive::extract(&archive, &root.path().join("stage")).is_err());
	}
	assert!(!root.path().join("escape").exists());
	Ok(())
}

#[test]
fn zip_case_insensitive_duplicate_paths_are_rejected() -> Result<()> {
	let root = tempfile::tempdir()?;
	let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
	for name in ["bin/hello", "BIN/HELLO"] {
		writer
			.start_file(
				name,
				zip::write::SimpleFileOptions::default()
					.compression_method(zip::CompressionMethod::Stored)
					.unix_permissions(0o755),
			)
			.expect("fixture");
		writer.write_all(b"hello").expect("fixture");
	}
	let archive = root.path().join("duplicate.zip");
	fs::write(&archive, writer.finish().expect("fixture").into_inner())?;
	let error = archive::extract(&archive, &root.path().join("stage"))
		.expect_err("duplicate paths must fail");
	assert!(error.to_string().contains("duplicate"));
	Ok(())
}

#[test]
fn tar_extracts_regular_files_but_rejects_links() -> Result<()> {
	let root = tempfile::tempdir()?;
	for link in [false, true] {
		let file = fs::File::create(root.path().join("test.tar.gz"))?;
		let gzip = flate2::write::GzEncoder::new(file, flate2::Compression::default());
		let mut tar = tar::Builder::new(gzip);
		let mut header = tar::Header::new_gnu();
		header.set_mode(0o755);
		if link {
			header.set_entry_type(tar::EntryType::Symlink);
			header.set_link_name("../../escape")?;
			header.set_size(0);
		} else {
			header.set_size(5);
		}
		header.set_cksum();
		tar.append_data(
			&mut header,
			"bin/hello",
			Cursor::new(if link {
				b"".as_slice()
			} else {
				b"hello".as_slice()
			}),
		)?;
		tar.into_inner()?.finish()?;
		let result = archive::extract(
			&root.path().join("test.tar.gz"),
			&root.path().join(if link { "links" } else { "regular" }),
		);
		assert_eq!(result.is_err(), link);
	}
	assert_eq!(fs::read(root.path().join("regular/bin/hello"))?, b"hello");
	Ok(())
}

#[test]
fn tar_path_traversal_is_rejected() -> Result<()> {
	let root = tempfile::tempdir()?;
	let tar = raw_tar_entry("../escape", b"hello")?;
	let file = fs::File::create(root.path().join("test.tar.gz"))?;
	let mut gzip = flate2::write::GzEncoder::new(file, flate2::Compression::default());
	gzip.write_all(&tar)?;
	gzip.finish()?;
	assert!(
		archive::extract(&root.path().join("test.tar.gz"), &root.path().join("stage")).is_err()
	);
	assert!(!root.path().join("escape").exists());
	Ok(())
}

#[test]
fn tar_xz_and_bz2_extract_regular_files() -> Result<()> {
	let root = tempfile::tempdir()?;
	let tar = tar_entry("bin/hello", b"hello")?;

	let mut xz = lzma_rust2::XzWriter::new(Vec::new(), lzma_rust2::XzOptions::default())?;
	xz.write_all(&tar)?;
	fs::write(root.path().join("test.tar.xz"), xz.finish()?)?;

	let mut bz2 = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::best());
	bz2.write_all(&tar)?;
	fs::write(root.path().join("test.tar.bz2"), bz2.finish()?)?;

	archive::extract(&root.path().join("test.tar.xz"), &root.path().join("xz"))?;
	archive::extract(&root.path().join("test.tar.bz2"), &root.path().join("bz2"))?;
	assert_eq!(fs::read(root.path().join("xz/bin/hello"))?, b"hello");
	assert_eq!(fs::read(root.path().join("bz2/bin/hello"))?, b"hello");
	Ok(())
}

#[tokio::test]
async fn install_replaces_an_empty_install_directory() -> Result<()> {
	let root = tempfile::tempdir()?;
	release(root.path(), &zip("bin/hello", false), None);
	let r = request(root.path(), "install");
	fs::create_dir_all(r.install_path.as_ref().expect("fixture"))?;
	dispatch(&FileStorage(root.path().into()), &r).await?;
	assert_eq!(
		fs::read(r.install_path.expect("fixture").join("bin/hello"))?,
		b"hello world"
	);
	Ok(())
}

#[tokio::test]
async fn read_rejects_non_utf8_blobs() -> Result<()> {
	let root = tempfile::tempdir()?;
	put(root.path(), "storage/blob.bin", &[0xff, 0x00]);
	let mut r = request(root.path(), "read");
	r.blob = Some("blob.bin".into());
	let error = dispatch(&FileStorage(root.path().join("storage")), &r)
		.await
		.expect_err("binary read must fail");
	assert!(error.to_string().contains("not valid UTF-8"));
	Ok(())
}

#[tokio::test]
async fn list_read_download_and_explicit_overwrite() -> Result<()> {
	let root = tempfile::tempdir()?;
	let store = FileStorage(root.path().join("storage"));
	put(root.path(), "source.txt", b"hello");
	let mut r = request(root.path(), "upload");
	r.blob = Some("notes.txt".into());
	r.file = Some(root.path().join("source.txt"));
	dispatch(&store, &r).await?;
	assert!(dispatch(&store, &r).await.is_err());
	r.overwrite = true;
	dispatch(&store, &r).await?;
	r.action = "read".into();
	assert_eq!(dispatch(&store, &r).await?, json!({"text": "hello"}));
	r.action = "list".into();
	assert_eq!(
		dispatch(&store, &r).await?,
		json!([{"name": "notes.txt", "size": 5}])
	);
	let output = root.path().join("output.txt");
	store.download("notes.txt", &output).await?;
	assert!(store.download("notes.txt", &output).await.is_err());
	assert_eq!(fs::read(output)?, b"hello");
	Ok(())
}

#[test]
fn hook_protocol_accepts_mise_options_and_null_paths() -> Result<()> {
	let request: Request = serde_json::from_value(
		json!({"action": "versions", "tool": "hello", "options": {
		"storage_url": options().storage_url, "tenant_id": "tenant", "client_id": "client", "arbitrary_mise_option": "false"
	}, "install_path": null, "download_path": null}),
	)?;
	assert!(request.options.validate().is_ok());
	let _: Value = serde_json::to_value(request)?;
	Ok(())
}
