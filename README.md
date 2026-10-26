# Azure Blob backend for mise

Install private tools from Azure Blob Storage with mise. Windows and Linux x86-64;
no Azure CLI required.

## Usage

```toml
[plugins]
azure-blob = "https://github.com/SNcom-GmbH/az-mise-plugin.git#v0.5.0"

[tools]
"azure-blob:my-tool" = { version = "1.2.3", storage_url = "https://youraccount.blob.core.windows.net/tools" }
```

```sh
mise trust
mise install
mise ls-remote azure-blob:my-tool
mise exec -- my-tool --version
```

`#v0.5.0` selects the plugin release; `version` selects the tool release.
Use a full commit SHA for production. After changing the plugin ref, reinstall it:

```sh
mise plugins install --force azure-blob 'https://github.com/SNcom-GmbH/az-mise-plugin.git#v0.5.0'
```

The Rust helper is downloaded from GitHub release `v0.5.0`, which must be published.
Private GitHub release downloads require `GITHUB_TOKEN` or `GH_TOKEN`.

Add `sha256 = "<64 hex characters>"` to the tool entry to pin the artifact
independently of its manifest. `mise.lock` alone does not pin artifact bytes.

## Authentication

- **Windows:** Work account through WAM. No browser or device-code login.
- **Service principal:** `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET`.
- **Workload identity:** `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, `AZURE_FEDERATED_TOKEN_FILE`.
- **Managed identity:** Tool option `auth = "managed"`; optionally set
  `managed_identity_client_id` for a user-assigned identity.
- **Existing storage token:** Pass `AZURE_BLOB_STORAGE_TOKEN` in the process
  environment. It overrides other identities; the caller must refresh it.

An empty, invalid, or rejected storage token fails without authentication fallback.
Keep secrets out of configuration and command arguments. Reads require **Storage
Blob Data Reader**; uploads require **Storage Blob Data Contributor**.

See [Azure DevOps integration](docs/azure-devops.md) and
[security and configuration](docs/security.md). Windows/Entra SSO still needs
[workplace verification](docs/workplace-test.md).

## Blob layout

- `my-tool/index.json`: `{"versions": ["1.2.3"]}`
- `my-tool/1.2.3/release.json`: `{"platforms": {"linux-amd64": {"file": "tool.tar.gz", "sha256": "<64-character SHA-256>", "size": 12345678}}}`

Archives must contain `bin/` at their root. Platforms: `linux-amd64`, `windows-amd64`.
Formats: ZIP, tar.gz, tar.xz, tar.bz2. Downloads are limited to 4 GiB; optional
`size` sets a lower byte limit. Upload the archive and manifest before the index.
Only Azure Public Cloud container URLs are supported; no SAS URLs.

## Publish a tool

```sh
azure-blob --storage-url https://youraccount.blob.core.windows.net/tools \
  publish my-tool 1.2.3 ./my-tool --platform linux-amd64
```

Packages a single binary under `bin/`, calculates SHA-256 and size, uploads it,
then updates `release.json` and `index.json`. Use `--platform windows-amd64` for
Windows. Authentication is the same as downloads; publishing requires **Storage
Blob Data Contributor**. Other platforms are preserved. Changed binaries require
a new version; rerunning the same publication is safe, including after failure.

## Manage files

Use the release binary, or `mise run blob --` in a checkout:

```sh
azure-blob --storage-url https://youraccount.blob.core.windows.net/tools list
azure-blob --storage-url https://youraccount.blob.core.windows.net/tools read notes.txt
azure-blob --storage-url https://youraccount.blob.core.windows.net/tools download notes.txt ./notes.txt
azure-blob --storage-url https://youraccount.blob.core.windows.net/tools upload notes.txt ./notes.txt
```

Uploads require `--overwrite` to replace a blob. Downloads never overwrite files.
`list` defaults to a 100,000-entry limit; use `--limit` to lower it.

## Development

```sh
mise install
mise run build
mise run check
```

Tests run without Azure access. Set `AZURE_BLOB_HELPER` to a local binary to test
without a published helper release. Pushing a `v*` tag publishes Linux-musl and
Windows static-CRT binaries with `SHA256SUMS`.

## License

MIT
