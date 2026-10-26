# Security and operations

## Identities and options

### Supplied storage token

When `AZURE_BLOB_STORAGE_TOKEN` is set, the helper uses that bearer token and
nothing else for blob metadata, indexes, manifests, and artifacts. It takes
precedence over every other identity and over valid `auth` modes, including
`broker`, `environment`, `workload`, and `managed`.

The value must be a nonempty bearer-syntax token without the `Bearer ` prefix,
whitespace, or control characters. The plugin treats tokens as opaque; Azure
checks the signature, audience, and expiry. Invalid values and HTTP 401/403 abort
the operation without switching identity or opening a WAM dialog. The caller
acquires and renews the token; the helper has no refresh credentials. See
[Azure DevOps](azure-devops.md).

### Windows/WAM (Web Account Manager)

`tenant_id` and `client_id` are tool options. They apply to the WAM broker and
are not stored as secrets. The default client is the public Azure CLI app;
corporate policy may block it.

### Environment and workload identity (Linux/CI)

`tenant_id` and `client_id` tool options take precedence over `AZURE_TENANT_ID`
and `AZURE_CLIENT_ID`. The client secret or federated assertion comes only from
the environment: `AZURE_CLIENT_SECRET` or `AZURE_FEDERATED_TOKEN_FILE`.

### Managed identity

Managed identity is used only with an explicit `auth = "managed"`. Select a
user-assigned identity with `managed_identity_client_id` or
`--managed-identity-client-id`; without one, the helper tries the system-assigned
identity.

### No interactive fallback

There is no browser or device-code login and no Azure CLI fallback. On Linux,
missing CI identities are rejected without an interactive fallback. Without
`AZURE_BLOB_STORAGE_TOKEN`, the existing identity selection is unchanged. `CS_*`
code-signing credentials are never used for Azure Blob Storage.

## WAM and app registration

A custom public-client app registration typically needs:

- the **Mobile and desktop applications** platform, or public-client flows;
- delegated Azure Storage permission (`user_impersonation`);
- the broker redirect URI `ms-appx-web://Microsoft.AAD.BrokerPlugin/<client_id>`;
- approval through tenant policy, Conditional Access, and MFA where required.

The WAM adapter stores only the selected Windows account ID in
`%LOCALAPPDATA%\mise-azure-blob\<hash>.json`; it never stores access or refresh
tokens. Deleting that file removes the saved selection but does not force an
account change; that follows Windows and broker policy.
`AZURE_BLOB_NONINTERACTIVE=1` prevents every WAM dialog.

## Artifact trust

The helper verifies the SHA-256 from `tool/version/release.json` before
extraction. The manifest and artifact live in the same container, however, so
anyone with write access to the container can replace both.

- `mise.lock` stores only the version for backend plugins, not a URL or checksum.
  A lockfile therefore does not guarantee immutable artifact bytes.
- The optional `sha256` tool option pins the artifact hash independently of the
  manifest. If the pin differs from `release.json`, nothing is downloaded or
  installed.
- Downloads are limited to 4 GiB per file or artifact; `size` in the manifest can
  lower that limit. The limit protects against accidentally oversized blobs, not
  against unauthorized access.
- For stronger protection, minimize write access to the container, enable blob
  immutability or versioning, publish releases only through a controlled
  pipeline, and use signed manifests where needed.
- The helper itself is fetched through mise's GitHub backend. mise verifies
  release checksums or GitHub attestations where available. Pin both the plugin
  ref and the release tag.

## Windows deployment

- The release binary is not Authenticode-signed. AppLocker, SmartScreen, Defender
  ASR, or Application Control may block it. Signing or a hash/publisher allowlist
  may be required.
- WAM needs an interactive Windows desktop session. In services, SSH sessions, or
  without a desktop, interactive login fails in a controlled way.
- WAM, the broker, and Conditional Access must be available on the target device.
  A successful CI build does not replace a
  [workplace test](workplace-test.md).

## Cloud, network, and cryptography

- Only **Azure Public Cloud** is supported: `*.blob.core.windows.net` and
  `https://storage.azure.com/.default`. Sovereign clouds, Azure Stack, and
  Azurite are rejected by the URL and scope validation.
- TLS uses Rustls with the platform certificate store; containers must provide CA
  certificates. `system-proxy` uses system or environment proxies; PAC, NTLM, and
  Kerberos proxy authentication are not guaranteed.
- Static musl binaries use the musl resolver. Corporate DNS, search domains, and
  split DNS in the CI network must be tested.
- Rustls/ring is not FIPS 140 validated. If FIPS is required, use an appropriate
  cryptographic provider or a separate build.

## Secrets and logging

- Secrets are read only from environment variables and are never written to tool
  options, command lines, or logs.
- `AZURE_BLOB_STORAGE_TOKEN` passes as an environment variable through Lua and the
  nested `mise exec` to the helper. It must never appear in request JSON,
  configuration, lockfiles, caches, artifacts, or pipeline output variables. The
  Rust credential type redacts debug output, and Lua and the helper also redact the
  token in error contexts. Do not enable environment dumps or shell traces in the
  calling task.
- Errors include the HTTP status and a filtered Azure error code, but no response
  bodies. There is no separate `--debug`; `RUST_BACKTRACE=1` remains the
  deliberate debug option.
