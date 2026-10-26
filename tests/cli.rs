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
			.env("AZURE_AC_STORAGE_TOKEN", "caller-storage-token")
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
				.env("AZURE_AC_STORAGE_TOKEN", token)
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
			assert!(error.contains("AZURE_AC_STORAGE_TOKEN must contain"));
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
		.env("AZURE_AC_STORAGE_TOKEN", OsString::from_vec(vec![0xff]))
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
			.contains("AZURE_AC_STORAGE_TOKEN must be valid Unicode")
	);
}

#[test]
fn errors_redact_supplied_token_even_in_caller_controlled_context() {
	let token = "caller-secret-token";
	let output = cli()
		.env("AZURE_AC_STORAGE_TOKEN", token)
		.env(
			"AZURE_AC_REQUEST",
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
