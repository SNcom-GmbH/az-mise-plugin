//! Small PE32+ resource fixtures. No compiler, Windows SDK, or runnable code is needed.

fn word(bytes: &mut [u8], offset: usize, value: u16) {
	bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn dword(bytes: &mut [u8], offset: usize, value: u32) {
	bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn utf16(value: &str) -> Vec<u8> {
	value
		.encode_utf16()
		.chain([0])
		.flat_map(u16::to_le_bytes)
		.collect()
}

fn block(key: &str, text: bool, value: &[u8], children: &[Vec<u8>]) -> Vec<u8> {
	let mut bytes = vec![0; 6];
	bytes.extend(utf16(key));
	bytes.resize(bytes.len().next_multiple_of(4), 0);
	bytes.extend(value);
	for child in children {
		bytes.resize(bytes.len().next_multiple_of(4), 0);
		bytes.extend(child);
	}
	let len = bytes.len() as u16;
	word(&mut bytes, 0, len);
	word(
		&mut bytes,
		2,
		(if text { value.len() / 2 } else { value.len() }) as u16,
	);
	word(&mut bytes, 4, u16::from(text));
	bytes
}

pub fn executable(version: Option<&str>, fixed: Option<[u16; 4]>) -> Vec<u8> {
	let versions = version
		.map(|version| ("040904B0", version))
		.into_iter()
		.collect::<Vec<_>>();
	with_languages(&versions, fixed)
}

pub fn with_languages(versions: &[(&str, &str)], fixed: Option<[u16; 4]>) -> Vec<u8> {
	let fixed = fixed
		.map(|[major, minor, patch, revision]| {
			[
				0xFEEF04BD,
				0x00010000,
				(u32::from(major) << 16) | u32::from(minor),
				(u32::from(patch) << 16) | u32::from(revision),
				0x00090009,
				0x00090009, // Distinct ProductVersion must never be selected.
				0x3f,
				0,
				0x00040004,
				1,
				0,
				0,
				0,
			]
			.into_iter()
			.flat_map(u32::to_le_bytes)
			.collect::<Vec<_>>()
		})
		.unwrap_or_default();
	let mut tables = versions
		.iter()
		.map(|(language, version)| {
			block(
				language,
				true,
				&[],
				&[block("FileVersion", true, &utf16(version), &[])],
			)
		})
		.collect::<Vec<_>>();
	// A ProductVersion alone is insufficient for automatic publication.
	tables.push(block(
		"041104B0",
		true,
		&[],
		&[block("ProductVersion", true, &utf16("9.9.9.9"), &[])],
	));
	let strings = vec![block("StringFileInfo", true, &[], &tables)];
	let version = block("VS_VERSION_INFO", false, &fixed, &strings);
	let resource_size = 96 + version.len();
	let raw_size = resource_size.next_multiple_of(512);
	let mut bytes = vec![0; 512 + raw_size];
	bytes[..2].copy_from_slice(b"MZ");
	dword(&mut bytes, 0x3c, 0x80);
	bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
	word(&mut bytes, 0x84, 0x8664); // AMD64
	word(&mut bytes, 0x86, 1); // One .rsrc section
	word(&mut bytes, 0x94, 240);
	word(&mut bytes, 0x96, 0x22);
	let optional = 0x98;
	word(&mut bytes, optional, 0x20b); // PE32+
	dword(&mut bytes, optional + 32, 4096);
	dword(&mut bytes, optional + 36, 512);
	dword(&mut bytes, optional + 56, 8192);
	dword(&mut bytes, optional + 60, 512);
	dword(&mut bytes, optional + 108, 16);
	dword(&mut bytes, optional + 128, 4096); // Resource directory RVA
	dword(&mut bytes, optional + 132, resource_size as u32);
	let section = optional + 240;
	bytes[section..section + 8].copy_from_slice(b".rsrc\0\0\0");
	dword(&mut bytes, section + 8, resource_size as u32);
	dword(&mut bytes, section + 12, 4096);
	dword(&mut bytes, section + 16, raw_size as u32);
	dword(&mut bytes, section + 20, 512);
	dword(&mut bytes, section + 36, 0x40000040);
	let resources = &mut bytes[512..];
	for (directory, id, target) in [(0, 16, 0x80000018), (24, 1, 0x80000030), (48, 0x409, 80)] {
		word(resources, directory + 14, 1);
		dword(resources, directory + 16, id);
		dword(resources, directory + 20, target);
	}
	dword(resources, 80, 4096 + 96);
	dword(resources, 84, version.len() as u32);
	resources[96..96 + version.len()].copy_from_slice(&version);
	bytes
}

pub fn pe32(mut bytes: Vec<u8>) -> Vec<u8> {
	// PE32 places the data directories and section table 16 bytes earlier.
	bytes.copy_within(0x108..0x1b0, 0xf8);
	word(&mut bytes, 0x84, 0x14c);
	word(&mut bytes, 0x94, 224);
	word(&mut bytes, 0x98, 0x10b);
	dword(&mut bytes, 0xf4, 16);
	bytes
}
