use anyhow::{Context, Result, ensure};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
	collections::HashMap,
	env, fs,
	io::{Cursor, Write},
	path::{Path, PathBuf},
	process::{Command, Output},
};

fn mise(project: &Path, environment: &HashMap<String, String>, args: &[&str]) -> Result<Output> {
	let mut command = Command::new(env::var_os("MISE_BIN").unwrap_or_else(|| "mise".into()));
	for (key, _) in env::vars_os() {
		let name = key.to_string_lossy();
		if name.starts_with("MISE_") || name.starts_with("AZURE_") || name == "CI" {
			command.env_remove(key);
		}
	}
	Ok(command
		.args(["-C"])
		.arg(project)
		.args(args)
		.envs(environment)
		.output()?)
}

fn success(output: Output) -> Result<String> {
	ensure!(
		output.status.success(),
		"mise failed:\n{}\n{}",
		String::from_utf8_lossy(&output.stdout),
		String::from_utf8_lossy(&output.stderr)
	);
	Ok(String::from_utf8(output.stdout)?)
}

#[test]
fn missing_or_empty_storage_url_explains_how_to_fix_the_tool_entry() -> Result<()> {
	let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
	for option in ["", ", storage_url = \"\"", ", storage_url = \" \\t \""] {
		let root = tempfile::tempdir()?;
		let project = root.path().join("consumer project");
		fs::create_dir_all(&project)?;
		fs::write(
			project.join("mise.toml"),
			format!(
				"[plugins]\nazure-blob = {}\n[tools]\n\"azure-blob:install-builder\" = {{ version = \"1.1.2\"{option} }}\n",
				toml::Value::String(repo.to_string_lossy().into_owned())
			),
		)?;
		let mut environment = HashMap::from([
			("MISE_YES".into(), "1".into()),
			("MISE_LOG_LEVEL".into(), "warn".into()),
			(
				"MISE_TRUSTED_CONFIG_PATHS".into(),
				project.to_string_lossy().into_owned(),
			),
			// Validation must fail before attempting to download or run the helper.
			(
				"AZURE_BLOB_HELPER".into(),
				root.path()
					.join("missing-helper")
					.to_string_lossy()
					.into_owned(),
			),
		]);
		for key in [
			"MISE_DATA_DIR",
			"MISE_CACHE_DIR",
			"MISE_CONFIG_DIR",
			"MISE_STATE_DIR",
		] {
			environment.insert(
				key.into(),
				root.path().join(key).to_string_lossy().into_owned(),
			);
		}
		for args in [
			["ls-remote", "azure-blob:install-builder"].as_slice(),
			["install"].as_slice(),
		] {
			let failure = mise(&project, &environment, args)?;
			assert!(!failure.status.success());
			let error = String::from_utf8_lossy(&failure.stderr);
			assert!(
				error.contains("Missing required tool option: storage_url."),
				"option {option:?}: {error}"
			);
			assert!(
				error.contains("mise.toml using a tool options table"),
				"{error}"
			);
			assert!(
				error.contains("[tools.\"azure-blob:install-builder\"]"),
				"{error}"
			);
			assert!(
				error.contains("storage_url = \"https://ACCOUNT.blob.core.windows.net/CONTAINER\""),
				"{error}"
			);
			assert!(error.contains("Keep your existing tool version"), "{error}");
			assert!(!error.contains("missing-helper"), "{error}");
			assert!(
				error.contains("runtime error: Missing required tool option"),
				"{error}"
			);
		}
	}
	Ok(())
}

#[test]
fn real_mise_lua_rust_install_reuse_and_upgrade() -> Result<()> {
	run_install(None, false)
}

#[test]
fn storage_token_survives_direct_and_nested_mise_with_debug_logging() -> Result<()> {
	for nested in [false, true] {
		run_install(Some("token"), nested)?;
	}
	Ok(())
}

#[test]
fn environment_credentials_survive_direct_and_nested_mise() -> Result<()> {
	for nested in [false, true] {
		run_install(Some("environment"), nested)?;
	}
	Ok(())
}

#[test]
fn supplied_token_never_retries_login_and_child_errors_are_redacted() -> Result<()> {
	for nested in [false, true] {
		for mode in ["required", "stderr"] {
			run_install(Some(mode), nested)?;
		}
	}
	Ok(())
}

fn assert_no_secret(output: &Output) {
	for bytes in [&output.stdout, &output.stderr] {
		let text = String::from_utf8_lossy(bytes);
		assert!(
			!text.contains("integration-storage-token"),
			"Storage token leaked in output"
		);
		assert!(
			!text.contains("integration-client-secret"),
			"Client secret leaked in output"
		);
	}
}

fn assert_no_secret_files(path: &Path) -> Result<()> {
	if !path.exists() || fs::symlink_metadata(path)?.file_type().is_symlink() {
		return Ok(());
	}
	if path.is_dir() {
		for entry in fs::read_dir(path)? {
			assert_no_secret_files(&entry?.path())?;
		}
	} else {
		let bytes = fs::read(path)?;
		for secret in [
			b"integration-storage-token".as_slice(),
			b"integration-client-secret".as_slice(),
		] {
			assert!(
				!bytes.windows(secret.len()).any(|window| window == secret),
				"Secret persisted in {}",
				path.display()
			);
		}
	}
	Ok(())
}

fn run_install(auth_mode: Option<&str>, nested: bool) -> Result<()> {
	let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
	let helper = PathBuf::from(env!("CARGO_BIN_EXE_mock_helper"));
	ensure!(
		helper.is_file(),
		"Run mise run test (or cargo test --features test-helper) first"
	);
	let root = tempfile::Builder::new()
		.prefix("azure mise integration spaces ")
		.tempdir()?;
	let project = root.path().join("consumer project");
	fs::create_dir_all(&project)?;
	let storage = root.path().join("storage/hello/1.0.0");
	fs::create_dir_all(&storage)?;
	let os = if cfg!(windows) { "windows" } else { "linux" };
	let arch = match env::consts::ARCH {
		"x86_64" => "amd64",
		other => anyhow::bail!("Unsupported test architecture {other}"),
	};
	let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
	zip.start_file(
		if cfg!(windows) {
			"bin/hello.cmd"
		} else {
			"bin/hello"
		},
		zip::write::SimpleFileOptions::default()
			.compression_method(zip::CompressionMethod::Stored)
			.unix_permissions(0o755),
	)?;
	zip.write_all(if cfg!(windows) {
		b"@echo off\r\necho from-mock\r\n"
	} else {
		b"#!/bin/sh\necho from-mock\n"
	})?;
	let bytes = zip.finish()?.into_inner();
	fs::write(storage.join("hello.zip"), &bytes)?;
	fs::write(
		storage.join("release.json"),
		json!({"platforms": {format!("{os}-{arch}"): {
			"file": "hello.zip", "sha256": hex::encode(Sha256::digest(&bytes))
		}}})
		.to_string(),
	)?;
	fs::write(
		storage
			.parent()
			.context("Invalid storage path")?
			.join("index.json"),
		json!({"versions": ["1.0.0"]}).to_string(),
	)?;
	fs::write(
		project.join("mise.toml"),
		format!(
			"[plugins]\nazure-blob = {}\n[tools]\n\"azure-blob:hello\" = {{ version = \"1.0.0\", storage_url = \"https://account.blob.core.windows.net/tools\" }}\n",
			toml::Value::String(repo.to_string_lossy().into_owned())
		),
	)?;
	let mut environment = HashMap::from([
		("MISE_YES".into(), "1".into()),
		("MISE_LOG_LEVEL".into(), "warn".into()),
		(
			"MISE_TRUSTED_CONFIG_PATHS".into(),
			project.to_string_lossy().into_owned(),
		),
		(
			"AZURE_BLOB_HELPER".into(),
			helper.to_string_lossy().into_owned(),
		),
		(
			"MOCK_AZURE_STORAGE".into(),
			root.path().join("storage").to_string_lossy().into_owned(),
		),
		(
			"MOCK_AZURE_LOGIN".into(),
			root.path()
				.join("authenticated")
				.to_string_lossy()
				.into_owned(),
		),
	]);
	for key in [
		"MISE_DATA_DIR",
		"MISE_CACHE_DIR",
		"MISE_CONFIG_DIR",
		"MISE_STATE_DIR",
	] {
		environment.insert(
			key.into(),
			root.path().join(key).to_string_lossy().into_owned(),
		);
	}
	if let Some(mode) = auth_mode {
		environment.insert("MOCK_AZURE_AUTH_MODE".into(), mode.into());
		environment.insert("MISE_LOG_LEVEL".into(), "trace".into());
		if mode == "environment" {
			environment.insert("CI".into(), "1".into());
			environment.insert("AZURE_TENANT_ID".into(), "integration-tenant".into());
			environment.insert("AZURE_CLIENT_ID".into(), "integration-client".into());
			environment.insert(
				"AZURE_CLIENT_SECRET".into(),
				"integration-client-secret".into(),
			);
		} else {
			environment.insert(
				"AZURE_BLOB_STORAGE_TOKEN".into(),
				"integration-storage-token".into(),
			);
			if mode == "token" {
				environment.insert("CI".into(), "1".into());
			}
		}
	}
	if nested {
		// Register the test executable as an already-installed GitHub helper.
		// The production Lua command still runs the real nested mise exec with
		// the exact release/asset selector, without network or a published release.
		let helper_backend = root
			.path()
			.join("MISE_DATA_DIR/installs/github-s-ncom-gmb-h-az-mise-plugin");
		let fixture = helper_backend.join(env!("CARGO_PKG_VERSION"));
		fs::create_dir_all(&fixture)?;
		fs::copy(
			&helper,
			fixture.join(if cfg!(windows) {
				"azure-blob.exe"
			} else {
				"azure-blob"
			}),
		)?;
		fs::write(
			helper_backend.join(".mise.backend.toml"),
			"short = \"github:SNcom-GmbH/az-mise-plugin\"\nfull = \"github:SNcom-GmbH/az-mise-plugin\"\n",
		)?;
		environment.remove("AZURE_BLOB_HELPER");
	}
	if matches!(auth_mode, Some("required" | "stderr")) {
		let failure = mise(&project, &environment, &["ls-remote", "azure-blob:hello"])?;
		assert_no_secret(&failure);
		assert!(!failure.status.success());
		let error = String::from_utf8_lossy(&failure.stderr);
		assert!(if auth_mode == Some("required") {
			error.contains("caller must acquire a new token")
		} else {
			error.contains("[redacted]")
		});
		assert!(!root.path().join("authenticated").exists());
		return Ok(());
	}
	// The project declaration installs the local plugin automatically; no manual link step.
	let versions = mise(&project, &environment, &["ls-remote", "azure-blob:hello"])?;
	assert_no_secret(&versions);
	assert_eq!(success(versions)?.trim(), "1.0.0");
	if auth_mode.is_some() {
		let lock = mise(&project, &environment, &["lock"])?;
		assert_no_secret(&lock);
		success(lock)?;
		let install = mise(&project, &environment, &["install", "--locked"])?;
		assert_no_secret(&install);
		success(install)?;
		assert!(!root.path().join("authenticated").exists());
		for path in [
			&project,
			&root.path().join("MISE_CACHE_DIR"),
			&root.path().join("MISE_STATE_DIR"),
			&root.path().join("MISE_CONFIG_DIR"),
			&root.path().join("MISE_DATA_DIR/downloads"),
		] {
			assert_no_secret_files(path)?;
		}
		return Ok(());
	}
	assert!(root.path().join("authenticated").is_file());
	success(mise(&project, &environment, &["install"])?)?;
	let args: &[&str] = if cfg!(windows) {
		&["exec", "--", "cmd", "/c", "hello"]
	} else {
		&["exec", "--", "hello"]
	};
	assert_eq!(
		success(mise(&project, &environment, args)?)?.trim(),
		"from-mock"
	);
	// Reproduce a legacy publisher's newest-first index, with 1.1.2 installed
	// and locked. A real mise upgrade --bump must select and install 1.1.3.
	let tool_storage = storage.parent().context("Invalid storage path")?;
	for version in ["1.1.2", "1.1.3"] {
		let release = tool_storage.join(version);
		fs::create_dir_all(&release)?;
		for file in ["hello.zip", "release.json"] {
			fs::copy(storage.join(file), release.join(file))?;
		}
	}
	fs::write(
		tool_storage.join("index.json"),
		json!({"versions": ["1.1.3", "1.1.2", "1.0.0"]}).to_string(),
	)?;
	let config = project.join("mise.toml");
	fs::write(
		&config,
		fs::read_to_string(&config)?.replace("version = \"1.0.0\"", "version = \"1.1.2\""),
	)?;
	success(mise(
		&project,
		&environment,
		&["cache", "clear", "azure-blob:hello"],
	)?)?;
	success(mise(&project, &environment, &["install"])?)?;
	success(mise(&project, &environment, &["lock"])?)?;
	assert_eq!(
		success(mise(
			&project,
			&environment,
			&["ls-remote", "azure-blob:hello"]
		)?)?
		.trim(),
		"1.0.0\n1.1.2\n1.1.3"
	);
	assert_eq!(
		success(mise(
			&project,
			&environment,
			&["latest", "azure-blob:hello"]
		)?)?
		.trim(),
		"1.1.3"
	);
	success(mise(
		&project,
		&environment,
		&["upgrade", "--bump", "azure-blob:hello"],
	)?)?;
	let config: toml::Value = toml::from_str(&fs::read_to_string(config)?)?;
	assert_eq!(
		config["tools"]["azure-blob:hello"]["version"].as_str(),
		Some("1.1.3")
	);
	assert_eq!(
		success(mise(&project, &environment, args)?)?.trim(),
		"from-mock"
	);
	environment.insert("AZURE_BLOB_NONINTERACTIVE".into(), "1".into());
	success(mise(
		&project,
		&environment,
		&["cache", "clear", "azure-blob:hello"],
	)?)?;
	assert!(
		success(mise(
			&project,
			&environment,
			&["ls-remote", "azure-blob:hello"]
		)?)?
		.contains("1.0.0")
	);
	fs::remove_file(root.path().join("authenticated"))?;
	success(mise(
		&project,
		&environment,
		&["cache", "clear", "azure-blob:hello"],
	)?)?;
	let failure = mise(&project, &environment, &["ls-remote", "azure-blob:hello"])?;
	assert!(!failure.status.success());
	assert!(String::from_utf8_lossy(&failure.stderr).contains("sign-in required"));
	Ok(())
}
