use std::{fs, path::Path};

#[test]
fn plugin_versions_match_the_crate_version() {
	let version = env!("CARGO_PKG_VERSION");
	let root = Path::new(env!("CARGO_MANIFEST_DIR"));

	let metadata = fs::read_to_string(root.join("metadata.lua")).expect("metadata.lua");
	assert!(
		metadata.contains(&format!("version = \"{version}\"")),
		"metadata.lua must declare version {version}"
	);

	let lua = fs::read_to_string(root.join("lib/azure.lua")).expect("lib/azure.lua");
	assert!(
		lua.contains(&format!("@{version}\"")),
		"lib/azure.lua must pin the helper to {version}"
	);

	let readme = fs::read_to_string(root.join("README.md")).expect("README.md");
	assert!(
		readme.contains(&format!("v{version}")),
		"README.md must mention helper release v{version}"
	);
}

#[test]
fn backend_and_release_assets_match_the_binary_name() {
	let name = env!("CARGO_PKG_NAME");
	assert_eq!(name, "azure-blob");
	let root = Path::new(env!("CARGO_MANIFEST_DIR"));
	let metadata = fs::read_to_string(root.join("metadata.lua")).expect("metadata.lua");
	assert!(metadata.contains(&format!("name = \"{name}\"")));
	let lua = fs::read_to_string(root.join("lib/azure.lua")).expect("lib/azure.lua");
	let workflow = fs::read_to_string(root.join(".github/workflows/build.yml")).expect("build.yml");
	let mise = fs::read_to_string(root.join("mise.toml")).expect("mise.toml");
	assert!(lua.contains(&format!("-- {name} hook")));
	assert!(mise.contains(&format!("--bin {name}")));
	for suffix in ["linux-amd64-musl.tar.gz", "windows-amd64.zip"] {
		let asset = format!("{name}-{suffix}");
		assert!(lua.contains(&asset), "Lua must select {asset}");
		assert!(workflow.contains(&asset), "CI must publish {asset}");
	}
	let example = fs::read_to_string(root.join("examples/mise.toml")).expect("example mise.toml");
	assert!(example.contains(&format!("{name} = ")));
	assert!(example.contains(&format!("\"{name}:hello\"")));
}
