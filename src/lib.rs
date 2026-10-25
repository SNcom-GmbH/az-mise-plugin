pub mod archive;
pub mod auth;
pub mod config;
pub mod storage;

use anyhow::{Context, Result, bail, ensure};
use config::{Request, relative};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::HashSet, fs, io::Read, path::Path};
use storage::Storage;

/// Installs the Rustls ring provider once per process.
pub fn initialize_tls() {
	let _ = rustls::crypto::ring::default_provider().install_default();
}

pub async fn dispatch(store: &dyn Storage, request: &Request) -> Result<Value> {
	match request.action.as_str() {
		"versions" => {
			let tool = relative(request.tool.as_deref().context("tool is required")?)?;
			let bytes = store
				.read(&format!("{tool}/index.json"))
				.await
				.with_context(|| format!("Could not read {tool}/index.json"))?;
			let index: Value = serde_json::from_slice(&bytes)
				.with_context(|| format!("Invalid JSON in {tool}/index.json"))?;
			let versions = index["versions"]
				.as_array()
				.context("index.json needs a versions array")?;
			ensure!(
				!versions.is_empty(),
				"index.json needs a nonempty versions array"
			);
			let mut seen = HashSet::new();
			for entry in versions {
				let version = entry
					.as_str()
					.or_else(|| entry["version"].as_str())
					.context("Invalid version")?;
				relative(version)?;
				ensure!(
					!version.contains('/') && seen.insert(version),
					"Invalid or duplicate release version"
				);
			}
			Ok(json!({"versions": versions}))
		}
		"install" => install(store, request).await,
		"list" => store
			.list(&request.prefix)
			.await
			.with_context(|| format!("Could not list blobs with prefix {:?}", request.prefix)),
		"read" => {
			let blob = relative(request.blob.as_deref().context("blob is required")?)?;
			let bytes = store
				.read(blob)
				.await
				.with_context(|| format!("Could not read blob {blob}"))?;
			let text = String::from_utf8(bytes)
				.with_context(|| format!("Blob {blob} is not valid UTF-8"))?;
			Ok(json!({"text": text}))
		}
		"download" => {
			let blob = relative(request.blob.as_deref().context("blob is required")?)?;
			let file = request.file.as_deref().context("file is required")?;
			// The storage implementation uses create_new; existing files are never replaced.
			store
				.download(blob, file)
				.await
				.with_context(|| format!("Could not download {blob} to {}", file.display()))?;
			Ok(json!({"downloaded": blob}))
		}
		"upload" => {
			let blob = relative(request.blob.as_deref().context("blob is required")?)?;
			let file = request.file.as_deref().context("file is required")?;
			store
				.upload(blob, file, request.overwrite)
				.await
				.with_context(|| format!("Could not upload {} to {blob}", file.display()))?;
			Ok(json!({"uploaded": blob}))
		}
		_ => bail!("Unknown action"),
	}
}

async fn install(store: &dyn Storage, request: &Request) -> Result<Value> {
	let tool = relative(request.tool.as_deref().context("tool is required")?)?;
	let version = relative(request.version.as_deref().context("version is required")?)?;
	ensure!(!version.contains('/'), "Version must be one path component");
	let platform = request
		.platform
		.as_deref()
		.context("platform is required")?;
	let release_bytes = store
		.read(&format!("{tool}/{version}/release.json"))
		.await
		.with_context(|| format!("Could not read {tool}/{version}/release.json"))?;
	let release: Value = serde_json::from_slice(&release_bytes)
		.with_context(|| format!("Invalid JSON in {tool}/{version}/release.json"))?;
	let asset = &release["platforms"][platform];
	ensure!(!asset.is_null(), "Release has no artifact for {platform}");
	let name = relative(
		asset["file"]
			.as_str()
			.context("Artifact file is required")?,
	)?;
	ensure!(!name.contains('/'), "Artifact file must be a filename");
	let expected = hex::decode(asset["sha256"].as_str().context("SHA-256 is required")?)
		.context("Artifact SHA-256 must contain 64 hexadecimal digits")?;
	ensure!(expected.len() == 32, "Artifact requires a SHA-256 digest");
	let download = request
		.download_path
		.as_deref()
		.context("download_path is required")?;
	let install = request
		.install_path
		.as_deref()
		.context("install_path is required")?;
	let parent = install.parent().context("Invalid install_path")?;

	// Reject an unusable destination before downloading a potentially large artifact.
	// The target is checked again immediately before the atomic rename.
	ensure_install_target(install)
		.with_context(|| format!("Invalid install_path {}", install.display()))?;
	fs::create_dir_all(parent).with_context(|| {
		format!(
			"Could not create install parent directory {}",
			parent.display()
		)
	})?;
	fs::create_dir_all(download)
		.with_context(|| format!("Could not create download directory {}", download.display()))?;
	let temporary = tempfile::tempdir_in(download).with_context(|| {
		format!(
			"Could not create a temporary directory in {}",
			download.display()
		)
	})?;
	let archive = temporary.path().join(name);
	let blob = format!("{tool}/{version}/{name}");
	store
		.download(&blob, &archive)
		.await
		.with_context(|| format!("Could not download artifact {blob}"))?;
	let actual = sha256_digest(&archive)
		.with_context(|| format!("Could not hash artifact {}", archive.display()))?;
	ensure!(
		actual.as_slice() == expected.as_slice(),
		"Artifact SHA-256 mismatch; nothing was installed"
	);
	let stage = tempfile::Builder::new()
		.prefix(".azure-ac-stage-")
		.tempdir_in(parent)
		.with_context(|| {
			format!(
				"Could not create a staging directory in {}",
				parent.display()
			)
		})?;
	archive::extract(&archive, stage.path())
		.with_context(|| format!("Could not extract artifact {}", archive.display()))?;
	let bin = stage.path().join("bin");
	ensure!(
		bin.is_dir(),
		"Artifact must contain bin/ at the archive root"
	);
	#[cfg(unix)]
	make_bin_executable(&bin)
		.with_context(|| format!("Could not make files in {} executable", bin.display()))?;
	ensure_install_target(install)
		.with_context(|| format!("Invalid install_path {}", install.display()))?;
	if install.exists() {
		fs::remove_dir(install).with_context(|| {
			format!(
				"Could not replace the empty install directory {}",
				install.display()
			)
		})?;
	}
	fs::rename(stage.path(), install).with_context(|| {
		format!(
			"Could not atomically finalize installation at {}",
			install.display()
		)
	})?;
	Ok(json!({}))
}

fn sha256_digest(path: &Path) -> Result<Vec<u8>> {
	let mut file = fs::File::open(path)?;
	let mut checksum = Sha256::new();
	let mut buffer = [0u8; 65536];
	loop {
		let size = file.read(&mut buffer)?;
		if size == 0 {
			break;
		}
		checksum.update(&buffer[..size]);
	}
	Ok(checksum.finalize().to_vec())
}

fn ensure_install_target(install: &Path) -> Result<()> {
	if install.exists() {
		ensure!(
			fs::symlink_metadata(install)?.is_dir() && fs::read_dir(install)?.next().is_none(),
			"Refusing to overwrite a nonempty install directory"
		);
	}
	Ok(())
}

#[cfg(unix)]
fn make_bin_executable(bin: &Path) -> Result<()> {
	use std::os::unix::fs::PermissionsExt;
	for entry in fs::read_dir(bin)? {
		let path = entry?.path();
		let metadata = fs::symlink_metadata(&path)?;
		if metadata.is_file() {
			// Windows-created zip/tar entries often carry no exec bit at all.
			// Keep existing permissions but guarantee owner read and execution.
			let mode = (metadata.permissions().mode() | 0o511) & 0o777;
			fs::set_permissions(&path, fs::Permissions::from_mode(mode))?;
		}
	}
	Ok(())
}
