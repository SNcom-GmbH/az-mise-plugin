# Azure Blob Backend für mise

Tools aus privaten Azure Blob Containern installieren: **minimales Lua für mise,
alles Weitere in Rust**. Keine Azure CLI, kein Python/uv, kein Browser-Login.

## Plattformen und Anmeldung

| Plattform | Anmeldung | Helfer |
| --- | --- | --- |
| Windows x64 | natives Entra-/WAM-SSO, optional CI-Identitäten | MSVC mit statisch gelinkter C-Runtime |
| Linux x64 | CI: Workload Identity, Service Principal, Managed Identity | statischer musl-Build |
| Linux ARM64 | dieselben CI-Anmeldeverfahren | statischer musl-Build |

**Linux ist bewusst CI-only.** Es gibt weder Browser- noch Device-Code-Login und
keinen automatischen Wechsel auf andere Identitäten, wenn die konfigurierte
CI-Anmeldung fehlschlägt. Unter Windows wird bei Bedarf der native WAM-Kontodialog
verwendet; MFA und Firmenrichtlinien bleiben wirksam.

**Prototyp:** Echte Azure-Zugriffe und das Verhalten der nativen WAM-Anmeldung mit
einem Firmenkonto müssen auf dem Arbeitsrechner getestet werden. Offline-Tests
sind keine Bestätigung für Tenant-Policies, Consent oder tatsächliches Windows-SSO.

## Im Projekt verwenden

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

mise installiert das deklarierte Plugin. Der Lua-Hook verwendet mises eingebautes
GitHub-Backend, um den **fertig gebauten Rust-Helfer 0.2.0** für die exakte Plattform
zu installieren und mit dessen Prüfsummenprüfung auszuführen. Nutzer brauchen
keinen Rust-Compiler. Voraussetzungen sind Git, mise mit Backend-Plugin-Support
(lokaler Referenzstand: **2026.10.3**) und Zugriff auf das private Repository.
Der Windows-Launcher verwendet mises Standard-Inline-Shell `cmd /c`; ein äußeres
PowerShell-Terminal ist kein Problem. Es werden keine PowerShell-Skripte benötigt.

**Bootstrap:** Dafür muss das GitHub-Release `v0.2.0` mit den unten beschriebenen
Assets veröffentlicht sein. Ein reiner Git-Checkout ersetzt kein Binary-Release.
Bis zur ersten Veröffentlichung den Helfer aus einem erfolgreichen CI-Artefakt
entpacken oder lokal bauen und `AZURE_AC_HELPER` auf das Binary setzen.
Für private Release-Downloads zusätzlich einen passenden `GITHUB_TOKEN` oder
`GH_TOKEN` bereitstellen. Git-/SSH-Zugriff auf das Plugin allein berechtigt nicht
automatisch zum Download privater GitHub-Release-Assets. GitHub-Zugang und
Azure-Anmeldung sind unabhängig voneinander; keine Tokens ins Projekt einchecken.

Für Produktion `#main` durch einen vollständigen geprüften Commit-SHA ersetzen.
Eine schon installierte Plugin-Kopie bei geändertem Ref explizit erneuern:
`mise plugins install --force azure-ac`.

## Azure-Bibliotheken statt eigener REST-Implementierung

- Microsofts **`azure_identity`**: Client Secret, Workload Identity und Managed Identity.
- Microsofts **`azure_storage_blob`**: Blob-Operationen, Paging, Downloads und Uploads.
- **Rustls mit Ring**, keine OpenSSL-/Native-TLS-Abhängigkeit für HTTP/TLS.
- Microsofts **`windows`-Crate**: native WinRT-WAM-API unter Windows. Das offizielle
  Rust-Identity-SDK hat derzeit keinen entsprechenden interaktiven Credential-Typ;
  unser schmaler Adapter implementiert dessen `TokenCredential`-Schnittstelle.

Azure CLI dient als Referenz für Entra-/Broker-Verhalten, ist aber weder eine
Laufzeitabhängigkeit noch eine Bibliothek, die wir einbetten. Unsere native
WAM-Anbindung ist kein vollständiger Nachbau von MSAL/Azure CLI.

## Windows: Tenant und Firmenkonto

```toml
[tools."azure-ac:mein-tool"]
version = "1.2.3"
storage_url = "https://firma.blob.core.windows.net/tools"
tenant_id = "DEINE-TENANT-ID"
# client_id = "DEINE-PUBLIC-CLIENT-APP-ID"
# auth = "broker" # Unter Windows bereits Vorgabe für auto ohne CI-Credentials.
```

Zuerst fragt der Helfer WAM still nach einem Storage-Token. Falls Benutzerinteraktion
erforderlich ist, fordert Lua einen sichtbaren Login-Schritt an. WAM verwaltet
Anmeldung, Token-Persistenz und stille Token-Erneuerung; unser Helfer schreibt keine
Tokens auf die Platte und gibt sie niemals an Lua oder auf stdout aus. Nur die
gewählte OS-Account-ID wird unter `%LOCALAPPDATA%/mise-azure-ac` pro Tenant und
Client-ID gespeichert, damit weitere Aufrufe nicht still auf ein anderes Konto wechseln.

Vorläufiger Entwicklungs-Client ist `04b07795-8ddb-461a-bbee-02f9e1bf7b46`, wie beim
bisherigen Prototyp. Das garantiert keine Freigabe durch eine Organisation. Für
Produktion ist eine eigene freigegebene Public-Client-App empfohlen. Die konkrete
App-/Redirect-Konfiguration muss zur nativen WAM-API passen; die MSAL-Anleitung
allein beweist nicht, dass diese direkte WinRT-Anbindung damit funktioniert.

Eine Storage-URL liefert kein Subscription-/Tenant-Auswahlmenü. Bei Gastkonten,
mehreren Tenants oder Richtlinien `tenant_id` explizit setzen. WSL gilt als Linux
und verwendet diesen Windows-Broker nicht. `AZURE_AC_NONINTERACTIVE=1` verbietet
einen interaktiven WAM-Dialog; eine bestehende stille Anmeldung kann weiterarbeiten.

## Linux und CI

**Workload Identity** (bevorzugt, kein langlebiges Secret):

```text
AZURE_TENANT_ID=...
AZURE_CLIENT_ID=...
AZURE_FEDERATED_TOKEN_FILE=/pfad/zur/gueltigen-oidc-assertion
```

Die CI-Plattform muss die gültige Assertion-Datei bereitstellen und ihre Rotation
sicherstellen. Das Plugin richtet keine Entra-Föderation ein und erzeugt nicht von
selbst einen GitHub-/Azure-Pipelines-OIDC-Token.

Alternativ **Service Principal**:

```text
AZURE_TENANT_ID=...
AZURE_CLIENT_ID=...
AZURE_CLIENT_SECRET=... # Nur aus dem CI-Secret-Store.
```

`auth = "auto"` erkennt diese Umgebungsvariablen; bei beiden Verfahren gleichzeitig
hat das Client-Secret Vorrang. Mit `auth = "environment"` oder `auth = "workload"`
lässt sich das Verfahren explizit festlegen. **Managed Identity** ist ausdrücklich
über `auth = "managed"` wählbar, optional mit `managed_identity_client_id`.

Das SDK übernimmt die Access-Token-Erneuerung im laufenden Prozess. Zwischen
Helfer-Aufrufen wird die CI-Identität erneut verwendet, ohne persistenten lokalen
Token-Cache. Bei gesetztem `CI` startet kein interaktiver Login und `auto` ohne
CI-Credentials verwendet nicht versehentlich ein Windows-Privatkonto.

Lesen benötigt **Storage Blob Data Reader**, Schreiben **Storage Blob Data
Contributor**. Reine Management-Plane-Rechte wie `Reader` reichen nicht.

## Blob-Layout und Installation

Das Format aus dem Python-Prototyp bleibt erhalten:

```text
Container tools
└── mein-tool/
    ├── index.json
    └── 1.2.3/
        ├── release.json
        ├── mein-tool-windows-amd64.zip
        └── mein-tool-linux-amd64.tar.gz
```

`index.json` (älteste Version zuerst, nur tatsächlich veröffentlichte Versionen):

```json
{
	"versions": ["1.2.2", "1.2.3"]
}
```

Optional werden Einträge mit `version` und RFC-3339-`created_at` durchgereicht.
`release.json`:

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

Archive enthalten direkt `bin/`, unter Windows z. B. `bin/mein-tool.exe`.
Unterstützt: ZIP, tar.gz, tar.xz, tar.bz2. Der Helfer prüft SHA-256, entpackt in ein
temporäres Staging-Verzeichnis und finalisiert die Installation atomar. Traversal,
Links, Spezialdateien, doppelte Archivpfade und problematische Windows-Dateinamen
werden abgewiesen. Vorhandene nichtleere Installationsverzeichnisse bleiben erhalten.
Grenzen: 100.000 Archiveinträge, insgesamt 4 GiB entpackte Daten, 16 MiB für JSON/Text.

Die Prüfsumme liegt im selben Storage und schützt nicht gegen einen kompromittierten
Herausgeber. Backend-Plugin-Artefakte erhalten keine URL-/Prüfsummen-Einträge in
`mise.lock`; veröffentlichte Tool-Versionen deshalb unveränderlich halten.
Linux-Tool-Artefakte unterscheiden im Manifest derzeit nicht zwischen glibc/musl;
das ist unabhängig vom statischen Build des **Helfers**.

## Dateien verwalten

Mit installiertem Release-Binary `azure-ac` oder im Entwickler-Checkout über
`mise run blob --` statt `azure-ac`:

```sh
azure-ac --storage-url https://firma.blob.core.windows.net/tools login
azure-ac --storage-url https://firma.blob.core.windows.net/tools list --prefix mein-tool/
azure-ac --storage-url https://firma.blob.core.windows.net/tools read mein-tool/index.json
azure-ac --storage-url https://firma.blob.core.windows.net/tools download notes.txt ./notes.txt
azure-ac --storage-url https://firma.blob.core.windows.net/tools upload notes.txt ./notes.txt
```

`--tenant-id`, `--client-id` und `--auth` stehen vor dem Unterkommando. Downloads
überschreiben keine lokalen Dateien. Uploads überschreiben nur mit ausdrücklich
gesetztem `upload BLOB DATEI --overwrite`. Uploads und Downloads streamen über das
Azure-SDK, statt ganze Archive in den RAM zu laden. `read` liefert UTF-8 als JSON-Feld `text`.

Veröffentlichen: Archiv, dann `release.json`, zuletzt `index.json` hochladen.
Ein transaktionssicherer paralleler Publisher ist noch nicht enthalten.
Erlaubt sind nur öffentliche Azure-Cloud-HTTPS-Endpunkte mit Containerpfad, keine
SAS-URLs, Storage Keys, beliebigen Hosts oder Sovereign Clouds.

## Entwickeln und prüfen

```sh
mise trust
mise install
mise run build
mise run check
```

Nur Entwickler benötigen Rust (mise pinnt **1.99.1**). `.editorconfig` und rustfmt
verwenden echte Tabs, Breite 4, und Rust-Strings mit Double Quotes. Ein temporärer
Mock-Helfer ist ein eigenes Rust-Example, kein Auth-Bypass im Produktionsbinary.
Die Tests prüfen auch die echte mise → Lua → Rust → Mock-Storage-Kette.

Lokalen Helfer vor dem ersten Release verwenden:

```sh
export AZURE_AC_HELPER="/ABSOLUTER/PFAD/az-mise-plugin/target/debug/azure-ac"
```

Unter Windows nach `mise run build`:

```powershell
$env:AZURE_AC_HELPER = "C:\ABSOLUTER\PFAD\az-mise-plugin\target\x86_64-pc-windows-msvc\debug\azure-ac.exe"
```

## Fertige Binaries und statisches Linken

`.github/workflows/build.yml` testet und baut Linux x64/ARM64 (musl) und Windows x64
(MSVC). Auf Branch-Pushes gibt es herunterladbare CI-Artefakte; ein ausdrücklich
gepushter `v*`-Tag veröffentlicht anschließend ein GitHub-Release mit:

```text
azure-ac-linux-amd64-musl.tar.gz
azure-ac-linux-arm64-musl.tar.gz
azure-ac-windows-amd64.zip
SHA256SUMS
```

Linux-Binaries werden auf fehlende ELF-Interpreter und dynamische Abhängigkeiten
geprüft. Windows verwendet `+crt-static` mit explizitem Target; **Windows-System-DLLs
und WAM bleiben notwendige Betriebssystemkomponenten**. Unser Code benötigt keine
zusätzliche MSAL-DLL. Rustls verwendet den System-Zertifikatsspeicher; auf Linux
müssen passende CA-Zertifikate vorhanden sein, auch im schlanken CI-Container.

Siehe [Testcheckliste](docs/workplace-test.md) für den Arbeitsrechner und CI-Test.
