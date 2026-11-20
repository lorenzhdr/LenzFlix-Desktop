//! Update-Einstellungen: Kanal, Prüfintervall, „später erinnern“, Abschalten.
//! Zeit kommt immer als Unix-Sekunden von außen (testbar). Speicherort bestimmt der Aufrufer
//! (z. B. unter dem Konfigurationsordner aus `lfx-platform`).

use crate::error::UpdateError;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Update-Kanal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    /// Nur stabile Releases.
    #[default]
    Stable,
    /// Auch Vorabversionen.
    Prerelease,
}

/// Dauer „später erinnern“ in Sekunden (24 h).
pub const REMIND_LATER_SECS: u64 = 24 * 3600;

/// Gespeicherte Einstellungen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdateSettings {
    /// Automatische Prüfung an/aus (aus = nur manuelle Prüfung).
    pub enabled: bool,
    /// Kanal.
    pub channel: Channel,
    /// Prüfintervall in Stunden (mindestens 1).
    pub check_interval_hours: u32,
    /// Unix-Zeit der letzten Prüfung (0 = nie).
    pub last_check: u64,
    /// Bis wann nicht erinnert wird (0 = nicht gesetzt).
    pub remind_after: u64,
    /// Übersprungene Version („Diese Version nicht mehr anzeigen“).
    pub skipped_version: Option<String>,
}

impl Default for UpdateSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            channel: Channel::Stable,
            check_interval_hours: 24,
            last_check: 0,
            remind_after: 0,
            skipped_version: None,
        }
    }
}

impl UpdateSettings {
    /// Ist jetzt eine automatische Prüfung fällig?
    pub fn check_due(&self, now: u64) -> bool {
        if !self.enabled || now < self.remind_after {
            return false;
        }
        let interval = u64::from(self.check_interval_hours.max(1)) * 3600;
        // Uhr zurückgestellt: sofort prüfen statt ewig zu warten.
        now < self.last_check || now - self.last_check >= interval
    }

    /// Prüfung ist erfolgt.
    pub fn mark_checked(&mut self, now: u64) {
        self.last_check = now;
    }

    /// „Später erinnern“: für [`REMIND_LATER_SECS`] keine automatische Meldung.
    pub fn remind_later(&mut self, now: u64) {
        self.remind_after = now + REMIND_LATER_SECS;
    }

    /// Diese Version nicht mehr melden.
    pub fn skip_version(&mut self, version: &str) {
        self.skipped_version = Some(version.to_string());
    }

    /// Soll ein gefundenes Update dem Nutzer automatisch gemeldet werden?
    pub fn should_announce(&self, version: &str, now: u64) -> bool {
        self.enabled && now >= self.remind_after && self.skipped_version.as_deref() != Some(version)
    }

    /// Lädt aus JSON; fehlende Datei oder kaputter Inhalt ergibt Standardwerte.
    pub fn load(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    /// Speichert atomar (temporäre Datei, dann Umbenennen).
    pub fn save(&self, path: &Path) -> Result<(), UpdateError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        let json =
            serde_json::to_vec_pretty(self).map_err(|e| UpdateError::Manifest(e.to_string()))?;
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
}

/// Aktuelle Unix-Zeit in Sekunden.
pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intervall_und_abschalten() {
        let mut s = UpdateSettings::default();
        assert!(s.check_due(1_000_000));
        s.mark_checked(1_000_000);
        assert!(!s.check_due(1_000_000 + 3600));
        assert!(s.check_due(1_000_000 + 24 * 3600));
        s.enabled = false;
        assert!(!s.check_due(10_000_000));
    }

    #[test]
    fn spaeter_erinnern_und_ueberspringen() {
        let mut s = UpdateSettings::default();
        s.remind_later(5000);
        assert!(!s.check_due(5000 + 100));
        assert!(!s.should_announce("0.2.0", 5100));
        assert!(s.should_announce("0.2.0", 5000 + REMIND_LATER_SECS));
        s.skip_version("0.2.0");
        assert!(!s.should_announce("0.2.0", 9_999_999));
        assert!(s.should_announce("0.3.0", 9_999_999));
    }

    #[test]
    fn speichern_und_laden() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("sub/update.json");
        let s = UpdateSettings {
            channel: Channel::Prerelease,
            check_interval_hours: 6,
            ..UpdateSettings::default()
        };
        s.save(&p).unwrap();
        assert_eq!(UpdateSettings::load(&p), s);
        std::fs::write(&p, b"kaputt").unwrap();
        assert_eq!(UpdateSettings::load(&p), UpdateSettings::default());
    }
}
