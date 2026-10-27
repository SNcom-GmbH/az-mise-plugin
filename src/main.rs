mod command;

use anyhow::{Context, Result};
use azure_blob::{
	auth::{self, AuthenticationRequired},
	config::{Options, Request, noninteractive},
	storage::AzureStorage,
};
use clap::Parser;
use command::{Cli, Command};
use serde_json::json;

#[tokio::main]
async fn main() {
	azure_blob::initialize_tls();
	if let Err(error) = run().await {
		// SDK errors omit response bodies; also redact caller-controlled context.
		eprintln!("azure-blob: {}", auth::redact(&format!("{error:#}")));
		std::process::exit(1);
	}
}

async fn run() -> Result<()> {
	let cli = Cli::parse();
	let hook = matches!(cli.command, Command::Hook);
	let mut request: Request = if hook {
		serde_json::from_str(&std::env::var("AZURE_BLOB_REQUEST").context("Missing hook request")?)
			.context("Invalid hook request")?
	} else {
		let mut value = json!({
			"options": Options {
				storage_url: cli.storage_url.context("--storage-url is required")?,
				tenant_id: cli.tenant_id, client_id: cli.client_id, auth: cli.auth,
				managed_identity_client_id: cli.managed_identity_client_id,
				sha256: None,
			}
		});
		let args = match cli.command {
			Command::Login => json!({"action": "login"}),
			Command::List { prefix, limit } => {
				json!({"action": "list", "prefix": prefix, "limit": limit})
			}
			Command::Versions { tool } => json!({"action": "versions", "tool": tool}),
			Command::Read { blob } => json!({"action": "read", "blob": blob}),
			Command::Download { blob, file } => {
				json!({"action": "download", "blob": blob, "file": file})
			}
			Command::Upload {
				blob,
				file,
				overwrite,
			} => json!({"action": "upload", "blob": blob, "file": file, "overwrite": overwrite}),
			Command::Publish {
				tool,
				version,
				file,
				platform,
			} => {
				json!({"action": "publish", "tool": tool, "version": version, "file": file, "platform": platform})
			}
			Command::Hook => unreachable!(),
		};
		value
			.as_object_mut()
			.context("Invalid request")?
			.extend(args.as_object().context("Invalid action")?.clone());
		serde_json::from_value(value)?
	};
	let url = request.options.validate()?;
	if request.action == "publish" && request.version.is_none() {
		// Resolve local metadata before acquiring credentials or contacting storage.
		request.version = Some(azure_blob::resolve_publish_version(&request)?);
	}
	let login =
		request.action == "login" || std::env::var("AZURE_BLOB_LOGIN").as_deref() == Ok("1");
	let credential = match auth::credential(&request.options, login).await {
		Ok(credential) => credential,
		Err(error) if error.is::<AuthenticationRequired>() && hook && !login => {
			println!("{}", json!({"authentication_required": true}));
			return Ok(());
		}
		Err(error)
			if error.is::<AuthenticationRequired>() && !hook && !login && !noninteractive() =>
		{
			auth::credential(&request.options, true).await?
		}
		Err(error) => return Err(error),
	};
	if login {
		// The streamed hook login must not pollute mise ls-remote's stdout.
		if !hook {
			println!("{{}}");
		}
		return Ok(());
	}
	let storage = AzureStorage::new(url, credential)?;
	let result = azure_blob::dispatch(&storage, &request).await?;
	println!("{}", serde_json::to_string(&result)?);
	Ok(())
}
