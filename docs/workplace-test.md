# First test: Windows corporate account and Linux CI

## App registration (WAM)

For a custom public-client app registration:

- Grant the delegated Azure Storage permission `user_impersonation`.
- Add the broker redirect URI `ms-appx-web://Microsoft.AAD.BrokerPlugin/<client_id>`
  under "Mobile and desktop applications".
- Allow public-client flows and tenant/Conditional Access policies.
- The default client is the Azure CLI app; corporate policy may block it.

## Windows

1. Clone the plugin repository and make sure GitHub access works.
2. Get the Windows ZIP from a successful GitHub Actions run or release and extract
   it. Set `AZURE_BLOB_HELPER` to the included `azure-blob.exe`; no Rust compiler is
   needed.
   - Alternatively, in the checkout run `mise trust`, `mise install`,
     `mise run build`, and `mise run check`, then set `AZURE_BLOB_HELPER` to
     `target/x86_64-pc-windows-msvc/debug/azure-blob.exe`.
3. Copy `examples/mise.toml` into a separate test project and replace the account
   and container. Add `tenant_id` explicitly when using multiple tenants.
4. Provide a harmless tool with `bin/NAME.exe` or `bin/NAME.cmd`, a correct
   SHA-256, `release.json`, and `index.json` following the README schema.
5. Optionally set `size` in the manifest and pin `sha256` as a tool option. A
   wrong pin must fail before download.
6. Give the test account `Storage Blob Data Reader`; enable VPN, DNS, and
   private-endpoint network rules if needed. If AppLocker, SmartScreen, or ASR
   blocks the unsigned binary, allow it or sign it.
7. Run `mise trust`, then `mise install`. The existing Windows corporate account
   should work silently where possible; the native WAM dialog appears only if
   needed. There is no separate browser or device-code login and no prior
   `az login`.
8. Check `mise ls-remote azure-blob:NAME` and `mise exec -- NAME --version`.
9. In a new terminal, run `mise cache clear azure-blob:NAME` and query versions
   again. This ensures mise's version cache does not hide the login.
10. Repeat after an access token expires and verify silent WAM renewal.
11. With `AZURE_BLOB_NONINTERACTIVE=1`, no dialog may start; without a usable
    silent login, the operation must fail.
12. Test a wrong `sha256` pin and a too-small `size`: installation must abort
    before extraction and leave the target directory unchanged.
13. Use `MISE_DEBUG=1` if needed; errors include the HTTP status and a filtered
    Azure error code, but no response bodies.

**Not yet verified:** direct WinRT WAM calls in the corporate tenant, default
account selection, tenant switching, consent, and Conditional Access. A successful
Windows build or mock test does not replace this test. The native adapter is not
an MSAL reimplementation and does not copy Azure CLI's subscription or tenant
selection logic.

If there are problems, check the app registration and broker availability. The
provisional development client may be blocked by corporate policy; a custom
public-client app may be required. Do not work around policy with an automatic
device-code fallback. Details: [security.md](security.md).

## Linux/CI

For InstallBuilder@2 with the ARM service connection `az-development`, the
[Azure DevOps contract and deployment order](azure-devops.md) apply. In the target
tenant, also test Workload Identity Federation, token expiry, and HTTP 401/403
without an identity switch. The task passes the storage token to
`mise install --locked` only as `AZURE_BLOB_STORAGE_TOKEN`; the helper cannot
renew it.

1. Use the static musl helper in an agent or container with CA certificates.
2. Set either `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, and
   `AZURE_FEDERATED_TOKEN_FILE`, or a service principal with
   `AZURE_CLIENT_SECRET` from the CI secret store.
3. For workload identity, the CI platform must provide the valid assertion file;
   the plugin does not create federation configuration or a CI OIDC assertion.
4. Set `CI=1`, run `mise install`, and run `mise ls-remote azure-blob:NAME`.
5. Check token renewal during a longer job, including rotation of the assertion
   file by the CI provider.
6. With missing or wrong credentials, the job must fail without a dialog, browser,
   Azure CLI call, or switch to a personal identity.
7. Use managed identity only with an explicit `auth = "managed"` on a suitable
   Azure agent; also set `managed_identity_client_id` for a user-assigned
   identity.

## Common failures

- Wrong checksum, wrong `sha256` pin, exceeded `size`, missing platform, or
  missing `bin/`: no installation.
- Existing nonempty installation directories are not overwritten.
- HTTP 403 after login: check data-plane permissions, tenant, and storage network
  rules.
- HTTP 404: check the blob/container path and `index.json`/`release.json`; this is
  not a permission error.
- HTTP 409 on upload: the blob already exists; use `--overwrite`.
- For private GitHub release assets, provide a suitable GitHub token; SSH access
  to the plugin alone is not enough.

Feedback: include the mise/Windows version or agent image, the command, the error
class/HTTP status, an anonymized configuration, and whether the Microsoft account
window appeared. **Do not share tokens, secrets, SAS URLs, or full credential
logs.**
