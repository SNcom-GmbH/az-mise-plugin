# Erster Test: Windows-Firmenkonto und Linux-CI

## App-Registrierung (WAM)

Für eine eigene Public-Client-App-Registrierung:

- Delegierte Azure-Storage-Berechtigung `user_impersonation` erteilen.
- Broker-Redirect-URI `ms-appx-web://Microsoft.AAD.BrokerPlugin/<client_id>`
  unter „Mobile and desktop applications“ eintragen.
- Public-Client-Flows und Tenant-/Conditional-Access-Richtlinien freigeben.
- Der Standard-Client ist die Azure-CLI-App; Firmenrichtlinien können ihn blockieren.

## Windows

1. Plugin-Repository klonen und GitHub-Zugriff sicherstellen.
2. Das Windows-ZIP aus einem erfolgreichen GitHub-Actions-Lauf oder Release
   herunterladen und entpacken. `AZURE_AC_HELPER` auf die enthaltene `azure-ac.exe`
   setzen; dafür ist kein Rust-Compiler erforderlich. Alternativ im Checkout
   `mise trust`, `mise install`, `mise run build`, `mise run check` ausführen und
   `AZURE_AC_HELPER` auf `target/x86_64-pc-windows-msvc/debug/azure-ac.exe` setzen.
3. `examples/mise.toml` in ein separates Testprojekt kopieren und Account/Container
   ersetzen. Bei mehreren Tenants `tenant_id` explizit ergänzen.
4. Ein harmloses Tool mit `bin/NAME.exe` oder `bin/NAME.cmd`, korrektem SHA-256,
   `release.json` und `index.json` nach README-Schema bereitstellen. Optional
   `size` im Manifest und `sha256` als Tool-Option pinnen; ein falscher Pin muss
   vor dem Download fehlschlagen.
5. Dem Testkonto `Storage Blob Data Reader` geben; VPN, DNS und Private-Endpoint-
   Netzwerkregeln gegebenenfalls aktivieren. Bei AppLocker/SmartScreen/ASR das
   unsignierte Binary freigeben oder signieren.
6. `mise trust`, dann `mise install`: vorhandenes Windows-Firmenkonto soll möglichst
   still funktionieren; falls nötig erscheint der native WAM-Dialog. Kein separater
   Browser- oder Device-Code-Login und kein vorheriges `az login`.
7. `mise ls-remote azure-ac:NAME` und `mise exec -- NAME --version` prüfen.
8. In einem neuen Terminal `mise cache clear azure-ac:NAME` ausführen, dann erneut
   Versionen abfragen. So wird der Login nicht durch mises Versionscache verdeckt.
9. Nach Ablauf eines Access Tokens wiederholen: stille WAM-Erneuerung prüfen.
10. Mit `AZURE_AC_NONINTERACTIVE=1` darf kein Dialog starten; ohne nutzbare stille
    Anmeldung muss der Vorgang fehlschlagen.
11. Einen falschen `sha256`-Pin bzw. eine zu kleine `size` testen: Die Installation
    muss vor Extraktion abbrechen und das Zielverzeichnis unverändert lassen.
12. Bei Bedarf `MISE_DEBUG=1` verwenden; Fehlermeldungen enthalten HTTP-Status und
    gefilterten Azure-Fehlercode, aber keine Response-Bodies.

**Noch nicht verifiziert:** direkte WinRT-WAM-Aufrufe im Unternehmens-Tenant,
Default-Kontoauswahl, Tenant-Wechsel, Consent und Conditional Access. Der erfolgreiche
Windows-Build bzw. Mocktest ersetzt diesen Test nicht. Der native Adapter ist kein
MSAL-Nachbau und übernimmt nicht Azure CLIs Subscription-/Tenant-Auswahllogik.

Bei Problemen: App-Registrierung und Broker-Verfügbarkeit prüfen. Der vorläufige
Entwicklungs-Client kann durch Firmenrichtlinien blockiert sein; eine eigene
Public-Client-App kann erforderlich werden. Richtlinien nicht mit einem automatischen
Device-Code-Fallback umgehen. Details: [security.md](security.md).

## Linux/CI

Für InstallBuilder@2 mit der ARM-Service-Connection `az-development` gilt der
[Azure-DevOps-Vertrag und die Deployment-Reihenfolge](azure-devops.md). Im Zieltenant
auch Workload Identity Federation, Token-Ablauf und HTTP 401/403 ohne Identitätswechsel
prüfen. Der Task übergibt das Storage-Token ausschließlich als
`AZURE_AC_STORAGE_TOKEN` an `mise install --locked`; der Helfer kann es nicht erneuern.

1. Statischen musl-Helfer in einem Agent/Container mit CA-Zertifikaten verwenden.
2. Entweder `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, `AZURE_FEDERATED_TOKEN_FILE` setzen
   oder einen Service Principal mit `AZURE_CLIENT_SECRET` aus dem CI-Secret-Store.
3. Für Workload Identity muss die CI-Plattform die gültige Assertion-Datei liefern;
   das Plugin erzeugt keine Föderationskonfiguration oder CI-OIDC-Assertion.
4. `CI=1` setzen, `mise install` und `mise ls-remote azure-ac:NAME` ausführen.
5. Token-Erneuerung bei längerem Job prüfen, einschließlich einer erforderlichen
   Rotation der Assertion-Datei durch den CI-Provider.
6. Mit fehlenden oder falschen Credentials muss der Job ohne Dialog, Browser,
   Azure-CLI-Aufruf oder Wechsel auf eine Personenidentität fehlschlagen.
7. Managed Identity nur explizit über `auth = "managed"` auf passendem Azure-Agent;
   bei benutzerzugewiesener Identität zusätzlich `managed_identity_client_id` setzen.

## Gemeinsame Fehlerfälle

- Falsche Prüfsumme, falscher `sha256`-Pin, überschrittene `size`, fehlende Plattform
  oder fehlendes `bin/`: keine Installation.
- Vorhandene nichtleere Installationsverzeichnisse: kein Überschreiben.
- HTTP 403 nach Login: Datenebenenrechte, Tenant und Storage-Netzwerkregeln prüfen.
- HTTP 404: Blob-/Container-Pfad und `index.json`/`release.json` prüfen; das ist
  kein Berechtigungsfehler.
- HTTP 409 beim Upload: Blob existiert bereits; `--overwrite` verwenden.
- Für private GitHub-Release-Assets einen passenden GitHub-Token bereitstellen;
  SSH-Zugriff auf das Plugin allein genügt dafür nicht.

Rückmeldung: mise-/Windows-Version bzw. Agent-Image, Befehl, Fehlerklasse/HTTP-Status,
anonymisierte Konfiguration und ob das Microsoft-Kontofenster erschien.
**Keine Tokens, Secrets, SAS-URLs oder vollständigen Credential-Logs weitergeben.**
