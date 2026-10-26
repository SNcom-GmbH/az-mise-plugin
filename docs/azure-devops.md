# Azure DevOps integration

## Contract with InstallBuilder@2

InstallBuilder@2 uses the Azure DevOps ARM service connection `az-development` to
access Azure Blob Storage, including service connections that use Workload
Identity Federation. The task signs in with the service connection and obtains an
access token for the `https://storage.azure.com/.default` scope.

The task starts `mise install --locked` with `AZURE_BLOB_STORAGE_TOKEN` only in
the child process environment. It must preserve the value across every
intermediate process. Never store token values in command arguments, request
JSON, tool options, `mise.toml`, `mise.lock`, temporary files, caches, or build
artifacts. Do not emit them as a pipeline output variable or through
`task.setvariable`. The task must not log the environment or the token response.

The token must be a storage access token. An ARM-audience token or the raw OIDC
assertion from a federated service connection is not enough. The helper does not
read service-connection configuration and does not perform that token exchange.
`AZURE_FEDERATED_TOKEN_FILE` remains the separate, supported path for an
assertion file supplied by the CI provider.

The plugin sends the token as `Authorization: Bearer ...` for `index.json`,
`release.json`, blob metadata, and artifact downloads. Direct blob commands use
the same identity. The token takes precedence over every valid `auth` option,
WAM, service principals, workload identity, and managed identity. Without the
variable, the existing authentication remains. An empty value counts as set and
fails. The helper also rejects whitespace, control characters, a `Bearer `
prefix, and other invalid bearer syntax. Azure validates the claims and
signature; the plugin does not require a specific JWT format.

HTTP 401 and 403, and token validation failures, end the operation. There is no
switch to another identity and no interactive login. On 401, the caller must
check audience, validity, and identity, then obtain a new token if needed. On
403, check data-plane permissions and network rules. `CS_*` code-signing
credentials must never be used for blob access.

## Lifetime and permissions

The caller is responsible for acquiring and renewing the token. The helper cannot
renew a supplied token and keeps it only in process memory. The SDK token cache
does not extend its actual lifetime. Before longer installations, the task must
obtain a token with a sufficiently long lifetime. A new token in the parent
process does not change the environment of an already running helper. After
expiry, a new invocation with a new token is required.

The service-connection identity needs existing data-plane permissions on the
target container, for example `Storage Blob Data Reader` to read. An ARM role
alone does not grant blob data access. Network rules and private endpoints still
apply. Introducing this plugin does not change any Azure permissions.

## Deployment order

1. In `SNcom-GmbH/az-mise-plugin`, publish the helper binaries and `SHA256SUMS`
   for the release. Both platform assets must be available:
   `azure-blob-linux-amd64-musl.tar.gz` and `azure-blob-windows-amd64.zip`.
2. Then provide the updated Lua revision. It loads the helper from the new
   repository at the published version.
3. In consumer repositories, update the plugin URLs to
   `https://github.com/SNcom-GmbH/az-mise-plugin.git` and the commit pins to the
   full SHA of that revision. Existing pins do not switch repository or helper
   version automatically. Keep using lockfiles.
4. Only then enable InstallBuilder@2 service-connection support and use
   `az-development`. The task must set `AZURE_BLOB_STORAGE_TOKEN`; earlier
   variable names are not supported.

Private GitHub release assets additionally need `GITHUB_TOKEN` or `GH_TOKEN`
with read access to the new repository. SSH access to the plugin alone is not
enough. The storage token does not replace these GitHub credentials.

The task implementation and its inputs live in the InstallBuilder repository.
This plugin implements the environment contract; it does not enable a task,
publish a release, or trigger a pipeline.

## Verification

`mise run check` runs Rust formatting, Clippy, unit tests, and real mise/Lua
integration tests without Azure access. The tests use artificial credentials
only. They cover direct and nested processes, `mise install --locked`, the SDK
authorization headers for index, manifest, and artifact, token precedence,
redaction, HTTP 401/403, and the existing environment login.

Before activation, a test in the target tenant is still required: install with
the `az-development` service connection, including its federated variant, on
Windows and Linux agents. Check token expiry, missing blob permissions, network
rules, and private GitHub downloads. Successful local tests do not replace this
test. Do not put token values in the test record. See
[security and operations](security.md) and the
[workplace test](workplace-test.md).
