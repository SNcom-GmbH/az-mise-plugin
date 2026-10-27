use std::process::Command;

#[path = "helpers/windows_executable.rs"]
mod windows_executable;

fn cli() -> Command {
	let mut command = Command::new(env!("CARGO_BIN_EXE_azure-blob"));
	for (key, _) in std::env::vars_os() {
		if key.to_string_lossy().starts_with("AZURE_") {
			command.env_remove(key);
		}
	}
	command
}

#[test]
fn cli_reports_azure_blob_name() {
	let output = cli().arg("--version").output().expect("CLI version");
	assert!(output.status.success());
	assert_eq!(
		String::from_utf8(output.stdout).expect("version output"),
		format!("azure-blob {}\n", env!("CARGO_PKG_VERSION"))
	);
	let output = cli().arg("--help").output().expect("CLI help");
	assert!(output.status.success());
	assert!(String::from_utf8_lossy(&output.stdout).contains("Usage: azure-blob"));
}

#[test]
fn publish_help_describes_both_version_forms() {
	let output = cli()
		.args(["publish", "--help"])
		.output()
		.expect("CLI help");
	assert!(output.status.success());
	let help = String::from_utf8_lossy(&output.stdout);
	assert!(help.contains("<TOOL> [VERSION] <FILE>"));
	assert!(help.contains("FileVersion"));
	assert!(help.contains("publish my-tool 1.2.3 ./my-tool --platform linux-amd64"));
	assert!(help.contains("publish my-tool ./my-tool.exe --platform windows-amd64"));
}

#[test]
fn publish_parses_explicit_and_automatic_versions_without_credentials() {
	let root = tempfile::tempdir().expect("tempdir");
	let file = root.path().join("my tool.exe");
	std::fs::write(
		&file,
		windows_executable::pe32(windows_executable::with_languages(
			&[("040904B0", "1.2.3"), ("040704B0", "1.2.3")],
			None,
		)),
	)
	.expect("fixture");
	for platform in ["linux-amd64", "windows-amd64"] {
		for explicit in [true, false] {
			let mut command = cli();
			command
				.env("CI", "1")
				.env("AZURE_BLOB_STORAGE_TOKEN", " ")
				.args([
					"--storage-url",
					"https://account.blob.core.windows.net/tools",
					"publish",
					"hello",
				]);
			if explicit {
				command.arg("9.8.7");
			}
			let output = command
				.arg(&file)
				.args(["--platform", platform])
				.output()
				.expect("CLI");
			assert!(!output.status.success());
			let error = String::from_utf8_lossy(&output.stderr);
			assert!(
				error.contains(if !explicit && platform == "linux-amd64" {
					"VERSION is required"
				} else {
					"AZURE_BLOB_STORAGE_TOKEN must contain"
				}),
				"{error}"
			);
		}
	}
}

#[test]
fn publish_reports_missing_windows_metadata_before_authentication() {
	let root = tempfile::tempdir().expect("tempdir");
	let file = root.path().join("unversioned.exe");
	std::fs::write(&file, windows_executable::executable(None, None)).expect("fixture");
	let output = cli()
		.env("CI", "1")
		.args([
			"--storage-url",
			"https://account.blob.core.windows.net/tools",
			"publish",
			"hello",
		])
		.arg(&file)
		.args(["--platform", "windows-amd64"])
		.output()
		.expect("CLI");
	assert!(!output.status.success());
	let error = String::from_utf8_lossy(&output.stderr);
	assert!(
		error.contains("Could not detect a usable Windows FileVersion"),
		"{error}"
	);
	assert!(error.contains("supply VERSION explicitly"));
	assert!(!error.contains("identity") && !error.contains("sign-in"));
	assert!(output.stdout.is_empty());
}

#[test]
fn publish_requires_an_explicit_supported_platform() {
	for platform in [None, Some("darwin-arm64")] {
		let mut command = cli();
		command.args(["publish", "hello", "1.0.0", "hello"]);
		if let Some(platform) = platform {
			command.args(["--platform", platform]);
		}
		let output = command.output().expect("CLI");
		assert!(!output.status.success());
		assert!(String::from_utf8_lossy(&output.stderr).contains("--platform"));
	}
}

#[test]
fn publish_uses_the_same_supplied_token_validation_as_download() {
	for action in [
		vec![
			"publish",
			"hello",
			"1.0.0",
			"hello",
			"--platform",
			"linux-amd64",
		],
		vec!["download", "hello/1.0.0/hello.zip", "hello.zip"],
	] {
		let output = cli()
			.env("CI", "1")
			.env("AZURE_BLOB_STORAGE_TOKEN", " ")
			.args([
				"--storage-url",
				"https://account.blob.core.windows.net/tools",
			])
			.args(action)
			.output()
			.expect("CLI");
		assert!(!output.status.success());
		let error = String::from_utf8_lossy(&output.stderr);
		assert!(error.contains("AZURE_BLOB_STORAGE_TOKEN must contain"));
		assert!(!error.contains("AZURE_TENANT_ID"));
		assert!(output.stdout.is_empty());
	}
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
fn invalid_managed_identity_client_id_is_rejected() {
	let output = cli()
		.args([
			"--storage-url",
			"https://account.blob.core.windows.net/tools",
			"--managed-identity-client-id",
			"bad id",
			"list",
		])
		.output()
		.expect("CLI");
	assert!(!output.status.success());
	assert!(String::from_utf8_lossy(&output.stderr).contains("managed_identity_client_id"));
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

#[test]
fn supplied_token_overrides_every_auth_mode_and_conflicting_identity() {
	for auth in ["auto", "environment", "workload", "managed", "broker"] {
		let output = cli()
			.env("CI", "1")
			.env("AZURE_BLOB_STORAGE_TOKEN", "caller-storage-token")
			.env("AZURE_CLIENT_SECRET", "conflicting-secret")
			.env("AZURE_FEDERATED_TOKEN_FILE", "missing-assertion-file")
			.args([
				"--storage-url",
				"https://account.blob.core.windows.net/tools",
				"--auth",
				auth,
				"login",
			])
			.output()
			.expect("CLI");
		assert!(output.status.success(), "token must override {auth}");
		assert_eq!(output.stdout, b"{}\n");
		assert!(output.stderr.is_empty());
	}
}

#[test]
fn invalid_supplied_token_fails_closed_for_every_auth_mode() {
	for auth in ["auto", "environment", "workload", "managed", "broker"] {
		for token in [
			"",
			" ",
			"Bearer secret",
			"secret\r\ninjected",
			"a=b",
			"secret\tvalue",
		] {
			let output = cli()
				.env("AZURE_BLOB_STORAGE_TOKEN", token)
				.env("AZURE_CLIENT_SECRET", "unused-secret")
				.args([
					"--storage-url",
					"https://account.blob.core.windows.net/tools",
					"--auth",
					auth,
					"list",
				])
				.output()
				.expect("CLI");
			assert!(!output.status.success());
			let error = String::from_utf8_lossy(&output.stderr);
			assert!(error.contains("AZURE_BLOB_STORAGE_TOKEN must contain"));
			assert!(!error.contains("secret"));
			assert!(!error.contains("AZURE_TENANT_ID"));
			assert!(!error.contains("sign-in required"));
			assert!(output.stdout.is_empty());
		}
	}
}

#[cfg(unix)]
#[test]
fn non_unicode_supplied_token_fails_closed() {
	use std::{ffi::OsString, os::unix::ffi::OsStringExt};
	let output = cli()
		.env("AZURE_BLOB_STORAGE_TOKEN", OsString::from_vec(vec![0xff]))
		.args([
			"--storage-url",
			"https://account.blob.core.windows.net/tools",
			"list",
		])
		.output()
		.expect("CLI");
	assert!(!output.status.success());
	assert!(
		String::from_utf8_lossy(&output.stderr)
			.contains("AZURE_BLOB_STORAGE_TOKEN must be valid Unicode")
	);
}

#[test]
fn errors_redact_supplied_token_even_in_caller_controlled_context() {
	let token = "caller-secret-token";
	let output = cli()
		.env("AZURE_BLOB_STORAGE_TOKEN", token)
		.env(
			"AZURE_BLOB_REQUEST",
			serde_json::json!({
				"action": "versions", "tool": format!("../{token}"),
				"options": {"storage_url": "https://account.blob.core.windows.net/tools"}
			})
			.to_string(),
		)
		.arg("hook")
		.output()
		.expect("CLI");
	assert!(!output.status.success());
	let error = String::from_utf8_lossy(&output.stderr);
	assert!(error.contains("[redacted]"));
	assert!(!error.contains(token));
	assert!(output.stdout.is_empty());
}

#[test]
fn absent_token_preserves_environment_and_workload_selection() {
	for auth in ["auto", "environment", "workload"] {
		let output = cli()
			.env("CI", "1")
			.env("AZURE_CLIENT_SECRET", "environment-secret")
			.env("AZURE_FEDERATED_TOKEN_FILE", "missing-assertion-file")
			.args([
				"--storage-url",
				"https://account.blob.core.windows.net/tools",
				"--auth",
				auth,
				"login",
			])
			.output()
			.expect("CLI");
		assert!(!output.status.success());
		assert!(String::from_utf8_lossy(&output.stderr).contains("AZURE_TENANT_ID is required"));
	}
}

#[test]
fn signing_credentials_never_supply_a_blob_identity() {
	let output = cli()
		.env("CI", "1")
		.env("CS_TENANT_ID", "signing-tenant")
		.env("CS_CLIENT_ID", "signing-client")
		.env("CS_CLIENT_SECRET", "signing-secret")
		.args([
			"--storage-url",
			"https://account.blob.core.windows.net/tools",
			"login",
		])
		.output()
		.expect("CLI");
	assert!(!output.status.success());
	let error = String::from_utf8_lossy(&output.stderr);
	assert!(error.contains("CI-only identity required"));
	assert!(!error.contains("signing-"));
}
