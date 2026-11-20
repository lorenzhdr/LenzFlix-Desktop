# LENZFLIX Desktop – CLAUDE.md

Voll nativer Rust-Client (UI Slint, Wiedergabe libmpv) für Jellyfin, der das Web-Theme LenzFlix nachbaut: Designs LENZFLIX, LENZ+, LENZPRIME, LENZ//84, TV-Modus plus native Seiten (Detail, Player-OSD, Suche, Bibliothek, Einstellungen, Musik/Bücher/Fotos/Live-TV, Admin, Offline, Couch-Modus, Statistik). VIDEOTHEK LENZ nur als Slot (`lfx-3d`), kein VR. Plattformen: Windows ARM64, Windows x64, macOS Apple Silicon (nur CI). UI-Sprache nur Deutsch, App-Name „LENZFLIX“, eigene Titelleiste.

Paritätsliste: `docs/parity.md` (pflegt der Orchestrator, Status ändert sich dort). Entscheidungen: `docs/adr/` (Vorlage `0000-template.md`).

## Crates (geplant, `crates/*` plus `xtask`)
`lfx-mpv-sys` (eigene FFI, dynamisch geladen) · `lfx-api` (Jellyfin-REST/WebSocket) · `lfx-core` (Konfig, Log, Schwärzung, Datensparmodus) · `lfx-session` (Server, Benutzer, Quick Connect, Schlüsselbund) · `lfx-store` (lokaler Cache) · `lfx-offline` (Downloads) · `lfx-tv-schedule` (Planer-Port) · `lfx-tv` (TV-Modus) · `lfx-player` (Wiedergabe) · `lfx-shell` (Fenster, Titelleiste) · `lfx-nav` (Routen, Fokus, Gamepad) · `lfx-ui` (Slint-Seiten, Theme-Globals) · `lfx-platform` (SMTC, Tray, Update, OS) · `lfx-3d` (Slot) · `lfx-app` (Binary) · `lfx-mock-jellyfin` (Testserver) · `xtask`.

## Befehle
- Vorher in Bash: `export PATH="$HOME/.cargo/bin:$PATH"`. Toolchain per `rust-toolchain.toml`, Edition 2024, Linker: Visual Studio 2022 Build Tools (ARM64 bzw. x64).
- `cargo build`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`.
- `cargo xtask fetch-mpv`: lädt die LGPL-libmpv (Windows: zhongfly-Builds, macOS: libmpv-darwin-build) nach `.tmp/`/`target/`, nie ins Repo.
- Mehrere Worktrees und Coder bauen im selben `target/`; der Cargo-Lock wartet automatisch.
- Workspace-`Cargo.toml` (`members = ["crates/*", "xtask"]`) und Hotspots (Routen-Enum, `app.slint`, Theme-Globals, DTO-Modul, Migrationen, `docs/parity.md`) ändert nur der Orchestrator.

## Regeln
- **Öffentliches Repo** (`lorenzhdr/LenzFlix-Desktop`): nie committen: private Domains (Dev-Server, Worker), Wetterort/Koordinaten, E-Mails, echte Benutzernamen/IDs, Tokens/API-Keys, DeviceIds, Pfade mit Benutzernamen (relative Pfade schreiben), Screenshots/Fixtures mit echten Titeln oder Covern, `.tmp/`, `reference/`, Binärdateien (libmpv-DLL), Minisign-Privatschlüssel. Fixtures nur synthetisch (erfundene Titel, generierte Platzhalter-Cover). Vor jedem Commit `git diff --cached --stat` und Diff prüfen; Pre-commit-Hook (gitleaks + `.tmp/denylist.txt`).
- **Marken:** keine fremden Marken-, Dienst- oder Gerätenamen, Logos oder Assets in Code und Texten. Eigene Marken: „LENZFLIX“, „LENZ+“, „LENZPRIME“, „LENZ//84“, „VIDEOTHEK LENZ“.
- **Lizenz:** libmpv nur LGPL und nur dynamisch (`libloading`), DLL nie committen. Kein GPL-Code kopieren (Lumiere, jellyfin-desktop, Slint-Backends). Slint unter Royalty-free 2.0: AboutSlint in App und README. Fremde Schriften mit Lizenztext.
- **Token-Schwärzung:** Tokens und Server-URLs nie loggen, in Logs und Fehlern schwärzen (mit Test). Zugangsdaten nur im OS-Schlüsselbund. Bild-URLs ohne `ApiKey`.
- **TV-Planer** (`lfx-tv-schedule`) muss bitgleich zum Web bleiben und deterministisch sein: kein Zufall außer dem Hash-Port, keine Lokalzeit, kein `Intl`, Zeit nur MEZ/MESZ arithmetisch. Absicherung über Golden-Vektoren aus dem Web-Code. Port-Fallen: UTF-16-Code-Units (fnv1a, Vergleiche), `Math.imul`/`>>>`/`Math.round`, `Date.parse` wie V8, Einfügereihenfolge (IndexMap), stabile Sortierung, `dedupe`-Gleichstand per JSON-Reihenfolge.
- **Web-Einstellungen:** nie in Jellyfins Web-DisplayPreferences schreiben. Die App hat eigenen Speicher; Server-Daten nur lesen oder über die dafür vorgesehenen API-Pfade ändern.
- **Web-Vorlage:** das Web-Theme-Repo LenzFlix (lokal eine Ebene höher) ist eine read-only Vorlage; dort nichts ändern. Nur Deutsch in der UI, Code-Kommentare und Doku Deutsch, Bezeichner Englisch.
- **Player-Fallen** (aus dem Web-TV-Player übernehmen): nie `StartTimeTicks` an die HLS-URL, `<video>`/Player erst leeren, dann `DELETE /Videos/ActiveEncodings`, `ApiKey` statt `api_key`, bei Untertiteln Brennen vermeiden (`docs/tvmode.md` des Web-Repos). Testkonto ohne Transkodierung: für Tests h264-Titel nehmen.
- **Prüfung:** nicht prüfbar sind macOS-Laufzeit, Transkodierung, Live-TV ohne Tuner; HDR-Test erst nach Bitte an den Nutzer (HDR am Dev-Rechner aus). Das nie als „geprüft“ melden.
- Keine Netzwerkzugriffe in Tests (gegen `lfx-mock-jellyfin` testen).
- Versionen ab 0.1.0; Release erst bei voller Parität.

## Aktueller Stand
Phase 0 begonnen am 20.11.2026: Workspace-Skelett, Toolchain (rustup, VS Build Tools ARM64), Paritätsliste, Golden-Vektoren und Planer-Port, Design-Token-Extraktion, CI/Hooks. Gates: G1 Toolchain, G2 Slint-UI (Fallback iced nur bei Scheitern), G3 HDR, G5 Planer-Golden. Noch keine Crates mit Inhalt, kein Commit.

`.tmp/` ist gitignored und enthält `LEITPLANKEN.md` (unverhandelbar, nach jeder Kontext-Zusammenfassung lesen), `PLAN.md` (Vollplan), `STATUS.md` (Logbuch, Ende lesen), `denylist.txt` und `reports/` (Worker-Berichte). Bei jedem Meilenstein Stand hier und in `.tmp/STATUS.md` aktualisieren, knapp halten.
