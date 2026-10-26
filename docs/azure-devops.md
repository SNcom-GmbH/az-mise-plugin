# Azure DevOps mit v0.4.0

## Vertrag mit InstallBuilder@2

InstallBuilder@2 verwendet die Azure-DevOps-ARM-Service-Connection `az-development`
für den Zugriff auf Azure Blob Storage. Das gilt auch für Service Connections mit
Workload Identity Federation. Der Task übernimmt die Anmeldung mit der Service
Connection und beschafft ein Access Token für den Scope
`https://storage.azure.com/.default`.

Der Task startet `mise install --locked` mit `AZURE_AC_STORAGE_TOKEN` ausschließlich
im Environment des Kindprozesses. Er muss den Wert durch alle Zwischenprozesse
erhalten. Keine Token-Werte in Kommandoargumenten, Request-JSON, Tool-Optionen,
`mise.toml`, `mise.lock`, temporären Dateien, Caches oder Build-Artefakten speichern.
Auch keine Pipeline-Output-Variable oder `task.setvariable` dafür ausgeben. Der Task
darf weder das Environment noch den Token-Antwortinhalt protokollieren.

Das Token muss ein Storage-Access-Token sein. Eine ARM-Audience oder die rohe
OIDC-Assertion einer föderierten Service Connection genügt nicht. Der Helfer liest
keine Service-Connection-Konfiguration und führt diesen Token-Austausch nicht durch.
`AZURE_FEDERATED_TOKEN_FILE` bleibt der getrennte vorhandene Weg für eine vom
CI-Provider bereitgestellte Assertion-Datei.

Das Plugin verwendet das Token als `Authorization: Bearer ...` für `index.json`,
`release.json`, Blob-Metadaten und Artefakt-Downloads. Auch direkte Blob-Befehle
verwenden dieselbe Identität. Das Token hat Vorrang vor jeder gültigen `auth`-Option,
WAM, Service Principals, Workload Identity und Managed Identity. Ohne die Variable
bleibt die bisherige Authentifizierung erhalten. Ein leerer Wert zählt als gesetzt
und führt zum Fehler. Der Helfer lehnt auch Whitespace, Steuerzeichen, ein
`Bearer `-Präfix und andere ungültige Bearer-Syntax ab. Azure validiert die Claims
und die Signatur; das Plugin verlangt kein bestimmtes JWT-Format.

HTTP 401 und 403 sowie Fehler bei der Token-Validierung beenden den Vorgang.
Es gibt keinen Wechsel auf eine andere Identität oder einen interaktiven Login.
Bei 401 muss der Aufrufer Audience, Gültigkeit und Identität prüfen und bei Bedarf
ein neues Token beschaffen. Bei 403 sind die Datenebenenrechte und Netzwerkregeln
zu prüfen. `CS_*`-Signierzugangsdaten dürfen niemals für Blob-Zugriffe dienen.

## Lebensdauer und Rechte

Der Aufrufer ist für Beschaffung und Erneuerung verantwortlich. Der Helfer kann
das vorgegebene Token nicht erneuern und speichert es nur im Prozessspeicher.
Die SDK-Cache-Zeit verlängert seine tatsächliche Gültigkeit nicht. Vor längeren
Installationen muss der Task ein ausreichend lange gültiges Token beschaffen.
Ein neues Token im Elternprozess ändert nicht das Environment eines bereits
laufenden Helfers. Nach Ablauf ist ein neuer Aufruf mit einem neuen Token nötig.

Die Identität der Service Connection benötigt bestehende Datenebenenrechte am
Zielcontainer, zum Lesen etwa `Storage Blob Data Reader`. Eine ARM-Rolle allein
gewährt keinen Blob-Datenzugriff. Netzwerkregeln und Private Endpoints gelten
weiterhin. Die Einführung dieses Plugins ändert keine Azure-Berechtigungen.

## Deployment-Reihenfolge

1. Im Repository `SNcom-GmbH/az-mise-plugin` die Helfer-Binaries für `v0.4.0`
   und `SHA256SUMS` veröffentlichen. Beide Plattform-Assets müssen verfügbar sein:
   `azure-ac-linux-amd64-musl.tar.gz` und `azure-ac-windows-amd64.zip`.
2. Danach die aktualisierte Lua-Revision bereitstellen. Sie lädt genau den
   v0.4.0-Helfer aus dem neuen Repository.
3. In den Consumer-Repositories die Plugin-URLs auf
   `https://github.com/SNcom-GmbH/az-mise-plugin.git` und die Commit-Pins auf den
   vollständigen SHA dieser Revision aktualisieren. Bestehende Pins wechseln
   weder Repository noch Helferversion automatisch. Lockfiles weiterhin verwenden.
4. Erst danach die Service-Connection-Unterstützung von InstallBuilder@2 aktivieren
   und `az-development` verwenden.

Private GitHub-Release-Assets benötigen zusätzlich `GITHUB_TOKEN` oder `GH_TOKEN`
mit Leserechten auf das neue Repository. SSH-Zugriff auf das Plugin allein genügt
nicht. Das Storage-Token ersetzt diese GitHub-Zugangsdaten nicht.

Die Task-Implementierung und ihre Eingaben liegen im InstallBuilder-Repository.
Dieses Plugin implementiert den Environment-Vertrag; es aktiviert keinen Task,
veröffentlicht kein Release und löst keine Pipeline aus.

## Prüfung

`mise run check` prüft Rust-Formatierung, Clippy, Unit-Tests und echte mise-/Lua-
Integration ohne Azure-Zugriff. Die Tests verwenden ausschließlich künstliche
Zugangsdaten. Sie prüfen direkte und verschachtelte Prozesse, `mise install
--locked`, die SDK-Authorization-Header für Index, Manifest und Artefakt,
Token-Vorrang, Redaktion, HTTP 401/403 und die bisherige Environment-Anmeldung.

Vor der Aktivierung bleibt ein Test im Zieltenant erforderlich: mit der
Service Connection `az-development`, auch in ihrer föderierten Variante, auf
Windows- und Linux-Agents installieren. Dabei Token-Ablauf, fehlende Blob-Rechte,
Netzwerkregeln und private GitHub-Downloads prüfen. Erfolgreiche lokale Tests
ersetzen diesen Test nicht. Keine Token-Werte in das Testprotokoll aufnehmen.
