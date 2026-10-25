use anyhow::{Result, bail, ensure};
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
		"Expected a safe relative blob/archive path"
	);
	Ok(value)
}

pub fn destination(root: &Path, name: &str) -> Result<PathBuf> {
	let name = relative(name.trim_end_matches('/'))?;
	for part in name.split('/') {
		let base = part
			.split('.')
			.next()
			.unwrap_or_default()
			.to_ascii_uppercase();
		if part.ends_with(['.', ' '])
			|| part.contains(['<', '>', '"', '|', '?', '*'])
			|| ["CON", "PRN", "AUX", "NUL"].contains(&base.as_str())
			|| (base.len() == 4
				&& (base.starts_with("COM") || base.starts_with("LPT"))
				&& matches!(base.as_bytes()[3], b'1'..=b'9'))
		{
			bail!("Archive contains a nonportable Windows filename");
		}
	}
	Ok(root.join(name))
}

pub fn noninteractive() -> bool {
	std::env::var_os("CI").is_some()
		|| std::env::var("AZURE_AC_NONINTERACTIVE").as_deref() == Ok("1")
}
