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
fn real_mise_lua_rust_install_and_silent_reuse() -> Result<()> {
	let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
	let debug = env::current_exe()?
		.parent()
		.context("Missing deps directory")?
		.parent()
		.context("Missing build directory")?
		.to_owned();
	let helper = debug.join("examples").join(if cfg!(windows) {
		"mock_helper.exe"
	} else {
		"mock_helper"
	});
	ensure!(
		helper.is_file(),
		"Run mise run test (or cargo build --example mock_helper) first"
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
			"[plugins]\nazure-ac = {}\n[tools]\n\"azure-ac:hello\" = {{ version = \"1.0.0\", storage_url = \"https://account.blob.core.windows.net/tools\" }}\n",
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
			"AZURE_AC_HELPER".into(),
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
	// The project declaration installs the local plugin automatically; no manual link step.
	assert_eq!(
		success(mise(
			&project,
			&environment,
			&["ls-remote", "azure-ac:hello"]
		)?)?
		.trim(),
		"1.0.0"
	);
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
	environment.insert("AZURE_AC_NONINTERACTIVE".into(), "1".into());
	success(mise(
		&project,
		&environment,
		&["cache", "clear", "azure-ac:hello"],
	)?)?;
	assert!(
		success(mise(
			&project,
			&environment,
			&["ls-remote", "azure-ac:hello"]
		)?)?
		.contains("1.0.0")
	);
	fs::remove_file(root.path().join("authenticated"))?;
	success(mise(
		&project,
		&environment,
		&["cache", "clear", "azure-ac:hello"],
	)?)?;
	let failure = mise(&project, &environment, &["ls-remote", "azure-ac:hello"])?;
	assert!(!failure.status.success());
	assert!(String::from_utf8_lossy(&failure.stderr).contains("sign-in required"));
	Ok(())
}
