use crate::config::destination;
use anyhow::{Context, Result, bail, ensure};
use std::{
	collections::HashSet,
	fs,
	io::{self, Read},
	path::Path,
};

const MAX_EXPANDED: u64 = 4 * 1024 * 1024 * 1024;
const MAX_ENTRIES: usize = 100_000;

pub fn extract(archive: &Path, target: &Path) -> Result<()> {
	let filename = archive
		.file_name()
		.and_then(|n| n.to_str())
		.context("Invalid archive filename")?;
	let file = fs::File::open(archive)?;
	if filename.ends_with(".zip") {
		let mut zip = zip::ZipArchive::new(file)?;
		ensure!(zip.len() <= MAX_ENTRIES, "Archive has too many entries");
		let mut seen = HashSet::new();
		let mut remaining = MAX_EXPANDED;
		for i in 0..zip.len() {
			let mut entry = zip.by_index(i)?;
			let name = entry.name().to_owned();
			let mode = entry.unix_mode().unwrap_or(0o100644);
			ensure!(
				matches!(mode & 0o170000, 0 | 0o100000 | 0o040000),
				"Archive contains a link or special file"
			);
			let path = destination(target, &name)?;
			ensure!(
				seen.insert(name.trim_end_matches('/').to_ascii_lowercase()),
				"Archive contains duplicate paths"
			);
			if entry.is_dir() {
				fs::create_dir_all(path)?;
			} else {
				write_file(&mut entry, &path, mode, &mut remaining)?;
			}
		}
	} else {
		let reader: Box<dyn Read> = if filename.ends_with(".tar.gz") {
			Box::new(flate2::read::GzDecoder::new(file))
		} else if filename.ends_with(".tar.xz") {
			Box::new(lzma_rust2::XzReader::new(file, false))
		} else if filename.ends_with(".tar.bz2") {
			Box::new(bzip2::read::BzDecoder::new(file))
		} else {
			bail!("Supported archives: .zip, .tar.gz, .tar.xz, .tar.bz2");
		};
		let mut tar = tar::Archive::new(reader);
		let mut seen = HashSet::new();
		let mut remaining = MAX_EXPANDED;
		for (count, item) in tar.entries()?.enumerate() {
			ensure!(count < MAX_ENTRIES, "Archive has too many entries");
			let mut entry = item?;
			let name = std::str::from_utf8(&entry.path_bytes())?.to_owned();
			let path = destination(target, &name)?;
			ensure!(
				seen.insert(name.trim_end_matches('/').to_ascii_lowercase()),
				"Archive contains duplicate paths"
			);
			let kind = entry.header().entry_type();
			if kind.is_dir() {
				fs::create_dir_all(path)?;
			} else {
				ensure!(kind.is_file(), "Archive contains a link or special file");
				let mode = entry.header().mode()?;
				write_file(&mut entry, &path, mode, &mut remaining)?;
			}
		}
	}
	Ok(())
}

fn write_file(reader: &mut dyn Read, path: &Path, mode: u32, remaining: &mut u64) -> Result<()> {
	fs::create_dir_all(path.parent().context("Invalid archive path")?)?;
	let mut output = fs::OpenOptions::new()
		.write(true)
		.create_new(true)
		.open(path)?;
	let written = io::copy(&mut reader.take(*remaining + 1), &mut output)?;
	ensure!(
		written <= *remaining,
		"Expanded archive exceeds 4 GiB limit"
	);
	*remaining -= written;
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt;
		output.set_permissions(fs::Permissions::from_mode(if mode & 0o111 != 0 {
			0o755
		} else {
			0o644
		}))?;
	}
	#[cfg(not(unix))]
	let _ = mode;
	Ok(())
}
