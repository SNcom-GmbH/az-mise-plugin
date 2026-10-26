#[cfg(windows)]
mod wam;

use crate::config::{DEFAULT_CLIENT, Options, SCOPE};
use anyhow::{Context, Result, anyhow, bail, ensure};
use azure_core::{
	credentials::{AccessToken, Secret, TokenCredential, TokenRequestOptions},
	time::{Duration, OffsetDateTime},
};
use azure_identity::{
	ClientSecretCredential, ClientSecretCredentialOptions, ManagedIdentityCredential,
	WorkloadIdentityCredential,
};
use std::{env, sync::Arc};

pub const STORAGE_TOKEN_ENV: &str = "AZURE_BLOB_STORAGE_TOKEN";

/// A caller-owned bearer token. No discovery, refresh, or identity fallback.
#[derive(Debug)]
pub(crate) struct StorageTokenCredential(Secret);

impl StorageTokenCredential {
	pub(crate) fn new(token: String) -> Result<Self> {
		// RFC 6750 b64token syntax. Treat access tokens as opaque: Azure validates
		// audience, expiry and signature. Do not trim or accept a Bearer prefix.
		let value = token.trim_end_matches('=');
		ensure!(
			!value.is_empty()
				&& value
					.bytes()
					.all(|b| b.is_ascii_alphanumeric() || b"-._~+/".contains(&b)),
			"AZURE_BLOB_STORAGE_TOKEN must contain a nonempty bearer token without whitespace or a Bearer prefix"
		);
		Ok(Self(Secret::new(token)))
	}
}

#[async_trait::async_trait]
impl TokenCredential for StorageTokenCredential {
	async fn get_token(
		&self,
		scopes: &[&str],
		_: Option<TokenRequestOptions<'_>>,
	) -> azure_core::Result<AccessToken> {
		if scopes != [SCOPE] {
			return Err(azure_core::Error::with_message(
				azure_core::error::ErrorKind::Credential,
				"Supplied storage token only supports the Azure Storage scope",
			));
		}
		// This timestamp only controls the SDK's in-memory cache. It does not
		// extend the real token lifetime. Every call returns the same token;
		// Azure rejects expiry and the caller must acquire a replacement.
		Ok(AccessToken::new(
			self.0.clone(),
			OffsetDateTime::now_utc() + Duration::hours(1),
		))
	}
}

/// Redact even errors whose context contains caller-controlled paths or values.
pub fn redact(message: &str) -> String {
	let mut message = message.to_owned();
	for name in [STORAGE_TOKEN_ENV, "AZURE_CLIENT_SECRET"] {
		if let Ok(secret) = env::var(name)
			&& !secret.trim().is_empty()
		{
			message = message.replace(&secret, "[redacted]");
		}
	}
	message
}

fn option_or_env(option: &str, default: &str, env_name: &str) -> Result<String> {
	if option != default {
		return Ok(option.to_owned());
	}
	env::var(env_name).with_context(|| format!("{env_name} is required"))
}

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
				401 if env::var_os(STORAGE_TOKEN_ENV).is_some() => {
					"the supplied storage token was rejected; the caller must acquire a new token for the storage audience"
				}
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
				redact(&safe_code(error_code.as_deref()))
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
	credential_with_environment_options(options, interactive, None).await
}

async fn credential_with_environment_options(
	options: &Options,
	interactive: bool,
	environment_options: Option<ClientSecretCredentialOptions>,
) -> Result<Arc<dyn TokenCredential>> {
	// Presence, including an empty/non-Unicode value, is authoritative. Validate
	// before considering auth modes, CI guards, broker or any Azure identity.
	match env::var(STORAGE_TOKEN_ENV) {
		Ok(token) => return Ok(Arc::new(StorageTokenCredential::new(token)?)),
		Err(env::VarError::NotUnicode(_)) => {
			bail!("AZURE_BLOB_STORAGE_TOKEN must be valid Unicode")
		}
		Err(env::VarError::NotPresent) => {}
	}
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
		let tenant = option_or_env(&options.tenant_id, "organizations", "AZURE_TENANT_ID")?;
		let client = option_or_env(&options.client_id, DEFAULT_CLIENT, "AZURE_CLIENT_ID")?;
		ClientSecretCredential::new(
			&tenant,
			client,
			env::var("AZURE_CLIENT_SECRET")
				.map_err(|_| anyhow!("AZURE_CLIENT_SECRET is required"))?
				.into(),
			environment_options,
		)
		.map_err(sdk_error)?
	} else if mode == "workload"
		|| (mode == "auto" && env::var_os("AZURE_FEDERATED_TOKEN_FILE").is_some())
	{
		let config = azure_identity::WorkloadIdentityCredentialOptions {
			tenant_id: Some(option_or_env(
				&options.tenant_id,
				"organizations",
				"AZURE_TENANT_ID",
			)?),
			client_id: Some(option_or_env(
				&options.client_id,
				DEFAULT_CLIENT,
				"AZURE_CLIENT_ID",
			)?),
			..Default::default()
		};
		WorkloadIdentityCredential::new(Some(config)).map_err(sdk_error)?
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
	use azure_core::{
		Bytes,
		error::ErrorKind,
		http::{
			AsyncRawResponse, Body, ClientOptions, HttpClient, Request, StatusCode, Transport,
			headers::Headers,
		},
	};
	use std::{
		process::Command,
		sync::atomic::{AtomicUsize, Ordering},
	};

	#[derive(Debug)]
	struct IdentityHttp {
		status: StatusCode,
		calls: AtomicUsize,
	}

	#[async_trait::async_trait]
	impl HttpClient for IdentityHttp {
		async fn execute_request(&self, request: &Request) -> azure_core::Result<AsyncRawResponse> {
			self.calls.fetch_add(1, Ordering::SeqCst);
			assert_eq!(
				request.url().as_str(),
				"https://login.microsoftonline.com/integration-tenant/oauth2/v2.0/token"
			);
			let Body::Bytes(body) = request.body() else {
				panic!("Expected identity form body")
			};
			let form: std::collections::HashMap<_, _> =
				url::form_urlencoded::parse(body).into_owned().collect();
			assert_eq!(form["client_id"], "integration-client");
			assert_eq!(form["client_secret"], "integration-client-secret");
			assert_eq!(form["scope"], SCOPE);
			let body = if self.status == StatusCode::Ok {
				br#"{"access_token":"environment-storage-token","expires_in":3600,"token_type":"Bearer"}"#.as_slice()
			} else {
				br#"{"error":"invalid_client","error_description":"integration-client-secret"}"#
					.as_slice()
			};
			Ok(AsyncRawResponse::from_bytes(
				self.status,
				Headers::new(),
				Bytes::from_static(body),
			))
		}
	}

	#[test]
	fn environment_authentication_succeeds_and_fails_without_fallback() -> Result<()> {
		for mode in ["auto", "environment"] {
			let mut child = Command::new(env::current_exe()?);
			for (key, _) in env::vars_os() {
				if key.to_string_lossy().starts_with("AZURE_") {
					child.env_remove(key);
				}
			}
			let output = child
				.args(["--exact", "auth::tests::environment_authentication_child"])
				.env("AZURE_BLOB_TEST_CHILD", mode)
				.env("CI", "1")
				.env("AZURE_TENANT_ID", "integration-tenant")
				.env("AZURE_CLIENT_ID", "integration-client")
				.env("AZURE_CLIENT_SECRET", "integration-client-secret")
				.output()?;
			assert!(
				output.status.success(),
				"Environment auth regression: {}",
				String::from_utf8_lossy(&output.stdout)
			);
		}
		Ok(())
	}

	#[tokio::test]
	async fn environment_authentication_child() -> Result<()> {
		// Use a child process so tests never mutate shared process credentials.
		let Ok(mode) = env::var("AZURE_BLOB_TEST_CHILD") else {
			return Ok(());
		};
		let options: Options = serde_json::from_value(serde_json::json!({
			"storage_url": "https://account.blob.core.windows.net/tools", "auth": mode
		}))?;
		for status in [StatusCode::Ok, StatusCode::BadRequest] {
			let http = Arc::new(IdentityHttp {
				status,
				calls: AtomicUsize::new(0),
			});
			let result = credential_with_environment_options(
				&options,
				false,
				Some(ClientSecretCredentialOptions {
					client_options: ClientOptions {
						transport: Some(Transport::new(http.clone())),
						..Default::default()
					},
				}),
			)
			.await;
			if status == StatusCode::Ok {
				let credential = result?;
				let token = credential.get_token(&[SCOPE], None).await?;
				assert_eq!(token.token.secret(), "environment-storage-token");
				assert!(!format!("{credential:?}").contains("integration-client-secret"));
			} else {
				let error = result.expect_err("Rejected environment credential");
				assert!(!error.is::<AuthenticationRequired>());
				assert!(error.to_string().contains("credential"));
				assert!(!format!("{error:#}").contains("integration-client-secret"));
			}
			assert_eq!(http.calls.load(Ordering::SeqCst), 1);
		}
		Ok(())
	}

	#[test]
	fn supplied_tokens_are_opaque_but_must_have_bearer_syntax() {
		for token in ["opaque-token", "header.payload.signature", "a+/~_-.=="] {
			let credential = StorageTokenCredential::new(token.into()).expect("bearer syntax");
			assert!(!format!("{credential:?}").contains(token));
		}
		for token in [
			"",
			" ",
			"\t",
			"\r\n",
			"Bearer token",
			" token",
			"token\n",
			"a=b",
			"=",
			"a,b",
			"tökén",
		] {
			let error = StorageTokenCredential::new(token.into()).expect_err("invalid syntax");
			assert!(
				error
					.to_string()
					.starts_with("AZURE_BLOB_STORAGE_TOKEN must contain")
			);
		}
	}

	#[tokio::test]
	async fn supplied_token_never_refreshes_or_requests_another_scope() {
		let credential = StorageTokenCredential::new("caller-token".into()).expect("token");
		for _ in 0..2 {
			let token = credential.get_token(&[SCOPE], None).await.expect("token");
			assert_eq!(token.token.secret(), "caller-token");
			assert!(!format!("{token:?}").contains("caller-token"));
		}
		assert!(
			credential
				.get_token(&["https://management.azure.com/.default"], None)
				.await
				.is_err()
		);
	}

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
	fn explicit_identity_options_are_used() {
		assert_eq!(
			option_or_env("tenant", "organizations", "AZURE_TENANT_ID").expect("tenant"),
			"tenant"
		);
		assert_eq!(
			option_or_env("client", DEFAULT_CLIENT, "AZURE_CLIENT_ID").expect("client"),
			"client"
		);
	}

	#[test]
	fn http_client_initializes_the_rustls_provider() {
		http_client().expect("HTTP client");
	}
}
