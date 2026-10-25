use std::process::Command;

fn cli() -> Command {
	let mut command = Command::new(env!("CARGO_BIN_EXE_azure-ac"));
	for (key, _) in std::env::vars_os() {
		if key.to_string_lossy().starts_with("AZURE_") {
			command.env_remove(key);
		}
	}
	command
}

#[test]
fn linux_without_ci_identity_never_launches_login() {
	if cfg!(windows) {
		return;
	}
	let output = cli()
		.args([
			"--storage-url",
			"https://account.blob.core.windows.net/tools",
			"list",
		])
		.output()
		.expect("CLI");
	assert!(!output.status.success());
	assert!(String::from_utf8_lossy(&output.stderr).contains("CI-only"));
}

#[test]
fn failed_ci_configuration_does_not_fall_back_to_a_user() {
	let output = cli()
		.env("AZURE_CLIENT_SECRET", "fake-secret-do-not-print")
		.args([
			"--storage-url",
			"https://account.blob.core.windows.net/tools",
			"list",
		])
		.output()
		.expect("CLI");
	assert!(!output.status.success());
	let error = String::from_utf8_lossy(&output.stderr);
	assert!(error.contains("AZURE_TENANT_ID is required"));
	assert!(!error.contains("fake-secret"));
}

#[test]
fn unsafe_endpoint_is_rejected_before_credentials() {
	let output = cli()
		.env("AZURE_CLIENT_SECRET", "fake-secret")
		.args(["--storage-url", "https://evil.invalid/tools", "list"])
		.output()
		.expect("CLI");
	assert!(!output.status.success());
	assert!(String::from_utf8_lossy(&output.stderr).contains("storage_url must be"));
}
