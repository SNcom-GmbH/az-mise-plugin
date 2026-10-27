use serde_json::Value;

#[derive(Eq, Ord, PartialEq, PartialOrd)]
struct NumericVersion {
	numbers: [u64; 4],
	prerelease: semver::Prerelease,
}

fn key(entry: &Value) -> Option<NumericVersion> {
	let name = entry.as_str().or_else(|| entry["version"].as_str())?;
	let name = name.strip_prefix('v').unwrap_or(name);
	let name = if let Some((name, build)) = name.split_once('+') {
		if build.is_empty() {
			return None;
		}
		semver::BuildMetadata::new(build).ok()?;
		name
	} else {
		name
	};
	let (number, prerelease) = name.split_once('-').unwrap_or((name, ""));
	if prerelease.is_empty() && number != name {
		return None;
	}
	let prerelease = semver::Prerelease::new(prerelease).ok()?;
	let mut numbers = [0; 4];
	let mut count = 0;
	for part in number.split('.') {
		if count == numbers.len() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
			return None;
		}
		numbers[count] = part.parse().ok()?;
		count += 1;
	}
	(3..=4).contains(&count).then_some(NumericVersion {
		numbers,
		prerelease,
	})
}

/// mise chooses the last matching version. Keep semantic and Windows numeric
/// versions ascending, preserving metadata and the order of equal-precedence
/// entries. Opaque names retain their source order before numeric versions.
pub(crate) fn order(versions: &mut [Value]) -> bool {
	if versions.is_sorted_by(|left, right| key(left) <= key(right)) {
		return false;
	}
	versions.sort_by_cached_key(key);
	true
}
