//! Test-only executable. Never packaged or used by the production CLI.
#[path = "../common/mod.rs"]
mod common;

use anyhow::{Context, Result};
use azure_blob::config::{Request, noninteractive};
use serde_json::json;
use std::{env, fs, path::PathBuf};

#[tokio::main]
async fn main() -> Result<()> {
	let raw = env::var("AZURE_BLOB_REQUEST")?;
	let request: Request = serde_json::from_str(&raw)?;
	request.options.validate()?;
	if let Ok(mode) = env::var("MOCK_AZURE_AUTH_MODE") {
		anyhow::ensure!(
			env::var_os("AZURE_BLOB_LOGIN").is_none(),
			"Unexpected login fallback"
		);
		match mode.as_str() {
			"token" => {
				let credential = azure_blob::auth::credential(&request.options, false).await?;
				let token = credential
					.get_token(&[azure_blob::config::SCOPE], None)
					.await?;
				anyhow::ensure!(
					token.token.secret() == "integration-storage-token",
					"Token was not forwarded"
				);
				anyhow::ensure!(!raw.contains(token.token.secret()), "Token in request JSON");
				anyhow::ensure!(
					!env::args().any(|arg| arg.contains(token.token.secret())),
					"Token in arguments"
				);
			}
			"environment" => {
				anyhow::ensure!(
					env::var_os("AZURE_BLOB_STORAGE_TOKEN").is_none(),
					"Unexpected storage token"
				);
				for (name, value) in [
					("AZURE_TENANT_ID", "integration-tenant"),
					("AZURE_CLIENT_ID", "integration-client"),
					("AZURE_CLIENT_SECRET", "integration-client-secret"),
				] {
					anyhow::ensure!(
						env::var(name).as_deref() == Ok(value),
						"Environment credential was not forwarded"
					);
					anyhow::ensure!(
						!raw.contains(value),
						"Environment credential in request JSON"
					);
				}
			}
			"required" => {
				println!("{}", json!({"authentication_required": true}));
				return Ok(());
			}
			"stderr" => anyhow::bail!("Rejected integration-storage-token"),
			_ => anyhow::bail!("Unknown mock auth mode"),
		}
		let storage = common::FileStorage(PathBuf::from(env::var("MOCK_AZURE_STORAGE")?));
		println!("{}", azure_blob::dispatch(&storage, &request).await?);
		return Ok(());
	}
	let login = PathBuf::from(env::var("MOCK_AZURE_LOGIN")?);
	if env::var("AZURE_BLOB_LOGIN").as_deref() == Ok("1") {
		anyhow::ensure!(!noninteractive(), "Interactive authentication disabled");
		fs::write(login, "mock-account-no-tokens")?;
		return Ok(());
	}
	if !login.exists() {
		println!("{}", json!({"authentication_required": true}));
		return Ok(());
	}
	let storage = common::FileStorage(PathBuf::from(
		env::var("MOCK_AZURE_STORAGE").context("Missing mock storage")?,
	));
	println!("{}", azure_blob::dispatch(&storage, &request).await?);
	Ok(())
}
