# LENZFLIX Desktop

Nativer Desktop-Client für Jellyfin, geschrieben in Rust (Oberfläche mit Slint, Wiedergabe mit libmpv). Er baut den Look des Web-Themes nach: https://github.com/lorenzhdr/LenzFlix

Inoffiziell, nicht mit dem Jellyfin-Projekt verbunden.

**Status: in Entwicklung (Phase 0).** Es gibt noch kein Release.

## Plattformen

- Windows 11 ARM64 (`aarch64-pc-windows-msvc`)
- Windows x64 (`x86_64-pc-windows-msvc`)
- macOS Apple Silicon (`aarch64-apple-darwin`, nur per CI gebaut)

## Bauen

Voraussetzungen: Rust (stable, siehe `rust-toolchain.toml`) und unter Windows die Visual Studio 2022 Build Tools mit MSVC-Toolchain für das Zielsystem.

```sh
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

### libmpv holen

Die Wiedergabe nutzt libmpv (LGPL), die nie im Repo liegt. Der Hilfsbefehl lädt das gepinnte Paket, prüft die SHA-256-Summe und entpackt es nach `vendor/mpv/<ziel>/` (gitignoriert):

```sh
cargo run -p xtask -- fetch-mpv                                  # passend zum Host
cargo run -p xtask -- fetch-mpv --target x86_64-pc-windows-msvc  # anderes Ziel
cargo run -p xtask -- mpv-info                                   # Exporte und API-Version prüfen
```

Versionen und Prüfsummen stehen in `xtask/mpv-pins.toml`.

## Entwicklung

Git-Hooks aktivieren (blockiert Geheimnisse, Binärdateien, private Begriffe und fremde Markennamen):

```sh
git config core.hooksPath .githooks
```

Der Hook nutzt `gitleaks`, falls installiert, und die lokale, nicht eingecheckte Liste `.tmp/denylist.txt`. Die Markenliste liegt in `tools/brand-denylist.txt`.

## Hinweis zu unsignierten Builds

Die Builds sind nicht signiert. Windows SmartScreen warnt beim ersten Start („Weitere Informationen“, dann „Trotzdem ausführen“). Unter macOS blockiert Gatekeeper die App zunächst (Rechtsklick auf die App, „Öffnen“ bzw. in den Systemeinstellungen unter „Datenschutz & Sicherheit“ freigeben).

## Lizenz

MIT, siehe `LICENSE`. Drittanbieter-Komponenten: `THIRD-PARTY-NOTICES.md`.

Made with [Slint](https://slint.dev).
