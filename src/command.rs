use azure_ac::config::DEFAULT_CLIENT;
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
}
