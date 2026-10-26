use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use url::Url;

pub const SCOPE: &str = "https://storage.azure.com/.default";
pub const DEFAULT_CLIENT: &str = "04b07795-8ddb-461a-bbee-02f9e1bf7b46";

#[derive(Clone, Deserialize, Serialize)]
pub struct Options {
	pub storage_url: String,
	#[serde(default = "default_tenant")]
	pub tenant_id: String,
	#[serde(default = "default_client")]
	pub client_id: String,
	#[serde(default = "default_auth")]
	pub auth: String,
	pub managed_identity_client_id: Option<String>,
	#[serde(default)]
	pub sha256: Option<String>,
}

fn default_tenant() -> String {
	"organizations".into()
}
fn default_client() -> String {
	DEFAULT_CLIENT.into()
}
fn default_auth() -> String {
	"auto".into()
}

impl Options {
	pub fn validate(&self) -> Result<Url> {
		// Validate the raw path as well: Url normalizes traversal components.
		let raw = self.storage_url.trim_end_matches("/");
		let url = Url::parse(raw).map_err(|_| anyhow::anyhow!("Invalid storage_url"))?;
		let host = url.host_str().unwrap_or_default();
		let account = host
			.strip_suffix(".blob.core.windows.net")
			.unwrap_or_default();
		let container = url.path().strip_prefix("/").unwrap_or_default();
		ensure!(
			url.scheme() == "https"
				&& url.username().is_empty()
				&& url.password().is_none()
				&& url.port().is_none()
				&& url.query().is_none()
				&& url.fragment().is_none()
				&& (3..=24).contains(&account.len())
				&& account
					.bytes()
					.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
				&& (3..=63).contains(&container.len())
				&& !container.contains("--")
				&& container
					.bytes()
					.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
				&& !container.starts_with('-')
				&& !container.ends_with('-')
				&& raw == format!("https://{host}/{container}"),
			"storage_url must be https://ACCOUNT.blob.core.windows.net/CONTAINER (no SAS)"
		);
		for value in [&self.tenant_id, &self.client_id] {
			ensure!(
				!value.is_empty()
					&& value
						.bytes()
						.all(|b| b.is_ascii_alphanumeric() || b"-._".contains(&b)),
				"Invalid tenant_id or client_id"
			);
		}
		if let Some(value) = &self.managed_identity_client_id {
			ensure!(
				!value.is_empty()
					&& value
						.bytes()
						.all(|b| b.is_ascii_alphanumeric() || b"-._".contains(&b)),
				"Invalid managed_identity_client_id"
			);
		}
		if let Some(value) = &self.sha256 {
			ensure!(
				value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit()),
				"sha256 must be 64 hexadecimal digits"
			);
		}
		ensure!(
			["auto", "broker", "environment", "workload", "managed"].contains(&self.auth.as_str()),
			"auth must be auto, broker, environment, workload or managed; browser/device login is not supported"
		);
		Ok(url)
	}
}

#[derive(Deserialize, Serialize)]
pub struct Request {
	pub action: String,
	pub options: Options,
	pub tool: Option<String>,
	pub version: Option<String>,
	pub platform: Option<String>,
	pub install_path: Option<PathBuf>,
	pub download_path: Option<PathBuf>,
	pub blob: Option<String>,
	pub file: Option<PathBuf>,
	#[serde(default)]
	pub prefix: String,
	#[serde(default)]
	pub limit: Option<usize>,
	#[serde(default)]
	pub overwrite: bool,
}

pub fn relative(value: &str) -> Result<&str> {
	ensure!(
		!value.is_empty()
			&& !value.contains(['\\', ':', '%'])
			&& !value.chars().any(char::is_control)
			&& value
				.split('/')
				.all(|p| !p.is_empty() && p != "." && p != ".."),
		"Expected a safe relative path, got {value:?}"
	);
	Ok(value)
}

/// Validates a blob name. Blob names may contain many characters that are not
/// valid in a local Windows filename; the SDK percent-encodes them for the URL.
pub fn blob_path(value: &str) -> Result<&str> {
	ensure!(
		!value.is_empty()
			&& !value.contains('\\')
			&& !value.chars().any(char::is_control)
			&& value
				.split('/')
				.all(|p| !p.is_empty() && p != "." && p != ".."),
		"Expected a safe relative blob path, got {value:?}"
	);
	Ok(value)
}

/// Validates a relative path that will also be used as a local path, including
/// Windows portability rules for every component.
pub fn portable_path(value: &str) -> Result<&str> {
	relative(value)?;
	for part in value.split('/') {
		validate_component(part)?;
	}
	Ok(value)
}

/// Validates a single filename that will be used as a local path.
pub fn filename(value: &str) -> Result<&str> {
	portable_path(value)?;
	ensure!(
		!value.contains('/'),
		"Expected a single filename, got {value:?}"
	);
	Ok(value)
}

pub fn destination(root: &Path, name: &str) -> Result<PathBuf> {
	let name = relative(name.trim_end_matches('/'))?;
	for part in name.split('/') {
		validate_component(part)?;
	}
	Ok(root.join(name))
}

fn validate_component(part: &str) -> Result<()> {
	ensure!(
		!part.is_empty() && part != "." && part != "..",
		"Invalid path component {part:?}"
	);
	ensure!(
		!part.chars().any(char::is_control),
		"Path component contains control characters: {part:?}"
	);
	ensure!(
		!part.contains(['<', '>', '"', '|', '?', '*', ':', '\\', '/']),
		"Nonportable Windows filename component {part:?}"
	);
	ensure!(
		!part.ends_with(['.', ' ']),
		"Nonportable Windows filename component {part:?}"
	);
	let base = part
		.split('.')
		.next()
		.unwrap_or_default()
		.to_ascii_uppercase();
	ensure!(
		!is_reserved_name(&base),
		"Reserved Windows filename component {part:?}"
	);
	Ok(())
}

fn is_reserved_name(base: &str) -> bool {
	matches!(
		base,
		"CON"
			| "PRN" | "AUX"
			| "NUL" | "CLOCK$"
			| "CONIN$"
			| "CONOUT$"
			| "COM¹" | "COM²"
			| "COM³" | "LPT¹"
			| "LPT²" | "LPT³"
	) || (base.len() == 4
		&& (base.starts_with("COM") || base.starts_with("LPT"))
		&& matches!(base.as_bytes()[3], b'1'..=b'9'))
}

pub fn noninteractive() -> bool {
	std::env::var_os("CI").is_some()
		|| std::env::var("AZURE_BLOB_NONINTERACTIVE").as_deref() == Ok("1")
}
