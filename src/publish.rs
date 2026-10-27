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
	let version = resolve_publish_version(request)?;
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
	merge_version(&mut index, &version)?;

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
		merge_version(index, &version)
	})
	.await
	.context("Could not publish index.json; rerun the same command to resume")?;
	Ok(json!({"published": tool, "version": version, "platform": platform, "artifact": artifact}))
}

/// Resolves and validates a publication version without accessing Blob Storage.
pub fn resolve_publish_version(request: &Request) -> Result<String> {
	if let Some(version) = request.version.as_deref() {
		return Ok(filename(version)?.to_owned());
	}
	ensure!(
		request.platform.as_deref() == Some("windows-amd64"),
		"VERSION is required unless --platform windows-amd64 is used for automatic version detection"
	);
	let source = request.file.as_deref().context("file is required")?;
	detect_windows_version(source).context(
		"Could not detect a usable Windows FileVersion; supply VERSION explicitly (publish TOOL VERSION FILE --platform windows-amd64)",
	)
}

fn detect_windows_version(source: &Path) -> Result<String> {
	let input = open_source(source)?;
	let size = input.metadata()?.len() as usize;
	ensure!(size > 0, "Executable is empty");
	let mapping = pelite::FileMap::open(source).context("Could not read executable metadata")?;
	// FileMap rounds its Unix mapping to a page boundary. Exclude padding so
	// truncated resource data cannot be mistaken for bytes present in the file.
	let bytes = mapping
		.as_ref()
		.get(..size)
		.context("Executable changed size")?;
	let pe =
		pelite::PeFile::from_bytes(bytes).context("File is not a valid Windows PE executable")?;
	let resources = pe
		.resources()
		.context("Executable has no readable version resources")?;
	let info = resources
		.version_info()
		.context("Executable has no readable FileVersion metadata")?
		.file_info();
	if let Some(fixed) = info.fixed {
		ensure!(
			fixed.dwSignature == 0xFEEF04BD,
			"Invalid Windows version metadata signature"
		);
	}
	let mut versions = info
		.strings
		.values()
		.filter_map(|strings| strings.get("FileVersion"));
	let version = if let Some(version) = versions.next() {
		ensure!(
			versions.all(|other| other == version),
			"FileVersion differs between resource languages"
		);
		version.clone()
	} else {
		info.fixed
			.context("Executable has no FileVersion string or fixed file version")?
			.dwFileVersion
			.to_string()
	};
	ensure!(
		!version.is_empty()
			&& version
				.bytes()
				.all(|byte| byte.is_ascii_alphanumeric() || b".-_+".contains(&byte)),
		"Detected FileVersion must use ASCII letters, digits, '.', '-', '_' or '+' without whitespace"
	);
	filename(&version).context("Detected FileVersion is not a safe tool version")?;
	Ok(version)
}

fn open_source(source: &Path) -> Result<fs::File> {
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
	Ok(input)
}

fn package(source: &Path, target: &Path) -> Result<()> {
	let name = filename(
		source
			.file_name()
			.and_then(|name| name.to_str())
			.context("Binary needs a UTF-8 filename")?,
	)?;
	let input = open_source(source)?;
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
