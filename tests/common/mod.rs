use anyhow::{Result, ensure};
use azure_ac::{config::blob_path, storage::Storage};
use serde_json::{Value, json};
use std::{
	fs,
	io::Write,
	path::{Path, PathBuf},
};

pub struct FileStorage(pub PathBuf);

#[async_trait::async_trait]
impl Storage for FileStorage {
	async fn read(&self, name: &str) -> Result<Vec<u8>> {
		Ok(fs::read(self.0.join(blob_path(name)?))?)
	}
	async fn download(&self, name: &str, target: &Path, max_bytes: Option<u64>) -> Result<()> {
		let bytes = self.read(name).await?;
		if let Some(max) = max_bytes {
			ensure!(
				bytes.len() as u64 <= max,
				"Download exceeds the {max}-byte limit; artifact rejected"
			);
		}
		let mut output = fs::OpenOptions::new()
			.write(true)
			.create_new(true)
			.open(target)?;
		output.write_all(&bytes)?;
		Ok(())
	}
	async fn upload(&self, name: &str, source: &Path, overwrite: bool) -> Result<()> {
		let target = self.0.join(blob_path(name)?);
		fs::create_dir_all(
			target
				.parent()
				.ok_or_else(|| anyhow::anyhow!("Invalid path"))?,
		)?;
		let mut output = fs::OpenOptions::new()
			.write(true)
			.create(true)
			.create_new(!overwrite)
			.truncate(overwrite)
			.open(target)?;
		output.write_all(&fs::read(source)?)?;
		Ok(())
	}
	async fn list(&self, prefix: &str, limit: usize) -> Result<Value> {
		ensure!(limit > 0, "limit must be greater than zero");
		fn walk(root: &Path, path: &Path, prefix: &str, result: &mut Vec<Value>) -> Result<()> {
			for item in fs::read_dir(path)? {
				let path = item?.path();
				if path.is_dir() {
					walk(root, &path, prefix, result)?;
				} else {
					let name = path
						.strip_prefix(root)?
						.to_string_lossy()
						.replace('\\', "/");
					if name.starts_with(prefix) {
						result.push(json!({"name": name, "size": path.metadata()?.len()}));
					}
				}
			}
			Ok(())
		}
		let mut result = Vec::new();
		walk(&self.0, &self.0, prefix, &mut result)?;
		ensure!(
			result.len() <= limit,
			"Listing exceeds {limit} blobs; use a narrower prefix"
		);
		result.sort_by_key(|v| v["name"].as_str().unwrap_or_default().to_owned());
		Ok(json!(result))
	}
}
