//! Speicherort der Offline-Dateien und Prüfung des freien Platzes.
//!
//! Aufbau: `<Wurzel>/<server>/<benutzer>/<element>/` mit festen Dateinamen (`media.<ext>`, `media.part`,
//! `item.json`, `primary.img`, …). Pfade entstehen nur aus geprüften Kennungen, nie aus Titeln.

use crate::error::{OfflineError, Result};
use lfx_platform::AppDirs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Prüft eine Kennung als Pfadsegment (nur Buchstaben, Ziffern, `-`, `_`; höchstens 64 Zeichen).
pub fn safe_segment(id: &str) -> Result<&str> {
    let ok = !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if ok {
        Ok(id)
    } else {
        Err(OfflineError::InvalidId)
    }
}

/// Dateiendung aus einem Containernamen (nur Buchstaben/Ziffern, höchstens 8 Zeichen).
pub fn safe_extension(container: &str, fallback: &str) -> String {
    let first = container.split(',').next().unwrap_or("").trim();
    if !first.is_empty() && first.len() <= 8 && first.bytes().all(|b| b.is_ascii_alphanumeric()) {
        first.to_ascii_lowercase()
    } else {
        fallback.to_owned()
    }
}

/// Wurzelordner der Offline-Dateien.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfflineLayout {
    root: PathBuf,
}

impl OfflineLayout {
    /// Eigene Wurzel (z. B. vom Nutzer gewählter Speicherort).
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Standard: `offline/` im Datenordner der App (`lfx-platform`).
    pub fn from_dirs(dirs: &AppDirs) -> Self {
        Self::new(dirs.data.join("offline"))
    }

    /// Wurzelordner.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Ordner eines Elements für ein Konto.
    pub fn item_dir(&self, server_id: &str, user_id: &str, item_id: &str) -> Result<PathBuf> {
        Ok(self
            .root
            .join(safe_segment(server_id)?)
            .join(safe_segment(user_id)?)
            .join(safe_segment(item_id)?))
    }
}

/// Freier Speicherplatz auf dem Datenträger eines Pfads (austauschbar für Tests).
pub trait SpaceProbe: Send + Sync + 'static {
    /// Freie Bytes; `None`, wenn nicht ermittelbar (dann wird nicht geprüft).
    fn available(&self, path: &Path) -> Option<u64>;
}

/// Echte Abfrage über das Betriebssystem.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemSpace;

impl SpaceProbe for SystemSpace {
    fn available(&self, path: &Path) -> Option<u64> {
        // Der Zielordner existiert womöglich noch nicht: nächsten vorhandenen Vorfahren fragen.
        let mut probe = path;
        while !probe.exists() {
            probe = probe.parent()?;
        }
        fs4::available_space(probe).ok()
    }
}

/// Feste Angabe (Tests, einstellbar).
#[derive(Debug)]
pub struct FixedSpace(AtomicU64);

impl FixedSpace {
    /// Neue Angabe in Bytes.
    pub fn new(bytes: u64) -> Self {
        Self(AtomicU64::new(bytes))
    }

    /// Ändert den Wert.
    pub fn set(&self, bytes: u64) {
        self.0.store(bytes, Ordering::SeqCst);
    }
}

impl SpaceProbe for FixedSpace {
    fn available(&self, _path: &Path) -> Option<u64> {
        Some(self.0.load(Ordering::SeqCst))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_reject_paths() {
        assert!(safe_segment("abc-DEF_12").is_ok());
        for bad in ["", "..", "a/b", "a\\b", "a b", "ä"] {
            assert!(safe_segment(bad).is_err(), "{bad}");
        }
        assert!(safe_segment(&"a".repeat(65)).is_err());
    }

    #[test]
    fn extension_is_sanitized() {
        assert_eq!(safe_extension("MKV", "bin"), "mkv");
        assert_eq!(safe_extension("mov,mp4,m4a", "bin"), "mov");
        assert_eq!(safe_extension("../x", "bin"), "bin");
        assert_eq!(safe_extension("", "mp4"), "mp4");
    }

    #[test]
    fn system_space_answers_for_missing_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let free = SystemSpace.available(&tmp.path().join("a").join("b"));
        assert!(free.is_some_and(|b| b > 0));
    }
}
