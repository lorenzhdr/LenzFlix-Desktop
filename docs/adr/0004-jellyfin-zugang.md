# ADR 0004: Jellyfin-Zugang mit eigenem Client, Quick Connect und Schlüsselbund

- Status: vorläufig (Live-Lauf offen)
- Datum: 2026-11-20
- Betroffene Crates: `lfx-api`, `lfx-platform`, `lfx-devcli`, `lfx-mock-jellyfin`

## Kontext
Gate G4: Die App muss sich ohne Web-Client an Jellyfin anmelden, Bibliothek und Wiedergabestatus lesen und schreiben, Token sicher speichern und Branding-/Charts-Angaben des Servers auswerten. Bisher nur gegen einen erfundenen Mock geprüft; ein Lauf gegen einen echten Server steht aus.

## Entscheidung
- Eigener Client `lfx-api` auf `reqwest` 0.13 (rustls) und `tokio`; DTOs nur mit benötigten Feldern, Endpunkte und Felder gegen die Jellyfin-OpenAPI-Spezifikation geprüft.
- Auth-Header `MediaBrowser Client="LENZFLIX", Device=…, DeviceId=…, Version=…[, Token=…]`, Werte prozentkodiert; die DeviceId stammt aus der Konfiguration (UUID v4).
- Anmeldung per Quick Connect (Poll alle 2 s, Timeout 5 min); Passwort-Anmeldung vorhanden, die Dev-CLI nutzt sie nicht.
- Tokens im OS-Schlüsselbund (`lfx-platform`: Trait `SecretStore`, Dienst "LENZFLIX", Konto `<server-id>:<user-id>`); Konfiguration als JSON v1, atomar geschrieben, beschädigte oder neuere Dateien werden nie überschrieben.
- Sicherheit: `Secret` ohne Serialize und mit Debug/Display `***`; `redact_url` schwärzt Host und sensible Query-Schlüssel; `fetch_bytes` lehnt fremde Hosts ab; Charts-Worker nur mit `X-Emby-Token` und ohne Redirects.
- Wiedergabe: `DeviceProfile::direct_play_everything()`, `SubtitleStreamIndex` -1, Fortschritt per Playing/Progress/Stopped; Retry (2 Wiederholungen, Backoff) nur für GET bei Netz/Timeout/502-504.
- Werkzeug `lfx-dev` (`login`, `whoami`, `servers`, `logout`, `discover`, `g4`) prüft den Ablauf mit den Checks `system_info_public`, `users_me`, `user_views`, Filmliste, Resume, NextUp, PlaybackInfo, Fortschritts-Roundtrip mit Wiederherstellung, Branding-CSS, Charts und Statistik.

## Alternativen
- Fremde Jellyfin-Client-Crates: nicht benötigt, DTO-Umfang und Header-Kontrolle sprechen für einen schlanken eigenen Client.
- Token in der Konfigurationsdatei: verworfen, Schlüsselbund ist Pflicht.

## Folgen
- Geprüft gegen den Mock: 28 Tests in `lfx-api`/Mock, 13 Tests in `lfx-platform` (plus 1 ignorierter Test mit echtem Schlüsselbund, lokal bestanden), 13 Integrationstests und 5 Branding-Unit-Tests in `lfx-devcli`; `g4` dreimal grün (ca. 15-35 ms), kein Token und kein Host in der Ausgabe. macOS-Schlüsselbund (`apple-native-keyring-store`) ist nur in der CI zu prüfen.
- Offen: Live-Lauf `lfx-dev g4` gegen einen echten Server (Nutzer meldet sich einmal an; Token landet im Schlüsselbund). `GET /Items/{id}?userId=` und die Endpunkte für Favorit/Gesehen/Bewertung sind dort noch ungeprüft. Gate G4 bleibt bis dahin vorläufig.
- Risiko im Live-Lauf: Jellyfin verwirft Positionen unter ca. 5 % der Laufzeit (MinResumePct); bei langen Filmen kann `progress_roundtrip` fehlschlagen. Dann kurzen Film wählen oder die Prüfung auf "Server hat Progress akzeptiert" lockern.
- Nicht rücksetzbare Spuren der Prüfung: PlayCount (+1) und LastPlayedDate; Position und Gesehen-Flag werden wiederhergestellt und verifiziert. Das Testkonto hat keine Transkodierung, daher h264-Titel.
- Der Branding-Abruf läuft anonym ohne Redirects, `@import` wird nie geladen; Charts-URL nur per https (http nur auf Loopback). Fehlt `--lfx-charts-url`, sind Charts-Checks SKIP.
- Folgearbeit: Heartbeat-Ping (`/Sessions/Playing/Ping`) bei Bedarf; Abhängigkeit von `lfx-platform` auf `lfx-api` (nur für `Secret`) später durch einen Typ aus `lfx-core` ersetzen; Workspace-Abhängigkeiten zentralisieren.
