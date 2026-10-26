# Working on this repository

The CLI and mise backend are named `azure-blob`; internal variables use
`AZURE_BLOB_*`. This is a breaking rename with no compatibility aliases.

## Layout

- `hooks/` and `lib/azure.lua`: mise backend hooks and Rust helper invocation.
- `src/`: authentication, Azure Blob access, CLI, and archive installation.
- `tests/`: integration tests; `tests/helpers/mock_helper.rs` is a binary gated
  behind the non-default `test-helper` feature, excluded from normal builds and installs.
- `docs/`: Azure DevOps contract, security notes, and Windows test checklist.

## Checks

Use `mise install` to install the pinned Rust toolchain, then `mise run check`
for tests, formatting, and Clippy. Use `mise run test` rather than bare
`cargo test`: it builds the mock helper needed by the mise integration tests.
Tests must run without Azure access or real credentials.

Follow `.editorconfig` and `rustfmt.toml`. Keep test helpers under `tests/`.
Keep README examples minimal; put detailed operational guidance in `docs/`.

## Constraints

- Read `docs/security.md` before changing authentication, downloads, or extraction.
  Read `docs/azure-devops.md` before changing the pipeline token contract.
- Never put credentials in command arguments, request JSON, logs, or persisted
  files. Preserve redaction across Rust, Lua, and nested mise processes.
- A supplied `AZURE_BLOB_STORAGE_TOKEN` must never trigger another identity or
  interactive login, even when empty, invalid, or rejected by Azure.
- Do not add browser/device-code login or an Azure CLI authentication fallback.
- Preserve checksum verification, download limits, safe extraction, and
  no-overwrite defaults.
- Publishing uses the same authentication as downloads. Keep artifacts immutable,
  use ETag-conditional metadata updates, and publish the version index last.
- Support Linux and Windows x86-64. CI success does not verify Windows WAM/Entra
  SSO; use `docs/workplace-test.md` for workplace verification.
- When changing versions, update `Cargo.toml`, `Cargo.lock`, `metadata.lua`,
  the helper pin in `lib/azure.lua`, and README references together.
