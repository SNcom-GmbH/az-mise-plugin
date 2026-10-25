# Azure Blob Backend für mise

Private Tools aus Azure Blob Storage installieren. Minimales Lua, Rust-Helfer mit
Rustls. Nur **Windows und Linux x86-64**; keine Azure CLI erforderlich.

## Verwendung

```toml
[plugins]
azure-ac = "https://github.com/DominicWrege/az-mise-plugin.git#main"

[tools]
"azure-ac:mein-tool" = { version = "1.2.3", storage_url = "https://firma.blob.core.windows.net/tools" }
```

```sh
mise trust
mise install
mise ls-remote azure-ac:mein-tool
mise exec -- mein-tool --version
```

Der Helfer wird automatisch aus GitHub-Release `v0.2.0` geladen. Das Release muss
veröffentlicht sein; private Downloads benötigen `GITHUB_TOKEN` oder `GH_TOKEN`.
Für Produktion den Plugin-Ref auf einen vollständigen Commit-SHA pinnen.

## Anmeldung

- **Windows:** Firmenkonto über WAM, möglichst still; kein Browser-/Device-Code-Login.
  Optional `tenant_id` und `client_id` als Tool-Optionen setzen.
- **Azure Pipelines/Linux:** Service Principal mit `AZURE_TENANT_ID`,
  `AZURE_CLIENT_ID` und `AZURE_CLIENT_SECRET`. Explizit `auth = "environment"`
  setzen oder die automatische Erkennung verwenden. Secrets nur aus dem
  Pipeline-Secret-Store als Umgebungsvariablen übergeben, niemals einchecken.
- Lesen: **Storage Blob Data Reader**. Hochladen: **Storage Blob Data Contributor**.

Echtes Windows-/Entra-SSO ist noch auf dem Arbeitsrechner zu prüfen:
[Testcheckliste](docs/workplace-test.md).

## Blob-Layout

- `mein-tool/index.json`: `{"versions": ["1.2.3"]}`
- `mein-tool/1.2.3/release.json`: `{"platforms": {"linux-amd64": {"file": "tool.tar.gz", "sha256": "<64-stelliger SHA-256>"}}}`
- Archive enthalten direkt `bin/`. Plattformen: `linux-amd64`, `windows-amd64`.
- Formate: ZIP, tar.gz, tar.xz, tar.bz2. Archiv und Manifest zuerst, Index zuletzt hochladen.

## Dateien verwalten

Mit dem Release-Binary; im Checkout alternativ `mise run blob --` statt `azure-ac`:

```sh
azure-ac --storage-url https://firma.blob.core.windows.net/tools list
azure-ac --storage-url https://firma.blob.core.windows.net/tools read notes.txt
azure-ac --storage-url https://firma.blob.core.windows.net/tools download notes.txt ./notes.txt
azure-ac --storage-url https://firma.blob.core.windows.net/tools upload notes.txt ./notes.txt
```

Uploads überschreiben nur mit `--overwrite`; Downloads überschreiben nie.

## Entwickeln

`mise install`, `mise run build`, `mise run check`.
Vor dem ersten Release `AZURE_AC_HELPER` auf das lokal gebaute Binary oder ein
entpacktes CI-Artefakt setzen. CI baut statisches Linux-musl und Windows mit statischer
C-Runtime. Ein `v*`-Tag veröffentlicht die Binaries und `SHA256SUMS` als GitHub-Release.

## Lizenz

MIT
