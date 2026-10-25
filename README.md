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

Optional pinnt `sha256 = "<64 Hex-Zeichen>"` auf dem Tool-Eintrag das Artefakt
unabhängig von `release.json`; bei Abweichung wird nichts installiert.

## Anmeldung

- **Windows:** Firmenkonto über WAM, möglichst still; kein Browser-/Device-Code-Login.
  Optional `tenant_id` und `client_id` als Tool-Optionen setzen.
- **Azure Pipelines/Linux:** Service Principal mit `AZURE_TENANT_ID`,
  `AZURE_CLIENT_ID` und `AZURE_CLIENT_SECRET`. Explizit `auth = "environment"`
  setzen oder die automatische Erkennung verwenden. `tenant_id`/`client_id` als
  Tool-Optionen haben Vorrang vor den `AZURE_*`-Variablen. Secrets nur aus dem
  Pipeline-Secret-Store als Umgebungsvariablen übergeben, niemals einchecken.
- **Managed Identity:** `auth = "managed"`, optional
  `managed_identity_client_id = "..."` für eine benutzerzugewiesene Identität.
- Lesen: **Storage Blob Data Reader**. Hochladen: **Storage Blob Data Contributor**.

Echtes Windows-/Entra-SSO ist noch auf dem Arbeitsrechner zu prüfen:
[Testcheckliste](docs/workplace-test.md). Betriebs- und Sicherheitshinweise:
[docs/security.md](docs/security.md).

## Blob-Layout

- `mein-tool/index.json`: `{"versions": ["1.2.3"]}`
- `mein-tool/1.2.3/release.json`: `{"platforms": {"linux-amd64": {"file": "tool.tar.gz", "sha256": "<64-stelliger SHA-256>", "size": 12345678}}}`
- `size` ist optional; es begrenzt den Download zusätzlich auf die angegebene Byte-Zahl.
  Ohne `size` gilt ein Maximum von 4 GiB pro Artefakt.
- Archive enthalten direkt `bin/`. Plattformen: `linux-amd64`, `windows-amd64`.
- Formate: ZIP, tar.gz, tar.xz, tar.bz2. Archiv und Manifest zuerst, Index zuletzt hochladen.

## Dateien verwalten

Mit dem Release-Binary; im Checkout alternativ `mise run blob --` statt `azure-ac`:

```sh
azure-ac --storage-url https://firma.blob.core.windows.net/tools list
azure-ac --storage-url https://firma.blob.core.windows.net/tools list --limit 100
azure-ac --storage-url https://firma.blob.core.windows.net/tools read notes.txt
azure-ac --storage-url https://firma.blob.core.windows.net/tools download notes.txt ./notes.txt
azure-ac --storage-url https://firma.blob.core.windows.net/tools upload notes.txt ./notes.txt
```

Uploads überschreiben nur mit `--overwrite`; Downloads überschreiben nie und sind auf
4 GiB pro Datei begrenzt. `list` liefert höchstens 100000 Einträge; `--limit` senkt
diese Grenze.

## Entwickeln

`mise install`, `mise run build`, `mise run check`.
Vor dem ersten Release `AZURE_AC_HELPER` auf das lokal gebaute Binary oder ein
entpacktes CI-Artefakt setzen. CI baut statisches Linux-musl und Windows mit statischer
C-Runtime. Ein `v*`-Tag veröffentlicht die Binaries und `SHA256SUMS` als GitHub-Release.

## Lizenz

MIT
