pub mod archive;
pub mod auth;
pub mod config;
pub mod storage;

use anyhow::{Context, Result, bail, ensure};
use config::{Request, relative};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::HashSet, fs, io::Read};
use storage::Storage;

pub fn initialize_tls() {
	let _ = rustls::crypto::ring::default_provider().install_default();
}

pub async fn dispatch(store: &dyn Storage, request: &Request) -> Result<Value> {
	match request.action.as_str() {
		"versions" => {
			let tool = relative(request.tool.as_deref().context("tool is required")?)?;
			let index: Value =
				serde_json::from_slice(&store.read(&format!("{tool}/index.json")).await?)?;
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
		"list" => store.list(&request.prefix).await,
		"read" => {
			let blob = relative(request.blob.as_deref().context("blob is required")?)?;
			let bytes = store.read(blob).await?;
			Ok(json!({"text": String::from_utf8(bytes).context("Blob is not UTF-8")?}))
		}
		"download" => {
			let blob = relative(request.blob.as_deref().context("blob is required")?)?;
			let file = request.file.as_deref().context("file is required")?;
			// The storage implementation uses create_new; existing files are never replaced.
			store.download(blob, file).await?;
			Ok(json!({"downloaded": blob}))
		}
		"upload" => {
			let blob = relative(request.blob.as_deref().context("blob is required")?)?;
			store
				.upload(
					blob,
					request.file.as_deref().context("file is required")?,
					request.overwrite,
				)
				.await?;
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
	let release: Value = serde_json::from_slice(
		&store
			.read(&format!("{tool}/{version}/release.json"))
			.await?,
	)?;
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
	fs::create_dir_all(download)?;
	let temporary = tempfile::tempdir_in(download)?;
	let archive = temporary.path().join(name);
	store
		.download(&format!("{tool}/{version}/{name}"), &archive)
		.await?;
	let mut file = fs::File::open(&archive)?;
	let mut checksum = Sha256::new();
	let mut buffer = [0u8; 65536];
	loop {
		let size = file.read(&mut buffer)?;
		if size == 0 {
			break;
		}
		checksum.update(&buffer[..size]);
	}
	ensure!(
		checksum.finalize().as_slice() == expected,
		"Artifact SHA-256 mismatch; nothing was installed"
	);
	let parent = install.parent().context("Invalid install_path")?;
	fs::create_dir_all(parent)?;
	let stage = tempfile::Builder::new()
		.prefix(".azure-ac-stage-")
		.tempdir_in(parent)?;
	archive::extract(&archive, stage.path())?;
	ensure!(
		stage.path().join("bin").is_dir(),
		"Artifact must contain bin/ at the archive root"
	);
	if install.exists() {
		ensure!(
			fs::symlink_metadata(install)?.is_dir() && fs::read_dir(install)?.next().is_none(),
			"Refusing to overwrite a nonempty install directory"
		);
		fs::remove_dir(install)?;
	}
	fs::rename(stage.path(), install).context("Could not atomically finalize installation")?;
	Ok(json!({}))
}
