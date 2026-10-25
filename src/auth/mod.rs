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
	// This client uses rustls-no-provider, so the process-wide provider must be
	// installed before the builder is created. Keep this here as well as in
	// main so library and test callers cannot forget it.
	crate::initialize_tls();
	reqwest::Client::builder()
		.no_gzip()
		.no_deflate()
		.no_brotli()
		.no_zstd()
		.redirect(reqwest::redirect::Policy::none())
		.connect_timeout(std::time::Duration::from_secs(20))
		// A read timeout detects a stalled body. A total timeout would cut off
		// large, slow-but-progressing artifact transfers.
		.read_timeout(std::time::Duration::from_secs(120))
		.build()
		.map_err(|_| anyhow!("Could not initialize Rustls HTTP client"))
}

fn safe_code(error_code: Option<&str>) -> String {
	error_code
		.filter(|code| {
			!code.is_empty()
				&& code.len() <= 64
				&& code
					.bytes()
					.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
		})
		.map(|code| format!("; code {code}"))
		.unwrap_or_default()
}

// Never format raw SDK errors: response bodies may contain credential details.
pub fn sdk_error(error: azure_core::Error) -> anyhow::Error {
	match error.kind() {
		azure_core::error::ErrorKind::HttpResponse {
			status, error_code, ..
		} => {
			let code = u16::from(*status);
			let hint = match code {
				400 => "the request was rejected; check storage_url and the blob name",
				401 => {
					"the token was rejected; sign in again and check tenant, client and storage audience"
				}
				403 => {
					"check Storage Blob Data Reader/Contributor, tenant and storage network rules"
				}
				404 => "the container or blob was not found; check storage_url and the blob layout",
				407 => "the proxy requires authentication",
				408 => "the request timed out; retry",
				409 => "a conflicting write or existing blob; use --overwrite for uploads",
				412 => "the blob changed or a precondition failed; retry",
				413 => "the artifact is too large for the service",
				429 => "the service throttled the request; retry later",
				500..=599 => "the Azure service failed temporarily; retry",
				_ => "check identity, permissions and network access",
			};
			anyhow!(
				"Azure request failed (HTTP {status}{}); {hint}",
				safe_code(error_code.as_deref())
			)
		}
		azure_core::error::ErrorKind::Connection => {
			anyhow!("Could not connect to Azure Storage; check DNS, TLS/proxy and network access")
		}
		azure_core::error::ErrorKind::Io => {
			anyhow!("Azure transfer failed while reading or writing data; check network and disk")
		}
		azure_core::error::ErrorKind::DataConversion => {
			anyhow!("Azure returned an unexpected response; retry or check the storage service")
		}
		azure_core::error::ErrorKind::Credential => anyhow!(
			"Azure rejected the credential; check tenant/client, secret/assertion, consent and network access"
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

#[cfg(test)]
mod tests {
	use super::*;
	use azure_core::{error::ErrorKind, http::StatusCode};

	fn http_error(status: StatusCode, error_code: Option<&str>) -> anyhow::Error {
		sdk_error(
			ErrorKind::HttpResponse {
				status,
				error_code: error_code.map(str::to_owned),
				raw_response: None,
			}
			.into_error(),
		)
	}

	#[test]
	fn http_errors_include_status_and_safe_code() {
		let message = http_error(StatusCode::NotFound, Some("BlobNotFound")).to_string();
		assert!(message.contains("HTTP 404"));
		assert!(message.contains("BlobNotFound"));
		assert!(message.contains("not found"));
	}

	#[test]
	fn unsafe_error_codes_are_not_echoed() {
		let message = http_error(StatusCode::BadRequest, Some("bad\ncode")).to_string();
		assert!(message.contains("HTTP 400"));
		assert!(!message.contains("bad"));
	}

	#[test]
	fn conflict_suggests_explicit_overwrite() {
		let message = http_error(StatusCode::Conflict, Some("BlobAlreadyExists")).to_string();
		assert!(message.contains("BlobAlreadyExists"));
		assert!(message.contains("--overwrite"));
	}

	#[test]
	fn credential_and_connection_errors_have_actionable_hints() {
		let credential = sdk_error(ErrorKind::Credential.into_error()).to_string();
		assert!(credential.contains("credential"));
		let connection = sdk_error(ErrorKind::Connection.into_error()).to_string();
		assert!(connection.contains("DNS"));
	}

	#[test]
	fn transient_errors_are_marked_retryable() {
		let throttled = http_error(StatusCode::TooManyRequests, None).to_string();
		assert!(throttled.contains("throttled"));
		let unavailable = http_error(StatusCode::ServiceUnavailable, None).to_string();
		assert!(unavailable.contains("temporarily"));
	}

	#[test]
	fn http_client_initializes_the_rustls_provider() {
		http_client().expect("HTTP client");
	}
}
