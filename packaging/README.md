# Paketierung

Ausgabe von `cargo run -p xtask -- package --target <triple>` liegt in `target/dist/`.

- Windows (`aarch64-pc-windows-msvc`, `x86_64-pc-windows-msvc`): NSIS-Installer
  `LENZFLIX_<version>_<arm64|x64>-setup.exe`, Benutzerinstallation nach `%LOCALAPPDATA%\Programs\LENZFLIX`
  (kein Administrator), Startmenü-Eintrag, Deinstallation über „Apps“ oder `uninstall.exe`.
  Inhalt: `lenzflix.exe`, `libmpv-2.dll` (LGPL, austauschbar), `lenzflix.ico`, `LICENSES/`.
  Voraussetzung: `makensis` im PATH (NSIS 3.x; die Installer-Stubs sind x86 und laufen unter ARM64 per Emulation).
  Ohne `makensis` meldet xtask das klar; `--stage-only` erzeugt nur den Staging-Ordner samt `.nsi`-Aufruf.
- macOS (`aarch64-apple-darwin`, nur auf macOS): `LENZFLIX.app` mit libmpv und Abhängigkeiten in
  `Contents/Frameworks` (install_name per `install_name_tool`, ad-hoc `codesign`), daraus
  `LENZFLIX_<version>_arm64.dmg`.
- `Info.plist.in` und `windows/installer.nsi` sind die Vorlagen; Symbole/Icons erzeugt xtask aus `assets/logo/lenzflix-l.svg`.

Weitere Unterbefehle: `sign` (Minisign, Schlüssel aus `MINISIGN_SECRET_KEY` und `MINISIGN_PASSWORD`),
`manifest` (`SHA256SUMS.txt` und `latest.json`). Siehe `.github/workflows/release.yml`.
