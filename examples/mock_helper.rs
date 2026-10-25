//! Test-only executable. Never packaged or used by the production CLI.
#[path = "../tests/common/mod.rs"]
mod common;

use anyhow::{Context, Result};
use azure_ac::config::{Request, noninteractive};
use serde_json::json;
use std::{env, fs, path::PathBuf};

#[tokio::main]
async fn main() -> Result<()> {
	let request: Request = serde_json::from_str(&env::var("AZURE_AC_REQUEST")?)?;
	request.options.validate()?;
	let login = PathBuf::from(env::var("MOCK_AZURE_LOGIN")?);
	if env::var("AZURE_AC_LOGIN").as_deref() == Ok("1") {
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
	println!("{}", azure_ac::dispatch(&storage, &request).await?);
	Ok(())
}
