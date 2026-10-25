#[cfg(windows)]
mod wam;

use crate::config::{Options, SCOPE};
use anyhow::{Result, anyhow, bail};
use azure_core::credentials::TokenCredential;
use azure_identity::{
	ClientSecretCredential, ManagedIdentityCredential, WorkloadIdentityCredential,
};
use std::{env, sync::Arc};

#[derive(Debug)]
pub struct AuthenticationRequired;
impl std::fmt::Display for AuthenticationRequired {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str("Azure sign-in required")
	}
}
impl std::error::Error for AuthenticationRequired {}

pub fn http_client() -> Result<reqwest::Client> {
	reqwest::Client::builder()
		.no_gzip()
		.no_deflate()
		.no_brotli()
		.no_zstd()
		.redirect(reqwest::redirect::Policy::none())
		.connect_timeout(std::time::Duration::from_secs(20))
		.timeout(std::time::Duration::from_secs(120))
		.build()
		.map_err(|_| anyhow!("Could not initialize Rustls HTTP client"))
}

// Never format raw SDK errors: response bodies may contain credential details.
pub fn sdk_error(error: azure_core::Error) -> anyhow::Error {
	match error.kind() {
		azure_core::error::ErrorKind::HttpResponse { status, .. } => anyhow!(
			"Azure request failed (HTTP {status}); check identity, Blob Data Reader/Contributor and network access"
		),
		_ => anyhow!(
			"Azure SDK request failed; check tenant/client configuration, identity and network access"
		),
	}
}

pub async fn credential(options: &Options, interactive: bool) -> Result<Arc<dyn TokenCredential>> {
	let mode = options.auth.as_str();
	if env::var_os("CI").is_some()
		&& mode == "auto"
		&& env::var_os("AZURE_CLIENT_SECRET").is_none()
		&& env::var_os("AZURE_FEDERATED_TOKEN_FILE").is_none()
	{
		bail!(
			"CI-only identity required: configure Workload Identity, a service principal or explicit auth=managed"
		);
	}
	let credential: Arc<dyn TokenCredential> = if mode == "environment"
		|| (mode == "auto" && env::var_os("AZURE_CLIENT_SECRET").is_some())
	{
		ClientSecretCredential::new(
			&env::var("AZURE_TENANT_ID").map_err(|_| anyhow!("AZURE_TENANT_ID is required"))?,
			env::var("AZURE_CLIENT_ID").map_err(|_| anyhow!("AZURE_CLIENT_ID is required"))?,
			env::var("AZURE_CLIENT_SECRET")
				.map_err(|_| anyhow!("AZURE_CLIENT_SECRET is required"))?
				.into(),
			None,
		)
		.map_err(sdk_error)?
	} else if mode == "workload"
		|| (mode == "auto" && env::var_os("AZURE_FEDERATED_TOKEN_FILE").is_some())
	{
		WorkloadIdentityCredential::new(None).map_err(sdk_error)?
	} else if mode == "managed" {
		let mut config = azure_identity::ManagedIdentityCredentialOptions::default();
		if let Some(client) = &options.managed_identity_client_id {
			config.user_assigned_id =
				Some(azure_identity::UserAssignedId::ClientId(client.clone()));
		}
		ManagedIdentityCredential::new(Some(config)).map_err(sdk_error)?
	} else {
		if mode == "broker" || (mode == "auto" && cfg!(windows)) {
			#[cfg(windows)]
			return wam::credential(options, interactive && !crate::config::noninteractive()).await;
			#[cfg(not(windows))]
			bail!("auth=broker requires native Windows; no automatic device-code fallback");
		}
		let _ = interactive;
		bail!(
			"Linux is CI-only: configure Workload Identity, a service principal or auth=managed; no browser/device login"
		);
	};
	credential
		.get_token(&[SCOPE], None)
		.await
		.map_err(sdk_error)?;
	Ok(credential)
}
