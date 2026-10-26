use crate::{
	MAX_ARTIFACT_BYTES,
	config::{Request, filename, portable_path},
	sha256_digest,
	storage::{Storage, WriteConflict},
};
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::{collections::HashSet, fs, io::Read, path::Path};

const UPDATE_ATTEMPTS: usize = 8;

pub(crate) async fn publish(store: &dyn Storage, request: &Request) -> Result<Value> {
	let tool = portable_path(request.tool.as_deref().context("tool is required")?)?;
	let version = filename(request.version.as_deref().context("version is required")?)?;
	let platform = request
		.platform
		.as_deref()
		.context("platform is required")?;
	ensure!(
		["linux-amd64", "windows-amd64"].contains(&platform),
		"platform must be linux-amd64 or windows-amd64"
	);
	ensure!(
		!request.overwrite,
		"Published platforms cannot be overwritten; use a new version"
	);
	let source = request.file.as_deref().context("file is required")?;
	let temporary =
		tempfile::tempdir().context("Could not create publication staging directory")?;
	let archive = temporary.path().join("artifact.zip");
	package(source, &archive)?;
	let size = fs::metadata(&archive)?.len();
	ensure!(
		size <= MAX_ARTIFACT_BYTES,
		"Packaged artifact exceeds the {MAX_ARTIFACT_BYTES}-byte limit"
	);
	let digest = hex::encode(sha256_digest(&archive)?);
	// Content-addressed artifacts allow safe retries without replacing a published blob.
	let name = format!("{platform}-{digest}.zip");
	let artifact = json!({"file": name, "sha256": digest, "size": size});
	let release_blob = format!("{tool}/{version}/release.json");
	let index_blob = format!("{tool}/index.json");

	// Reject malformed metadata and conflicting releases before uploading anything.
	let (mut release, _) = read_document(store, &release_blob, json!({"platforms": {}})).await?;
	merge_platform(&mut release, platform, &artifact)?;
	let (mut index, _) = read_document(store, &index_blob, json!({"versions": []})).await?;
	merge_version(&mut index, version)?;

	let artifact_blob = format!("{tool}/{version}/{name}");
	match store.upload(&artifact_blob, &archive, false).await {
		Ok(()) => {}
		Err(error) if error.is::<WriteConflict>() => {
			// An earlier attempt may have uploaded the archive but not the index.
			// Verify existing bytes instead of trusting a content-addressed filename.
			let existing = temporary.path().join("existing.zip");
			store
				.download(&artifact_blob, &existing, Some(size))
				.await?;
			ensure!(
				fs::metadata(&existing)?.len() == size
					&& hex::encode(sha256_digest(&existing)?) == digest,
				"Existing artifact does not match its checksum; refusing publication"
			);
		}
		Err(error) => return Err(error).context("Could not upload publication artifact"),
	}
	update_document(store, &release_blob, json!({"platforms": {}}), |release| {
		merge_platform(release, platform, &artifact)
	})
	.await
	.context("Could not publish release.json; rerun the same command to resume")?;
	// Visibility through mise is the final step. Never announce a missing manifest.
	update_document(store, &index_blob, json!({"versions": []}), |index| {
		merge_version(index, version)
	})
	.await
	.context("Could not publish index.json; rerun the same command to resume")?;
	Ok(json!({"published": tool, "version": version, "platform": platform, "artifact": artifact}))
}

fn package(source: &Path, target: &Path) -> Result<()> {
	let name = filename(
		source
			.file_name()
			.and_then(|name| name.to_str())
			.context("Binary needs a UTF-8 filename")?,
	)?;
	ensure!(
		fs::symlink_metadata(source)
			.with_context(|| format!("Could not read {}", source.display()))?
			.is_file(),
		"Publish expects a regular binary file, not a directory or symlink"
	);
	let input = fs::File::open(source)?;
	ensure!(
		input.metadata()?.len() <= MAX_ARTIFACT_BYTES,
		"Binary exceeds the {MAX_ARTIFACT_BYTES}-byte limit"
	);
	let mut zip = zip::ZipWriter::new(fs::File::create(target)?);
	zip.start_file(
		format!("bin/{name}"),
		zip::write::SimpleFileOptions::default()
			.compression_method(zip::CompressionMethod::Deflated)
			.last_modified_time(zip::DateTime::default())
			.unix_permissions(0o755),
	)?;
	let copied = std::io::copy(&mut input.take(MAX_ARTIFACT_BYTES + 1), &mut zip)?;
	ensure!(
		copied <= MAX_ARTIFACT_BYTES,
		"Binary exceeds the {MAX_ARTIFACT_BYTES}-byte limit"
	);
	zip.finish()?;
	Ok(())
}

async fn read_document(
	store: &dyn Storage,
	blob: &str,
	default: Value,
) -> Result<(Value, Option<String>)> {
	match store
		.read_versioned(blob)
		.await
		.with_context(|| format!("Could not read {blob}"))?
	{
		Some(snapshot) => Ok((
			serde_json::from_slice(&snapshot.bytes)
				.with_context(|| format!("Invalid JSON in {blob}"))?,
			Some(snapshot.etag),
		)),
		None => Ok((default, None)),
	}
}

async fn update_document(
	store: &dyn Storage,
	blob: &str,
	default: Value,
	merge: impl Fn(&mut Value) -> Result<bool>,
) -> Result<()> {
	for _ in 0..UPDATE_ATTEMPTS {
		let (mut value, etag) = read_document(store, blob, default.clone()).await?;
		if !merge(&mut value)? {
			return Ok(());
		}
		match store
			.write_metadata(blob, &serde_json::to_vec_pretty(&value)?, etag.as_deref())
			.await
		{
			Ok(()) => return Ok(()),
			Err(error) if error.is::<WriteConflict>() => continue,
			Err(error) => return Err(error).with_context(|| format!("Could not write {blob}")),
		}
	}
	bail!("{blob} kept changing during publication; retry later")
}

fn merge_platform(release: &mut Value, platform: &str, artifact: &Value) -> Result<bool> {
	let platforms = release
		.get_mut("platforms")
		.and_then(Value::as_object_mut)
		.context("release.json needs a platforms object")?;
	if let Some(existing) = platforms.get(platform) {
		ensure!(
			existing == artifact,
			"Release already has a different artifact for {platform}; use a new version"
		);
		return Ok(false);
	}
	platforms.insert(platform.into(), artifact.clone());
	Ok(true)
}

fn merge_version(index: &mut Value, version: &str) -> Result<bool> {
	let versions = index
		.get_mut("versions")
		.and_then(Value::as_array_mut)
		.context("index.json needs a versions array")?;
	let mut seen = HashSet::new();
	for entry in versions.iter() {
		let name = entry
			.as_str()
			.or_else(|| entry["version"].as_str())
			.context("Invalid version in index.json")?;
		filename(name)?;
		ensure!(seen.insert(name), "Duplicate release version {name:?}");
	}
	if seen.contains(version) {
		return Ok(false);
	}
	versions.insert(0, json!(version));
	Ok(true)
}
