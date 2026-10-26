# Azure Blob backend for mise

Install private tools from Azure Blob Storage with mise. A small Lua backend
calls a Rust helper that handles authentication, downloads, and safe extraction.
Supports Linux and Windows x86-64; no Azure CLI required.

## Requirements

- A current mise release.
- The helper release matching the plugin version (`v0.5.1`) must be published on
  GitHub. Private release downloads need `GITHUB_TOKEN` or `GH_TOKEN` with read
  access to `SNcom-GmbH/az-mise-plugin`.
- `Storage Blob Data Reader` on the target container. Publishing additionally
  needs `Storage Blob Data Contributor`.

## Usage

```toml
[plugins]
azure-blob = "https://github.com/SNcom-GmbH/az-mise-plugin.git#v0.5.1"

[tools."azure-blob:my-tool"]
version = "1.2.3"
storage_url = "https://youraccount.blob.core.windows.net/tools"
```

```sh
mise trust
mise install
mise ls-remote azure-blob:my-tool
mise exec -- my-tool --version
```

`#v0.5.1` selects the plugin release; `version` selects the tool release. Use a
full commit SHA for production. After changing the plugin ref, reinstall it:

```sh
mise plugins install --force azure-blob 'https://github.com/SNcom-GmbH/az-mise-plugin.git#v0.5.1'
```

The plugin downloads its Rust helper through mise's GitHub backend. mise
verifies release checksums or GitHub attestations where available.

## Tool options

| Option                       | Required | Purpose                                                              |
| ---------------------------- | -------- | -------------------------------------------------------------------- |
| `storage_url`                | yes      | Azure Public Cloud container URL; no SAS URLs.                       |
| `version`                    | yes      | Tool version listed in `index.json`.                                 |
| `sha256`                     | no       | Pin the artifact hash with 64 hex digits.                            |
| `auth`                       | no       | `auto` (default), `broker`, `environment`, `workload`, or `managed`. |
| `tenant_id`                  | no       | WAM tenant; overrides `AZURE_TENANT_ID`.                             |
| `client_id`                  | no       | WAM client; overrides `AZURE_CLIENT_ID`.                             |
| `managed_identity_client_id` | no       | User-assigned identity for `auth = "managed"`.                       |

`storage_url` must be `https://ACCOUNT.blob.core.windows.net/CONTAINER`.
`tenant_id` and `client_id` default to `organizations` and the Azure CLI public
client. Tool options take precedence over the environment variables for
environment and workload identities.

Add `sha256 = "<64 hex characters>"` to the tool entry to pin the artifact
independently of its manifest. `mise.lock` alone does not pin artifact bytes.

## Authentication

- **Windows:** Work account through WAM (Windows Web Account Manager). No
  browser or device-code login.
- **Service principal:** `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`,
  `AZURE_CLIENT_SECRET`.
- **Workload identity:** `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`,
  `AZURE_FEDERATED_TOKEN_FILE`.
- **Managed identity:** Tool option `auth = "managed"`; optionally set
  `managed_identity_client_id` for a user-assigned identity.
- **Existing storage token:** Pass `AZURE_BLOB_STORAGE_TOKEN` in the process
  environment. It overrides other identities; the caller must refresh it.

On Windows, running the release binary once with
`azure-blob login --storage-url <url>` signs in ahead of time. Installs otherwise
sign in on first use.

An empty, invalid, or rejected storage token fails without authentication
fallback. Keep secrets out of configuration and command arguments. Reads require
**Storage Blob Data Reader**; uploads require **Storage Blob Data Contributor**.

See [Azure DevOps integration](docs/azure-devops.md) and
[security and configuration](docs/security.md). Windows/Entra SSO still needs
[workplace verification](docs/workplace-test.md).

## Blob layout

```text
my-tool/index.json
{"versions": ["1.2.3"]}

my-tool/1.2.3/release.json
{
  "platforms": {
    "linux-amd64": {
      "file": "tool.tar.gz",
      "sha256": "<64-character SHA-256>",
      "size": 12345678
    }
  }
}
```

Archives must contain `bin/` at their root. Platforms: `linux-amd64`,
`windows-amd64`. Formats: ZIP, tar.gz, tar.xz, tar.bz2. Downloads are limited to
4 GiB; optional `size` sets a lower byte limit. Upload the archive and manifest
before the index. Only Azure Public Cloud container URLs are supported; no SAS
URLs.

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
azure-blob --storage-url https://youraccount.blob.core.windows.net/tools versions my-tool
```

Uploads require `--overwrite` to replace a blob. Downloads never overwrite
files. `list` defaults to a 100,000-entry limit; use `--limit` to lower it.
`read` returns text up to 16 MiB. Direct downloads and tool artifacts are limited
to 4 GiB.

## Development

```sh
mise install
mise run build
mise run check
mise run ci
```

`mise run check` runs formatting, Clippy, and the tests. `mise run ci` also
builds the release binary; CI sets `MISE_CI_TARGET` so the same tasks use the
matrix target.

Tests run without Azure access. Set `AZURE_BLOB_HELPER` to a local binary to test
without a published helper release. Pushing a `v*` tag publishes Linux-musl and
Windows static-CRT binaries with `SHA256SUMS`.

## License

MIT
