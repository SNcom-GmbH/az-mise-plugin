use azure_blob::config::DEFAULT_CLIENT;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
	version,
	about = "Azure Blob helper: native Windows WAM and Linux CI credentials; no browser login"
)]
pub(crate) struct Cli {
	#[arg(long)]
	pub(crate) storage_url: Option<String>,
	#[arg(long, default_value = "organizations")]
	pub(crate) tenant_id: String,
	#[arg(long, default_value = DEFAULT_CLIENT)]
	pub(crate) client_id: String,
	#[arg(long, default_value = "auto")]
	pub(crate) auth: String,
	#[arg(long)]
	pub(crate) managed_identity_client_id: Option<String>,
	#[command(subcommand)]
	pub(crate) command: Command,
}

#[derive(Subcommand)]
pub(crate) enum Command {
	#[command(hide = true)]
	Hook,
	Login,
	List {
		#[arg(long, default_value = "")]
		prefix: String,
		#[arg(long)]
		limit: Option<usize>,
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
	/// Package a binary and publish a tool version to Azure Blob Storage.
	#[command(
		allow_missing_positional = true,
		after_help = "Examples:\n  azure-blob --storage-url URL publish my-tool 1.2.3 ./my-tool --platform linux-amd64\n  azure-blob --storage-url URL publish my-tool ./my-tool.exe --platform windows-amd64\n\nOmit VERSION only for windows-amd64 to read the executable's FileVersion metadata.\nThe executable is never run. An explicit VERSION always takes precedence."
	)]
	Publish {
		tool: String,
		/// Tool version. Omit to detect a Windows executable's FileVersion.
		version: Option<String>,
		/// Binary to install under bin/, keeping its filename.
		file: PathBuf,
		#[arg(long, value_parser = ["linux-amd64", "windows-amd64"])]
		platform: String,
	},
}
