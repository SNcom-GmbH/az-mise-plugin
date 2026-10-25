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
