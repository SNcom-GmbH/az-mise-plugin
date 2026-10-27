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
