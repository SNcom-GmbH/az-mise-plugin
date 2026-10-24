# Azure Blob Backend für mise

Privater mise-Backend-Prototyp: Tools aus einem Azure Blob Container installieren,
Versionen auflisten und Dateien lesen, herunterladen oder hochladen. **Keine Azure
CLI und kein manuelles `az login` erforderlich.**

**Stand:** Installation einschließlich Lua-Hooks, automatischer Plugin-Installation
und Login-Wiederverwendung lokal mit Mock-Storage getestet. Echter Azure-Zugriff und
Windows-WAM-SSO sind noch auf dem Arbeitsrechner zu prüfen. Das ist ein Startpunkt,
kein bereits produktiv verifiziertes Authentifizierungsprodukt.

## Im Projekt verwenden

Voraussetzungen: aktuelles mise mit Backend-Plugin-Unterstützung und Git-Zugriff auf
dieses private Repository. Lokal getestet mit mise **2026.10.3**. Unter Windows ist
PowerShell erforderlich; der Launcher erwartet mises standardmäßige Inline-Shell
`cmd /c`. Ein PowerShell-Terminal als äußere Shell ist dabei völlig in Ordnung.

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

`mise install` installiert das deklarierte Plugin automatisch. Das Plugin verwendet
mise intern, um **uv 0.13.4** bereitzustellen. uv richtet Python und die durch
`uv.lock` gesperrten SDK-Abhängigkeiten ein. Es ist kein separat zu installierendes
Azure-Tool nötig. Beim ersten Start braucht das Netzwerk Zugriff auf die
mise/uv/Python-Downloadquellen, PyPI, Microsoft Entra und Azure Storage. Vorhandene
Unternehmens-Proxys und Zertifikatsvorgaben müssen diese Zugriffe erlauben.

Der Plugin-Code wird pro Benutzer installiert; die Deklaration liegt im Projekt.
Für produktive Nutzung `#main` durch einen geprüften vollständigen Commit-SHA
ersetzen. Bei geänderter Plugin-Revision eine bereits installierte Kopie explizit
mit `mise plugins install --force azure-ac` erneuern.

## Anmeldung und Token-Erneuerung

- **Natives Windows:** offizielles `azure-identity-broker`, WAM und bevorzugt das
  bereits verbundene Windows-Konto. Zuerst werden gespeicherte Anmeldungen genutzt.
  Falls nötig, läuft die Broker-Anmeldung im interaktiven mise-Schritt und kann
  einen Microsoft-Kontodialog oder MFA zeigen. WSL ist hier **nicht** natives Windows.
- **Linux/macOS:** Browser-Login über `InteractiveBrowserCredential`. Optional
  `auth = "device"` für einen ausdrücklich gewählten Device-Code-Login.
- **Silent Refresh:** Azure Identity/MSAL verwaltet Tokens. Sie werden weder an Lua
  ausgegeben noch in `mise.toml` oder `mise.lock` geschrieben.
- **Persistenz:** verschlüsselter SDK-Token-Cache (DPAPI/Keychain/Secret Service).
  Ohne passenden OS-Schlüsselspeicher schlägt die Anmeldung fehl; es gibt keinen
  automatischen Klartext-Fallback. Ein separater Account-Auswahldatensatz liegt
  unter `$XDG_DATA_HOME/mise-azure-ac` bzw. `~/.local/share/mise-azure-ac`; dieser
  Datensatz enthält Account-Metadaten, aber keine Tokens.
- **CI:** `AZURE_CLIENT_ID`, `AZURE_TENANT_ID`, `AZURE_CLIENT_SECRET` für eine
  Service-Principal-Anmeldung oder `AZURE_FEDERATED_TOKEN_FILE` für Workload Identity.
  Managed Identity ist explizit über `auth = "managed"` wählbar. Automatische
  interaktive Logins sind bei gesetztem `CI` oder `AZURE_AC_NONINTERACTIVE=1` aus.

Das SDK nutzt vorläufig seine Microsoft-Entwicklungs-App. Eine eigene Entra-App ist
für Produktion empfohlen und kann durch Firmenrichtlinien erforderlich werden:

```toml
[tools."azure-ac:mein-tool"]
version = "1.2.3"
storage_url = "https://firma.blob.core.windows.net/tools"
tenant_id = "DEINE-TENANT-ID"
client_id = "DEINE-PUBLIC-CLIENT-APP-ID"
# auth = "broker"  # Unter Windows bereits die Vorgabe für auth = "auto".
```

Die App benötigt für WAM den Redirect
`ms-appx-web://Microsoft.AAD.BrokerPlugin/CLIENT-ID`; für Browser-Login eine passende
localhost-Redirect-Konfiguration, für Device Code die Public-Client-Freigabe.
Der Zugriff auf Azure Storage muss entsprechend freigegeben/consented sein.
Für diese interaktiven Public-Client-Anmeldungen wird **kein Client-Secret** benötigt.

**Wichtig:** Aus einer Blob-URL wird kein Tenant-/Subscription-Auswahlmenü wie in
Azure CLI abgeleitet. Normalerweise genügt der Organisations-Login; bei mehreren
Tenants, Gastkonten oder Richtlinien `tenant_id` explizit setzen. MFA und Conditional
Access lassen sich nicht umgehen und können erneute Interaktion verlangen.

Zum Lesen braucht das Konto **Storage Blob Data Reader**, zum Hochladen
**Storage Blob Data Contributor**, jeweils im richtigen Scope. Eine reine
Management-Plane-Rolle wie `Reader` genügt nicht.

## Erwartete Blob-Struktur

```text
Container tools
└── mein-tool/
    ├── index.json
    └── 1.2.3/
        ├── release.json
        ├── mein-tool-windows-amd64.zip
        └── mein-tool-linux-amd64.tar.gz
```

`mein-tool/index.json` führt Versionen **älteste zuerst** auf:

```json
{
	"versions": ["1.2.2", "1.2.3"]
}
```

Nur tatsächlich veröffentlichte Versionen aufführen. Optional akzeptiert der
Leser auch Objekte mit `version` und RFC-3339-`created_at`.

`mein-tool/1.2.3/release.json`:

```json
{
	"platforms": {
		"windows-amd64": {
			"file": "mein-tool-windows-amd64.zip",
			"sha256": "HIER-DEN-ECHTEN-64-STELLIGEN-SHA256-EINTRAGEN"
		},
		"linux-amd64": {
			"file": "mein-tool-linux-amd64.tar.gz",
			"sha256": "HIER-DEN-ECHTEN-64-STELLIGEN-SHA256-EINTRAGEN"
		}
	}
}
```

Weitere mögliche Schlüssel sind z. B. `darwin-arm64` und `windows-arm64`, sofern
die jeweilige Plattform und die SDK-Abhängigkeiten unterstützt werden. Linux-musl
wird derzeit nicht von GNU/Linux unterschieden: dafür getrennte Toolnamen nutzen.

Archive müssen `bin/` direkt an der Wurzel enthalten, beispielsweise
`bin/mein-tool.exe` unter Windows. Unterstützt sind `.zip`, `.tar.gz`, `.tar.xz`
und `.tar.bz2`. Prüfsummen werden **vor** dem Entpacken geprüft. Absolute Pfade,
Traversal, Links und Spezialdateien werden abgewiesen. Archive mit Symlinks müssen
für diesen ersten Prototyp mit regulären Dateien neu gepackt werden.

Prüfsumme unter PowerShell:

```powershell
(Get-FileHash .\mein-tool-windows-amd64.zip -Algorithm SHA256).Hash.ToLowerInvariant()
```

Prüfsumme und Archiv stammen aus demselben Storage: Das schützt gegen Korruption,
nicht gegen einen kompromittierten Herausgeber. Backend-Plugins schreiben aktuell
keine Artefakt-URLs oder Prüfsummen in `mise.lock` und prüfen keine Attestierungen.
Veröffentlichte Versionen deshalb organisatorisch unveränderlich halten.

## Dateien verwalten

Im geklonten Plugin-Repository, mit derselben SDK-Anmeldung:

```sh
mise run blob -- --storage-url https://firma.blob.core.windows.net/tools login
mise run blob -- --storage-url https://firma.blob.core.windows.net/tools list --prefix mein-tool/
mise run blob -- --storage-url https://firma.blob.core.windows.net/tools read mein-tool/index.json
mise run blob -- --storage-url https://firma.blob.core.windows.net/tools download notes.txt ./notes.txt
mise run blob -- --storage-url https://firma.blob.core.windows.net/tools upload notes.txt ./notes.txt
```

`--tenant-id`, `--client-id` und `--auth` ebenfalls **vor** dem Unterkommando angeben.
`read` gibt UTF-8-Text als JSON-Feld `text` aus. Downloads überschreiben keine lokalen
Dateien. Uploads überschreiben keine Blobs, außer mit dem expliziten Flag
`upload BLOB DATEI --overwrite`.

Für eine erste Veröffentlichung: Archiv hochladen, danach `release.json`, zuletzt
`index.json`. Für parallele Veröffentlichungen gibt es noch keinen atomaren
Publisher; ein blind überschriebenes `index.json` könnte andere Änderungen verlieren.

Aktuell sind nur HTTPS-URLs der öffentlichen Azure-Cloud mit einem Containerpfad
erlaubt. Keine SAS-URLs, Storage Keys, beliebigen Hosts oder Sovereign Clouds.

## Entwickeln und prüfen

```sh
mise trust
mise install
mise run check
```

`.editorconfig` und Ruff konfigurieren Tabs mit Breite 4 und Double Quotes.
Die Tests verwenden ausschließlich temporären Mock-Storage und isolierte
mise-Datenverzeichnisse; keine echten Azure-Zugangsdaten erforderlich.
Der Integrationstest läuft durch die echte mise/Lua/uv/Python-Kette und benötigt
bereits installiertes uv, wie es `mise install` für dieses Repository bereitstellt.

Struktur:

```text
metadata.lua              Plugin-Metadaten
hooks/                    mise-Backend-Hooks
lib/azure.lua             Launcher und interaktiver Login-Schritt
scripts/launch.ps1        Windows-Launcher
src/azure_ac.py           Azure Identity, Blob-Zugriff, SHA-256 und Installation
tests/                    Offline-Tests und ausschließlich testseitige Mock-Injektion
examples/mise.toml        Beispiel für ein konsumierendes Projekt
docs/workplace-test.md    Checkliste für den Arbeitsrechner
```
