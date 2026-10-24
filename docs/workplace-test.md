# Erster Test auf dem Arbeitsrechner

## Vorbereiten

1. Repository klonen; Zugriff auf das private GitHub-Repo sicherstellen.
2. Aktuelles mise installieren. Lokaler Referenzstand: 2026.10.3.
3. Zunächst im Plugin-Repo `mise trust`, `mise install`, `mise run check` ausführen.
   Auf Windows werden dabei Launcher und Installation gegen Mock-Storage geprüft,
   aber weder WAM noch echtes Azure verwendet.
4. Für das echte Testprojekt `examples/mise.toml` kopieren und die URL ersetzen.
5. Ein harmloses Test-Tool als ZIP mit `bin/NAME.exe` oder `bin/NAME.cmd`,
   `release.json` und `index.json` nach dem README-Schema bereitstellen.
6. Dem Testkonto `Storage Blob Data Reader` geben. Firmen-VPN, Firewall und
   Private-Endpoint-DNS bei Bedarf aktivieren.

## Prüfen

- `mise trust`, dann `mise install`: Plugin wird automatisch aus Git installiert;
  uv/Python/SDK werden ohne separate Azure-CLI-Installation bereitgestellt.
- Ohne vorheriges `az login`: Windows-Konto wird bevorzugt. Falls nötig erscheint
  der Microsoft-Anmeldedialog. MFA/Consent normal bestätigen.
- `mise ls-remote azure-ac:NAME`: erwartete Versionen, nicht nur die installierte.
- `mise exec -- NAME --version`: installiertes Tool läuft.
- Neues Terminal: erneute Versionsabfrage ohne unnötige Anmeldung.
  Bei gecachter Versionsliste vorher `mise cache clear azure-ac:NAME` ausführen.
- Später, nach Ablauf eines Access Tokens, erneut abfragen: Silent Refresh prüfen.
- Optional erneute interaktive Anmeldung nach Widerruf/Conditional-Access-Challenge
  beobachten. Die Firmenrichtlinien dürfen nicht umgangen werden.
- Falsche Prüfsumme, fehlende Plattform und fehlende Blob-Rechte liefern Fehler.
- Mit `AZURE_AC_NONINTERACTIVE=1` darf keine interaktive Anmeldung starten.

## Wenn der Login nicht passt

- Mehrere Firmen/Gastkonten: `tenant_id` explizit setzen.
- Entwicklungs-App gesperrt: eigene Public-Client-App mit `client_id`, passendem
  WAM-Redirect und Azure-Storage-Zugriff verwenden; siehe README.
- Account-Picker statt gewünschtem SSO: WAM-Verfügbarkeit und verbundenes
  Windows-Konto prüfen. Derzeit wird das Standard-Brokerkonto bevorzugt.
- Device-Code nur ausdrücklich mit `auth = "device"` testen; nicht als stillen
  Ersatz für einen durch Richtlinien abgelehnten Broker verwenden.
- Ein erfolgreicher Login mit HTTP 403 beim Lesen deutet häufig auf fehlende
  **Datenebenenrechte**, falschen Tenant oder Storage-Netzwerkregeln hin.
- Bei blockierten PowerShell-Skripten die freigegebene Unternehmenslösung nutzen;
  der Launcher setzt keine Execution-Policy außer Kraft.

Für die Rückmeldung genügen mise-Version, Windows-Version, ausgeführter Befehl,
Fehlerklasse/HTTP-Status und anonymisierte Konfiguration. **Keine Tokens, Secrets,
SAS-URLs oder vollständigen Credential-Logs weitergeben.**
