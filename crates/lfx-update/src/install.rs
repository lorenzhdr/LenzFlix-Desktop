//! Installation: Windows NSIS-Installer still starten, macOS App-Bundle aus der DMG austauschen.
//!
//! Die Skript-/Argument-Logik ist plattformunabhängig und testbar; der eigentliche Start
//! (`start_install`) ist hinter `cfg` und für macOS nur kompiliert, nicht live getestet.
//! Nach `Ok(())` MUSS die App sich sofort beenden, damit Installer bzw. Skript die Dateien ersetzen können.

use crate::error::UpdateError;
use std::path::{Path, PathBuf};

/// Einfaches Quoting für POSIX-`sh` (Single-Quotes).
pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Findet das `.app`-Bundle zu einem Pfad im Bundle (`…/X.app/Contents/MacOS/x`).
pub fn app_bundle_from_exe(exe: &Path) -> Option<PathBuf> {
    exe.ancestors()
        .find(|p| p.extension().is_some_and(|e| e == "app"))
        .map(Path::to_path_buf)
}

/// Austausch-Skript für macOS: wartet auf das Ende der App, hängt die DMG ein, ersetzt das Bundle
/// (mit Rücksicherung bei Fehler), entfernt die Quarantäne und startet die App neu.
pub fn macos_update_script(pid: u32, dmg: &Path, app_bundle: &Path) -> String {
    let dmg = sh_quote(&dmg.to_string_lossy());
    let app = sh_quote(&app_bundle.to_string_lossy());
    format!(
        r#"#!/bin/sh
PID={pid}
DMG={dmg}
APP={app}
while kill -0 "$PID" 2>/dev/null; do sleep 0.5; done
MNT=$(mktemp -d)
hdiutil attach -nobrowse -readonly -mountpoint "$MNT" "$DMG" >/dev/null || exit 1
NEW=$(find "$MNT" -maxdepth 1 -name '*.app' | head -n 1)
if [ -z "$NEW" ]; then hdiutil detach "$MNT" -quiet; exit 1; fi
rm -rf "$APP.old"
mv "$APP" "$APP.old" || {{ hdiutil detach "$MNT" -quiet; exit 1; }}
if ditto "$NEW" "$APP"; then
  rm -rf "$APP.old"
else
  rm -rf "$APP"
  mv "$APP.old" "$APP"
fi
hdiutil detach "$MNT" -quiet || true
xattr -dr com.apple.quarantine "$APP" 2>/dev/null || true
open "$APP"
"#
    )
}

/// Befehlszeile für Windows: kurz warten (App beendet sich), dann NSIS-Installer still (`/S`).
pub fn windows_command_line(installer: &Path) -> String {
    format!(
        "/C ping -n 3 127.0.0.1 >nul & \"{}\" /S",
        installer.to_string_lossy()
    )
}

/// Startet die Installation für die aktuelle Plattform. Danach sofort beenden.
pub fn start_install(package: &Path) -> Result<(), UpdateError> {
    start_platform(package)
}

#[cfg(windows)]
fn start_platform(package: &Path) -> Result<(), UpdateError> {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    if package
        .extension()
        .is_none_or(|e| !e.eq_ignore_ascii_case("exe"))
    {
        return Err(UpdateError::Install(
            "Paket ist kein Installer (.exe)".into(),
        ));
    }
    std::process::Command::new("cmd")
        .raw_arg(windows_command_line(package))
        .creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW)
        .spawn()
        .map(|_| ())
        .map_err(|e| UpdateError::Install(e.to_string()))
}

#[cfg(target_os = "macos")]
fn start_platform(package: &Path) -> Result<(), UpdateError> {
    let exe = std::env::current_exe().map_err(|e| UpdateError::Install(e.to_string()))?;
    let app = app_bundle_from_exe(&exe)
        .ok_or_else(|| UpdateError::Install("App läuft nicht aus einem .app-Bundle".into()))?;
    let script = macos_update_script(std::process::id(), package, &app);
    let script_path = package.with_extension("update.sh");
    std::fs::write(&script_path, script)?;
    std::process::Command::new("/bin/sh")
        .arg(&script_path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| UpdateError::Install(e.to_string()))
}

#[cfg(not(any(windows, target_os = "macos")))]
fn start_platform(_package: &Path) -> Result<(), UpdateError> {
    Err(UpdateError::Install(
        "Plattform wird nicht unterstützt".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_schuetzt_einfache_anfuehrungszeichen() {
        assert_eq!(sh_quote("a'b"), "'a'\\''b'");
    }

    #[test]
    fn bundle_wird_aus_exe_pfad_gefunden() {
        let p = Path::new("/Applications/LENZFLIX.app/Contents/MacOS/lenzflix");
        assert_eq!(
            app_bundle_from_exe(p).unwrap(),
            PathBuf::from("/Applications/LENZFLIX.app")
        );
        assert!(app_bundle_from_exe(Path::new("/usr/bin/lenzflix")).is_none());
    }

    #[test]
    fn macos_skript_enthaelt_alle_schritte() {
        let s = macos_update_script(
            42,
            Path::new("/tmp/u it.dmg"),
            Path::new("/Applications/L.app"),
        );
        assert!(s.contains("PID=42"));
        assert!(s.contains("DMG='/tmp/u it.dmg'"));
        for kw in [
            "hdiutil attach",
            "ditto",
            "quarantine",
            "open \"$APP\"",
            "mv \"$APP.old\" \"$APP\"",
        ] {
            assert!(s.contains(kw), "{kw}");
        }
    }

    #[test]
    fn windows_befehlszeile_ist_still() {
        let c = windows_command_line(Path::new("C:\\t\\setup.exe"));
        assert!(c.ends_with("\"C:\\t\\setup.exe\" /S"));
    }
}
