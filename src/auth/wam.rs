//! Native Windows WAM; no MSAL DLL, browser launcher, or plaintext token cache.
//! Runtime validation against a corporate tenant remains a workplace test.
use super::AuthenticationRequired;
use crate::config::{Options, SCOPE};
use anyhow::{Context, Result, anyhow, bail, ensure};
use azure_core::{
	credentials::{AccessToken, TokenCredential},
	time::{Duration, OffsetDateTime},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};
use std::{
	fs,
	io::Write,
	path::{Path, PathBuf},
	sync::Arc,
};
use tokio::sync::Mutex;
use windows::{
	Security::Authentication::Web::Core::{
		WebAuthenticationCoreManager, WebTokenRequest, WebTokenRequestResult, WebTokenRequestStatus,
	},
	Win32::System::{
		Console::GetConsoleWindow,
		WinRT::{
			IWebAuthenticationCoreManagerInterop, RO_INIT_MULTITHREADED, RoInitialize,
			RoUninitialize,
		},
	},
	Win32::UI::WindowsAndMessaging::GetForegroundWindow,
	core::HSTRING,
};

pub async fn credential(options: &Options, interactive: bool) -> Result<Arc<dyn TokenCredential>> {
	let token = match acquire(options.clone(), false).await {
		Ok(token) => token,
		Err(error) if interactive && error.is::<AuthenticationRequired>() => {
			acquire(options.clone(), true).await?
		}
		Err(error) => return Err(error),
	};
	Ok(Arc::new(WamCredential {
		options: options.clone(),
		token: Mutex::new(Some(token)),
	}))
}

struct WamCredential {
	options: Options,
	token: Mutex<Option<AccessToken>>,
}
impl std::fmt::Debug for WamCredential {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str("WamCredential { .. }")
	}
}

#[async_trait::async_trait]
impl TokenCredential for WamCredential {
	async fn get_token(
		&self,
		scopes: &[&str],
		_: Option<azure_core::credentials::TokenRequestOptions<'_>>,
	) -> azure_core::Result<AccessToken> {
		if scopes != [SCOPE] {
			return Err(azure_core::Error::with_message(
				azure_core::error::ErrorKind::Credential,
				"Unsupported token scope",
			));
		}
		let mut cached = self.token.lock().await;
		if let Some(token) = cached
			.as_ref()
			.filter(|token| token.expires_on > OffsetDateTime::now_utc() + Duration::minutes(5))
		{
			return Ok(token.clone());
		}
		let token = acquire(self.options.clone(), false).await.map_err(|_| {
			azure_core::Error::with_message(
				azure_core::error::ErrorKind::Credential,
				"WAM could not silently renew the storage token; rerun the command interactively",
			)
		})?;
		*cached = Some(token.clone());
		Ok(token)
	}
}

async fn acquire(options: Options, interactive: bool) -> Result<AccessToken> {
	tokio::task::spawn_blocking(move || acquire_on_com_thread(&options, interactive))
		.await
		.context("WAM thread failed")?
}

fn acquire_on_com_thread(options: &Options, interactive: bool) -> Result<AccessToken> {
	// A dedicated blocking thread owns every COM object and balances initialization.
	unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.map_err(windows_error)?;
	struct Apartment;
	impl Drop for Apartment {
		fn drop(&mut self) {
			unsafe { RoUninitialize() };
		}
	}
	let _apartment = Apartment;
	let provider = WebAuthenticationCoreManager::FindAccountProviderWithAuthorityAsync(
		&HSTRING::from("https://login.microsoft.com"),
		&HSTRING::from("organizations"),
	)
	.map_err(windows_error)?
	.join()
	.map_err(windows_error)?;
	// The AAD WAM provider uses resource/authority properties for this request.
	let request = WebTokenRequest::Create(
		&provider,
		&HSTRING::new(),
		&HSTRING::from(&options.client_id),
	)
	.map_err(windows_error)?;
	let properties = request.Properties().map_err(windows_error)?;
	properties
		.Insert(
			&HSTRING::from("resource"),
			&HSTRING::from("https://storage.azure.com/"),
		)
		.map_err(windows_error)?;
	properties
		.Insert(
			&HSTRING::from("authority"),
			&HSTRING::from(format!(
				"https://login.microsoftonline.com/{}",
				options.tenant_id
			)),
		)
		.map_err(windows_error)?;
	let account_path = account_path(options)?;
	let saved_id = if interactive {
		None
	} else {
		saved_account_id(&account_path)?
	};
	let result = if interactive {
		let mut window = unsafe { GetConsoleWindow() };
		if window.is_invalid() {
			window = unsafe { GetForegroundWindow() };
		}
		ensure!(
			!window.is_invalid(),
			"WAM login requires an interactive Windows desktop"
		);
		let interop: IWebAuthenticationCoreManagerInterop =
			windows::core::factory::<WebAuthenticationCoreManager, _>().map_err(windows_error)?;
		// SAFETY: HWND is a live OS window, request and apartment live until completion.
		let operation: windows_future::IAsyncOperation<WebTokenRequestResult> =
			unsafe { interop.RequestTokenForWindowAsync(window, &request) }
				.map_err(windows_error)?;
		operation.join().map_err(windows_error)?
	} else if let Some(id) = saved_id.as_deref() {
		// Never silently switch to another Windows account when a selection disappears.
		let account = WebAuthenticationCoreManager::FindAccountAsync(&provider, &HSTRING::from(id))
			.map_err(windows_error)?
			.join()
			.map_err(|_| AuthenticationRequired)?;
		WebAuthenticationCoreManager::GetTokenSilentlyWithWebAccountAsync(&request, &account)
			.map_err(windows_error)?
			.join()
			.map_err(windows_error)?
	} else {
		WebAuthenticationCoreManager::GetTokenSilentlyAsync(&request)
			.map_err(windows_error)?
			.join()
			.map_err(windows_error)?
	};
	let status = result.ResponseStatus().map_err(windows_error)?;
	if status == WebTokenRequestStatus::UserInteractionRequired
		|| status == WebTokenRequestStatus::AccountSwitch
	{
		return Err(AuthenticationRequired.into());
	}
	if status == WebTokenRequestStatus::UserCancel {
		bail!("WAM sign-in cancelled");
	}
	ensure!(
		status == WebTokenRequestStatus::Success,
		"WAM failed (status {}); check tenant, public-client app registration and corporate policy",
		status.0
	);
	let response = result
		.ResponseData()
		.map_err(windows_error)?
		.GetAt(0)
		.map_err(windows_error)?;
	let token = response.Token().map_err(windows_error)?.to_string();
	ensure!(!token.is_empty(), "WAM returned no access token");
	let id = response
		.WebAccount()
		.map_err(windows_error)?
		.Id()
		.map_err(windows_error)?
		.to_string();
	if saved_id.as_deref() != Some(id.as_str()) {
		// Account selection is only a convenience for silent sign-in. A valid
		// token must never fail because the profile or metadata file is locked.
		let _ = save_account_id(&account_path, &id);
	}
	// JWT decoding here ONLY schedules renewal; it does not verify identity or grant access.
	let expiry = token
		.split('.')
		.nth(1)
		.and_then(|payload| URL_SAFE_NO_PAD.decode(payload).ok())
		.and_then(|payload| serde_json::from_slice::<serde_json::Value>(&payload).ok())
		.and_then(|claims| claims["exp"].as_i64())
		.and_then(|exp| OffsetDateTime::from_unix_timestamp(exp).ok())
		// If the JWT payload cannot be decoded, refresh conservatively instead
		// of retrying the broker on every storage request.
		.unwrap_or_else(|| OffsetDateTime::now_utc() + Duration::minutes(10));
	Ok(AccessToken::new(token, expiry))
}

fn account_path(options: &Options) -> Result<PathBuf> {
	let base = std::env::var_os("LOCALAPPDATA")
		.context("LOCALAPPDATA is required for Windows account metadata")?;
	let identity = format!("{}:{}", options.tenant_id, options.client_id);
	Ok(PathBuf::from(base).join("mise-azure-blob").join(format!(
		"{}.json",
		hex::encode(Sha256::digest(identity.as_bytes()))
	)))
}

fn saved_account_id(path: &Path) -> Result<Option<String>> {
	match fs::read(path) {
		Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes).context(
			"Invalid saved WAM account selection; remove the account metadata file and sign in again",
		)?)),
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
		Err(error) => Err(error)
			.with_context(|| format!("Could not read WAM account metadata {}", path.display())),
	}
}

fn save_account_id(path: &Path, id: &str) -> Result<()> {
	let parent = path.parent().context("Invalid WAM metadata path")?;
	fs::create_dir_all(parent)?;
	// This is only the selected OS account ID, never an access or refresh token.
	let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
	temporary.write_all(serde_json::to_string(id)?.as_bytes())?;
	temporary
		.persist(path)
		.map_err(|_| anyhow!("Could not save WAM account selection"))?;
	Ok(())
}

fn windows_error(error: windows::core::Error) -> anyhow::Error {
	// Avoid OS error messages that may contain usernames or tenant response bodies.
	anyhow!(
		"Windows WAM API failed (HRESULT 0x{:08X}); check broker availability and app registration",
		error.code().0 as u32
	)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn saved_account_id_round_trips() {
		let root = tempfile::tempdir().expect("temp dir");
		let path = root.path().join("metadata/account.json");
		assert_eq!(saved_account_id(&path).expect("missing file"), None);
		save_account_id(&path, "account-id").expect("save account id");
		assert_eq!(
			saved_account_id(&path).expect("read account id"),
			Some("account-id".into())
		);
	}

	#[test]
	fn corrupt_account_metadata_is_rejected() {
		let root = tempfile::tempdir().expect("temp dir");
		let path = root.path().join("account.json");
		fs::write(&path, b"not json").expect("fixture");
		let error = saved_account_id(&path).expect_err("corrupt metadata");
		assert!(
			error
				.to_string()
				.contains("Invalid saved WAM account selection")
		);
	}
}
