# Publication version detection

The publish command accepts both forms:

```text
azure-blob --storage-url URL publish TOOL VERSION FILE --platform PLATFORM
azure-blob --storage-url URL publish TOOL FILE --platform windows-amd64
```

An explicit `VERSION` works for `linux-amd64` and `windows-amd64`. It overrides
executable metadata and keeps the existing version validation. The file does not
need version metadata when a version is supplied.

When `VERSION` is omitted, the file must be a Windows PE executable and
the platform must be `windows-amd64`. The publisher reads the file's version
resource directly on Linux or Windows. It never runs the executable or invokes
PowerShell, Wine, or another program to detect the version.

Detection uses the `FileVersion` string when present. Localized strings within
the version resource must agree. The string is used exactly as stored, including
prerelease and build suffixes. It must be a safe filename made of ASCII letters,
digits, periods, hyphens, underscores, or plus signs. Empty strings, whitespace,
paths, and reserved Windows filenames are rejected.

When no `FileVersion` string is present, detection uses the fixed numeric file
version as `major.minor.patch.revision`. All four components are preserved, so
`1.2.3.0` remains `1.2.3.0`. `ProductVersion` is never used. An invalid
`FileVersion` string fails instead of falling back to a different version.

If the executable is unreadable, malformed, unversioned, or has an unusable
version, the command fails with a hint to supply `VERSION` explicitly. Detection
and validation finish before storage access, and failure leaves Blob Storage
unchanged. For example:

```text
azure-blob: Could not detect a usable Windows FileVersion; supply VERSION explicitly ...
```

The detected version appears in the command's JSON result and selects the same
blob paths as an explicit version. Publication still uploads the immutable
artifact first, updates the manifest with ETag conditions, and updates the
version index last. Rerun the same command with the same file to resume after a
failure. If the binary changes, give it a new version or update its version
metadata before publishing.

## Version index order

`index.json` lists numeric versions oldest first. mise selects the last matching
entry, so an index containing `1.1.2` and `1.1.3` must put `1.1.3` last:

```json
{"versions": ["1.1.2", "1.1.3"]}
```

The publisher orders three-part versions and four-part Windows versions
numerically, including prerelease precedence. Build metadata does not change
precedence. Other version names retain their source order before numeric
versions. Version objects keep their timestamps and other metadata.

Version listing also applies this ordering to existing indexes. Republishing
the same release repairs an older publisher's reversed index with an
ETag-conditional metadata update. Artifacts remain immutable, and an older
release published later does not replace a higher numeric version as latest.

After updating the plugin and helper, clear any cached version list before
upgrading:

```sh
mise cache clear azure-blob:install-builder
mise upgrade --bump azure-blob:install-builder
```
