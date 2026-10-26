# Azure Blob backend for mise

Install private tools from Azure Blob Storage. A small Lua plugin calls a Rust
helper with Rustls. Supports **Windows and Linux x86-64**. No Azure CLI required.

## Usage

```toml
[plugins]
azure-ac = "https://github.com/SNcom-GmbH/az-mise-plugin.git#v0.4.0"

[tools]
"azure-ac:my-tool" = { version = "1.2.3", storage_url = "https://youraccount.blob.core.windows.net/tools" }
```

```sh
mise trust
mise install
mise ls-remote azure-ac:my-tool
mise exec -- my-tool --version
```

`#v0.4.0` pins the plugin version. `version = "1.2.3"` selects the tool version
from Azure Blob Storage. To use another plugin version, change the tag after `#`.
If the plugin is already installed, reinstall it after changing the tag:

```sh
mise plugins install --force azure-ac 'https://github.com/SNcom-GmbH/az-mise-plugin.git#v0.4.0'
```

The plugin downloads its helper automatically from GitHub release `v0.4.0`.
That release must be published. Private downloads require `GITHUB_TOKEN` or
`GH_TOKEN`. For production, pin the plugin ref to a full commit SHA.

Optionally, add `sha256 = "<64 hex characters>"` to the tool entry to pin the
artifact independently of `release.json`. A checksum mismatch prevents installation.

## Authentication

- **Azure DevOps with InstallBuilder@2:** The task uses the `az-development` ARM
  service connection, including Workload Identity Federation, to acquire an access
  token for `https://storage.azure.com/.default`. It passes the token to
  `mise install --locked` only through the `AZURE_AC_STORAGE_TOKEN` process
  environment variable. This token takes precedence over all identities and any
  valid `auth` option. The caller acquires and refreshes it; the helper cannot
  refresh it. See the [Azure DevOps contract](docs/azure-devops.md) for details and
  rollout order.
- **Windows:** Uses your work account through WAM, silently where possible.
  No browser or device-code login. Optionally set `tenant_id` and `client_id`
  as tool options.
- **Azure Pipelines/Linux:** Use a service principal with `AZURE_TENANT_ID`,
  `AZURE_CLIENT_ID`, and `AZURE_CLIENT_SECRET`. Set `auth = "environment"` or use
  automatic detection. The `tenant_id` and `client_id` tool options override the
  corresponding `AZURE_*` variables. Pass secrets from the pipeline secret store
  as environment variables. Never commit them.
- **Workload Identity without a pre-acquired storage token:** Set `AZURE_TENANT_ID`,
  `AZURE_CLIENT_ID`, and `AZURE_FEDERATED_TOKEN_FILE`, optionally with
  `auth = "workload"`. The file contains the federated assertion for token
  exchange, not a storage access token.
- **Managed Identity:** Set `auth = "managed"`, optionally with
  `managed_identity_client_id = "..."` for a user-assigned identity.
- Required roles: **Storage Blob Data Reader** for reads and
  **Storage Blob Data Contributor** for uploads.

If `AZURE_AC_STORAGE_TOKEN` is set but empty or syntactically invalid, the helper
fails. It does not fall back to another identity or an interactive login, including
on HTTP 401/403. Without this variable, the existing authentication methods apply.
`CS_*` signing credentials are never used for Blob access.

Windows/Entra SSO still needs verification on a work computer. See the
[test checklist](docs/workplace-test.md) and the
[operations and security notes](docs/security.md).

## Blob layout

- `my-tool/index.json`: `{"versions": ["1.2.3"]}`
- `my-tool/1.2.3/release.json`: `{"platforms": {"linux-amd64": {"file": "tool.tar.gz", "sha256": "<64-character SHA-256>", "size": 12345678}}}`
- `size` is optional and caps the download at the specified byte count.
  Without `size`, the limit is 4 GiB per artifact.
- Archives contain `bin/` at their root. Platforms: `linux-amd64`, `windows-amd64`.
- Formats: ZIP, tar.gz, tar.xz, tar.bz2. Upload the archive and manifest first,
  then the index.

## Manage files

Use the release binary, or replace `azure-ac` with `mise run blob --` in a checkout:

```sh
azure-ac --storage-url https://youraccount.blob.core.windows.net/tools list
azure-ac --storage-url https://youraccount.blob.core.windows.net/tools list --limit 100
azure-ac --storage-url https://youraccount.blob.core.windows.net/tools read notes.txt
azure-ac --storage-url https://youraccount.blob.core.windows.net/tools download notes.txt ./notes.txt
azure-ac --storage-url https://youraccount.blob.core.windows.net/tools upload notes.txt ./notes.txt
```

Uploads overwrite existing blobs only with `--overwrite`. Downloads never overwrite
existing files and are limited to 4 GiB per file. `list` returns at most 100,000
entries; use `--limit` to lower that limit.

## Development

`mise install`, `mise run build`, `mise run check`.
Before the first release, set `AZURE_AC_HELPER` to a locally built binary or an
extracted CI artifact. CI builds a static Linux-musl binary and a Windows binary
with a static C runtime. Pushing a `v*` tag publishes the binaries and `SHA256SUMS`
as a GitHub release. Tests run without Azure access. They check actual SDK
Authorization headers and direct and nested mise processes with test credentials,
including trace logging.

## License

MIT
