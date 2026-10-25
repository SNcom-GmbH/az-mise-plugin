use anyhow::{Context, Result};
use azure_ac::{
	auth::{self, AuthenticationRequired},
	config::{DEFAULT_CLIENT, Options, Request, noninteractive},
	storage::AzureStorage,
};
use clap::{Parser, Subcommand};
use serde_json::json;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
	version,
	about = "Azure Blob helper: native Windows WAM and Linux CI credentials; no browser login"
)]
struct Cli {
	#[arg(long)]
	storage_url: Option<String>,
	#[arg(long, default_value = "organizations")]
	tenant_id: String,
	#[arg(long, default_value = DEFAULT_CLIENT)]
	client_id: String,
	#[arg(long, default_value = "auto")]
	auth: String,
	#[command(subcommand)]
	command: Command,
}

#[derive(Subcommand)]
enum Command {
	#[command(hide = true)]
	Hook,
	Login,
	List {
		#[arg(long, default_value = "")]
		prefix: String,
	},
	Versions {
		tool: String,
	},
	Read {
		blob: String,
	},
	Download {
		blob: String,
		file: PathBuf,
	},
	Upload {
		blob: String,
		file: PathBuf,
		#[arg(long)]
		overwrite: bool,
	},
}

#[tokio::main]
async fn main() {
	azure_ac::initialize_tls();
	if let Err(error) = run().await {
		// Display only the top-level contextual message, never raw SDK response chains.
		eprintln!("azure-ac: {error}");
		std::process::exit(1);
	}
}

async fn run() -> Result<()> {
	let cli = Cli::parse();
	let hook = matches!(cli.command, Command::Hook);
	let request: Request = if hook {
		serde_json::from_str(&std::env::var("AZURE_AC_REQUEST").context("Missing hook request")?)
			.context("Invalid hook request")?
	} else {
		let mut value = json!({
			"options": Options {
				storage_url: cli.storage_url.context("--storage-url is required")?,
				tenant_id: cli.tenant_id, client_id: cli.client_id, auth: cli.auth,
				managed_identity_client_id: None,
			}
		});
		let args = match cli.command {
			Command::Login => json!({"action": "login"}),
			Command::List { prefix } => json!({"action": "list", "prefix": prefix}),
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
			Command::Hook => unreachable!(),
		};
		value
			.as_object_mut()
			.context("Invalid request")?
			.extend(args.as_object().context("Invalid action")?.clone());
		serde_json::from_value(value)?
	};
	let url = request.options.validate()?;
	let login = request.action == "login" || std::env::var("AZURE_AC_LOGIN").as_deref() == Ok("1");
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
	let result = azure_ac::dispatch(&storage, &request).await?;
	println!("{}", serde_json::to_string(&result)?);
	Ok(())
}
