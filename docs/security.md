# Sicherheit und Betrieb

## Identitäten und Optionen

- **Windows/WAM:** `tenant_id` und `client_id` sind Tool-Optionen. Sie gelten für
  den WAM-Broker und werden nicht als Secret gespeichert. Der Standard-Client ist
  die öffentliche Azure-CLI-App; Unternehmensrichtlinien können ihn blockieren.
- **Environment/Workload (Linux/CI):** `tenant_id`/`client_id` aus den Tool-Optionen
  haben Vorrang vor `AZURE_TENANT_ID`/`AZURE_CLIENT_ID`. Das Client-Secret bzw. die
  föderierte Assertion kommt ausschließlich aus der Umgebung:
  `AZURE_CLIENT_SECRET` bzw. `AZURE_FEDERATED_TOKEN_FILE`.
- **Managed Identity:** nur explizit mit `auth = "managed"`. Eine
  benutzerzugewiesene Identität wird über `managed_identity_client_id` oder
  `--managed-identity-client-id` ausgewählt; ohne Angabe wird die systemzugewiesene
  Identität versucht.
- **Kein Browser-/Device-Code-Login, kein Azure-CLI-Fallback.** Auf Linux werden
  fehlende CI-Identitäten ohne interaktiven Fallback abgelehnt.

## WAM und App-Registrierung

Für eine eigene Public-Client-App-Registrierung sind typischerweise erforderlich:

- Plattform **Mobile and desktop applications** bzw. Public-Client-Flows.
- Delegierte Berechtigung für Azure Storage (`user_impersonation`).
- Broker-Redirect-URI `ms-appx-web://Microsoft.AAD.BrokerPlugin/<client_id>`.
- Freigabe durch Tenant-Richtlinien, Conditional Access und ggf. MFA.

Der WAM-Adapter speichert nur die ausgewählte Windows-Konto-ID unter
`%LOCALAPPDATA%\mise-azure-ac\<hash>.json`; niemals Access- oder Refresh-Tokens.
Das Löschen der Datei entfernt nur die gespeicherte Auswahl und erzwingt keinen
Kontowechsel; dieser folgt den Windows-/Broker-Richtlinien.
`AZURE_AC_NONINTERACTIVE=1` verhindert jeden WAM-Dialog.

## Artefakt-Vertrauen

Der Helfer prüft die SHA-256 aus `tool/version/release.json` vor der Extraktion.
Manifest und Artefakt liegen jedoch im selben Container. Wer Schreibzugriff auf
den Container hat, kann beide ersetzen.

- `mise.lock` speichert für Backend-Plugins nur die Version, keine URL und keinen
  Checksum. Ein Lockfile garantiert daher keine unveränderlichen Artefakt-Bytes.
- Die optionale Tool-Option `sha256` pinnt den Artefakt-Hash unabhängig vom
  Manifest. Weicht der Pin von `release.json` ab, wird nichts heruntergeladen oder
  installiert.
- Downloads sind auf 4 GiB pro Datei/Artefakt begrenzt; `size` im Manifest kann sie
  zusätzlich verkleinern. Die Grenze schützt vor versehentlich übergroßen Blobs,
  ersetzt aber keine Zugriffskontrolle.
- Für stärkeren Schutz: Schreibrechte auf den Container minimieren,
  Blob-Immutability/Versionierung aktivieren, Releases nur über eine kontrollierte
  Pipeline veröffentlichen und bei Bedarf signierte Manifeste verwenden.
- Der Helfer selbst wird über den mise-GitHub-Backend geladen; mise prüft
  Release-Checksums bzw. GitHub-Attestations, sofern vorhanden. Plugin-Ref und
  Release-Tag sollten gepinnt werden.

## Windows-Bereitstellung

- Das Release-Binary ist derzeit nicht Authenticode-signiert. AppLocker,
  SmartScreen, Defender ASR oder Application Control können es blockieren.
  Erforderlich sind ggf. Signierung oder eine freigabebasierte Allowlist
  (Hash/Publisher).
- WAM benötigt eine interaktive Windows-Desktop-Sitzung. In Diensten, SSH-Sitzungen
  oder ohne Desktop schlägt der interaktive Login kontrolliert fehl.
- WAM, Broker und Conditional Access müssen auf dem Zielgerät verfügbar sein; ein
  erfolgreicher CI-Build ersetzt keinen Arbeitsplatz-Test.

## Cloud, Netzwerk und Kryptografie

- Unterstützt wird **Azure Public Cloud**: `*.blob.core.windows.net` und
  `https://storage.azure.com/.default`. Sovereign Clouds, Azure Stack und Azurite
  werden von der URL-/Scope-Validierung abgelehnt.
- TLS erfolgt über Rustls mit dem Plattform-Zertifikatsspeicher; in Containern
  müssen CA-Zertifikate vorhanden sein. `system-proxy` nutzt System-/Umgebungsproxys;
  PAC, NTLM oder Kerberos-Proxy-Authentifizierung sind nicht garantiert.
- Statische musl-Binaries verwenden den musl-Resolver. Corporate DNS,
  Suchdomains und Split-DNS im CI-Netz müssen getestet werden.
- Rustls/ring ist nicht FIPS-140-validiert. Falls FIPS erforderlich ist, muss ein
  entsprechender Kryptografie-Provider bzw. ein separater Build verwendet werden.

## Secrets und Logs

- Secrets werden nur aus Umgebungsvariablen gelesen und nicht in Tool-Optionen,
  Kommandozeilen oder Logs geschrieben.
- Fehlermeldungen enthalten HTTP-Status und einen gefilterten Azure-Fehlercode,
  aber keine Response-Bodies. Ein gesondertes `--debug` gibt es nicht;
  `RUST_BACKTRACE=1` bleibt eine bewusste Debug-Option.
