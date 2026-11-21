# LENZFLIX Desktop – CLAUDE.md

Voll nativer Rust-Client (UI Slint, Wiedergabe libmpv) für Jellyfin, der das Web-Theme LenzFlix nachbaut: Designs LENZFLIX, LENZ+, LENZPRIME, LENZ//84, TV-Modus plus native Seiten (Detail, Player-OSD, Suche, Bibliothek, Einstellungen, Musik/Bücher/Fotos/Live-TV, Admin, Offline, Couch-Modus, Statistik). VIDEOTHEK LENZ nur als Slot (`lfx-3d`), kein VR. Plattformen: Windows ARM64, Windows x64, macOS Apple Silicon (nur CI). UI-Sprache nur Deutsch, App-Name „LENZFLIX“, eigene Titelleiste.

Paritätsliste: `docs/parity.md` (pflegt der Orchestrator, Status ändert sich dort). Entscheidungen: `docs/adr/` (Vorlage `0000-template.md`).

## Crates (`crates/*` plus `xtask`)
- `lfx-mpv-sys`: eigene FFI zu libmpv (client, render, render_d3d11/gl), dynamisch per `libloading` geladen.
- `lfx-api`: Jellyfin-REST-Client und DTOs (inkl. Admin, Musik, Live-TV, Charts-Client, Branding-Parser).
- `lfx-core`: reine Logik der Seiten (Zeilen, Billboard, Bibliothek, Neu und beliebt, Datensparmodus, Bildgrößen, Cache-Regeln, Tagesmischung); Golden gegen den Web-Code.
- `lfx-session`: Server, Benutzer, Anmeldung, Quick Connect, Token im Schlüsselbund, Erreichbarkeit.
- `lfx-store`: SQLite (rusqlite, WAL): Einstellungen, Outbox, Bildindex, Download-Zeilen; Migrationen nur anhängen.
- `lfx-images`: Bildlader mit Speicher- und Plattencache, Blurhash, Ambient-Farbe.
- `lfx-platform`: Schlüsselbund, Pfade, App-Konfig, Windows-SMTC/Medientasten, Wachhalten.
- `lfx-tv-schedule`: bitgleicher Port des TV-Planers (kein `Math.random`, keine Lokalzeit).
- `lfx-tv`: TV-Modus-Controller (sans-IO), EPG, Katalog, Stream-Einordnung.
- `lfx-player`: Wiedergabe (mpv-Zustand, Request-Aufbau, Fortschrittsmeldung, Segmente).
- `lfx-shell`: rahmenloses Fenster, DirectComposition (Video-Ebene unter Slint-UI), Hit-Test, Video-Format/HDR-Plan.
- `lfx-nav`: Fokusgraph, Tastatur-/Gamepad-Abbildung, Eingabe-Tracker (Couch-Erkennung).
- `lfx-ui`: Slint-Komponenten, Seiten, Theme-Globals (Tokens aus `tools/tokens`), Snapshot-Tests.
- `lfx-offline`: Download-Manager, Offline-Bibliothek, Fortschrittsabgleich.
- `lfx-remote`: Fernsteuerung per `/socket`, SyncPlay.
- `lfx-update`: Update-Prüfung, Minisign-/SHA-Prüfung, Installerstart.
- `lfx-retro-fx`: Retro-Effekte ohne UI (Synth, Boot, Terminal, Code-Regen, Cheat-Code).
- `lfx-app`: Binary `lenzflix`, Router, alle Seiten-Verdrahtung, Dev-Automation (Feature `dev-automation`) und E2E-Tests.
- `lfx-devcli`: Binary `lfx-dev` (Login per Quick Connect, `g4`-Serverchecks).
- `lfx-mock-jellyfin`: Testserver mit erfundenen Daten (je Welle ein `w*_*.rs`-Modul).
- `xtask`: `fetch-mpv`, `mpv-info`, `package`, `sign`, `manifest`. Dazu `spikes/g2-ui`, `spikes/g3-video` (eigene Workspaces, nur Messungen).
- Kein Crate `lfx-3d`: der VIDEOTHEK-Platz ist nur eine ausgegraute Karte im Picker.

## Befehle
- Vorher in Bash: `export PATH="$HOME/.cargo/bin:$PATH"`. Toolchain per `rust-toolchain.toml`, Edition 2024, Linker: Visual Studio 2022 Build Tools (ARM64 bzw. x64).
- `cargo build`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all --check`; Tests: `LFX_GOLDEN_REQUIRED=1 cargo test --workspace` (ohne die Variable überspringt der Planer-Golden-Test still, wenn die Vektoren fehlen).
- Golden erzeugen (Web-Repo als Quelle, `LENZFLIX_WEB_DIR=<Pfad>`, Standard = Elternrepo, Node ≥ 20): Planer `node tools/golden/gen.mjs` (nach `crates/lfx-tv-schedule/tests/golden/`, gitignoriert, 26 MB; Commit in `tools/golden/web-commit.txt`; `node tools/golden/selftest.mjs` prüft Determinismus), Kern `node crates/lfx-core/tests/golden/gen.mjs` (`web.json` ist eingecheckt), Ambient `node crates/lfx-images/tools/gen_ambient_vectors.mjs`. CI erzeugt die Planer-Vektoren je Job selbst.
- Tokens/Icons aus der Vorlage: `node tools/tokens/extract.mjs`, `node tools/icons/extract.mjs`.
- Snapshots: `cargo test -p lfx-ui --test snapshots` (Vergleich nur unter Windows; neu schreiben mit `LFX_UPDATE_SNAPSHOTS=1`).
- E2E (echte App gegen den Mock, alle `#[ignore]`, nur ein Thread, Fenster nicht verdecken): `cargo test -p lfx-app --test <name> -- --ignored --test-threads=1` mit `<name>` aus `e2e`, `e2e_tvui`, `e2e_player`, `e2e_detail`, `e2e_retro`, `home_e2e`, `prime_e2e`, `w2_library`, `w3_picker`, `w3_plus`, `w4_media`, `account_e2e`. Brauchen `ffmpeg` im Pfad (Testvideos, nie eingecheckt) und `vendor/mpv/<triple>/` (siehe `fetch-mpv`). Screenshots der Läufe: `LFX_SHOT_DIR`, `LFX_SHOT_UI_ONLY=1` (UI-Schnappschuss statt Desktop-Aufnahme).
- Dev-Automation (Feature `dev-automation`, 127.0.0.1, zufälliger Port, Einmal-Schlüssel in `<data>/automation.json`): Befehle `state`, `nav`, `route`, `login`, `quick_connect`, `design`, `screenshot`, `quit` plus je Bereich `home_*`, `detail_*`, `tv`, `library`/`search_text`, `picker_*`/`couch`, `retro_*`, `media*`, `account_*`. Env: `LFX_TV_NOW`, `LFX_RETRO_NO_BOOT=1`, `LFX_NO_CURSOR_POLL=1`, `LFX_AUDIO_NULL=1`, `LFX_NO_OPEN`, `LFX_OPEN_BROWSER=1`, `LFX_MOCK_VIDEO_FILE`, `LFX_MOCK_TRACK_SECS`.
- App starten: `cargo run -p lfx-app --bin lenzflix -- --home <ordner> [--server <url>] [--design flix|plus|prime|retro] [--maximized]`; mit `--home` liegen Tokens nur im Speicher (echter Schlüsselbund nur mit `--keyring`). Dev-CLI: `cargo run -p lfx-devcli -- --home <ordner> login --server <url>|g4|whoami|servers|logout|discover`.
- `cargo xtask fetch-mpv [--target <triple>]`: lädt die LGPL-libmpv (Windows: zhongfly-Builds, macOS: libmpv-darwin-build) nach `vendor/mpv/<triple>/` (gitignoriert), nie ins Repo; Pins in `xtask/mpv-pins.toml`.
- Pakete: `cargo xtask package --target <triple> [--skip-build] [--stage-only]` (Staging, ICO/ICNS, NSIS-Installer bzw. DMG nach `target/dist/`; braucht `makensis` bzw. unter macOS `hdiutil`), `cargo xtask sign --dir <ordner>` (Minisign, Env `MINISIGN_SECRET_KEY`/`MINISIGN_PASSWORD`; ohne Schlüssel übersprungen), `cargo xtask manifest --dir <ordner> --tag <vX.Y.Z> --base-url <url>` (`SHA256SUMS.txt` + `latest.json`). Der Update-Schlüssel `verify::UPDATE_PUBLIC_KEY` in `lfx-update` ist noch ein Platzhalter.
- Workspace-`Cargo.toml` (`members = ["crates/*", "xtask"]`) und Hotspots (Routen-Enum, `app.slint`/`main.slint`, Theme-Globals, DTO-Modul, Migrationen, `docs/parity.md`) ändert nur der Orchestrator; Coder markieren Einhängepunkte in gemeinsamen Dateien mit `// W<n>-<name>`.

## Betriebsfallen
- Platz: jeder Worktree erzeugt 14–25 GB in `target/`. Höchstens 5–6 gleichzeitig, nach dem Merge sofort `git worktree remove --force --force`; mit 11 Worktrees lief die Platte voll. Workflow-Worktrees nutzen teils Junctions, dann leert das Entfernen Verzeichnisse des Hauptprojekts.
- Parallele Builds kollidieren bei `aws-lc-sys` im gemeinsamen `%TEMP%` („unable to rename temporary“) → je Worker eigenes `TMP`/`TEMP`. Unter Last fehlt teils die Auslagerungsdatei; dann Lauf wiederholen.
- macOS ist nur in CI prüfbar (kein Mac): `cfg`-Pfade für macOS (Keychain, Now Playing, Vollbild, Video-Ebene) kompilieren lokal nicht; Stand dort nie als „geprüft“ melden.
- Mock-Routen nicht doppelt registrieren: axum verträgt doppelte Pfade nicht (Panik beim Start). Neue Routen in einem eigenen `w*_*.rs`-Modul per `.merge(...)` einhängen und vorher den Pfad in `lib.rs`/`extra.rs` suchen; der Mock bleibt im Standardverhalten unverändert (Erweiterungen opt-in).
- E2E teilen sich den Desktop und `target/`: nie parallel starten, Fenster nicht überdecken (GDI-Screenshots zeigen fremde Fenster), `e2e_player` ist unter Last flaky (`awake`-Zeitpunkt). Fenster ohne `--maximized` startet teils außerhalb des Bildschirms.
- Slint: `z`, `maximized`, `close` sind reservierte Namen; feste Größe ohne `x`/`y` zentriert im Elternelement; Plattform ist thread-lokal. Windows-Binaries mit „update“ im Namen brauchen ein Manifest (sonst Fehler 740).
- Zwei Cargo.lock-Fallen: neue Abhängigkeiten ändern die Lockdatei stark (Diff beim Merge prüfen); `serde_json` braucht `float_roundtrip`, sonst weichen JS-Vektoren um 1 ULP ab.
- Mock-Bilder sind gültige 24×36-PNG; Testvideos erzeugt der Mock per `ffmpeg` zur Laufzeit.

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
Stand 20.11.2026 (Version 0.1.0, nichts veröffentlicht): Wellen W1–W4 auf `main` (PR #1, CI grün auf Windows ARM64/x64 und macOS), W5 in Integration (Branch `integrate-w5`) (Downloads/Offline, Fernsteuerung/SyncPlay, Statistik, Tray, Feinschliff). Gates: G2 Slint bestanden (femtovg-wgpu bzw. Skia/D3D, ADR 0001), G3 Modell A trägt (DirectComposition, ADR 0002; HDR-Sichttest steht aus), G5 Planer bitgleich (ADR 0003), G4-Werkzeug `lfx-dev g4` nur gegen den Mock gelaufen. Alle Seiten und Designs laufen gegen den Mock mit E2E; kein Lauf gegen einen echten Server. Offene Punkte je Funktion: `docs/parity.md` (dort auch bewusste Abweichungen und Nicht-Prüfbares). HDR bleibt Platzhalter, bis der Nutzer HDR am Dev-Rechner einschaltet.

`.tmp/` ist gitignored und enthält `LEITPLANKEN.md` (unverhandelbar, nach jeder Kontext-Zusammenfassung lesen), `PLAN.md` (Vollplan), `STATUS.md` (Logbuch, Ende lesen), `denylist.txt` und `reports/` (Worker-Berichte). Bei jedem Meilenstein Stand hier und in `.tmp/STATUS.md` aktualisieren, knapp halten.
