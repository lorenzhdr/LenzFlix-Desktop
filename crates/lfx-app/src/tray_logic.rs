//! W5-tray: reine Logik des Infobereich-Symbols (Einstellung, Menüeinträge, Schließen-Entscheidung).
//! Ohne Fenster und ohne Betriebssystem, damit sie unter `cargo test` laufen kann.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// App-weite Fenster-Einstellungen (nicht je Benutzer: das Symbol gehört zum Gerät).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TrayPrefs {
    /// „Beim Schließen im Infobereich weiterlaufen“ (Standard aus).
    pub close_to_tray: bool,
}

impl TrayPrefs {
    /// Speicherort in der Konfiguration.
    pub fn path(config_dir: &Path) -> PathBuf {
        config_dir.join("window.json")
    }

    /// Lädt; fehlende oder kaputte Datei ergibt die Standardwerte.
    pub fn load(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    /// Speichert atomar (temporäre Datei, dann Umbenennen).
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        let json = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, path)
    }
}

/// Einträge des Symbol-Menüs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    /// Fenster zeigen und nach vorn holen.
    Open,
    /// Wiedergabe pausieren bzw. fortsetzen.
    TogglePause,
    /// App wirklich beenden (auch bei „im Infobereich weiterlaufen“).
    Quit,
}

impl TrayAction {
    /// Alle Einträge in Menüreihenfolge.
    pub const ALL: [Self; 3] = [Self::Open, Self::TogglePause, Self::Quit];

    /// Stabile Kennung je Eintrag (Menüereignisse, Tests).
    pub fn id(self) -> &'static str {
        match self {
            Self::Open => "lfx-tray-open",
            Self::TogglePause => "lfx-tray-pause",
            Self::Quit => "lfx-tray-quit",
        }
    }

    /// Aus der Menükennung.
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.id() == id)
    }
}

/// Beschriftung des Pause-Eintrags: „fortsetzen“ nur bei pausierter Wiedergabe.
pub fn pause_label(playing: bool, paused: bool) -> &'static str {
    if playing && paused {
        "Wiedergabe fortsetzen"
    } else {
        "Wiedergabe pausieren"
    }
}

/// Was tun, wenn das Fenster geschlossen werden soll (X, Alt+F4, Schließen-Knopf)?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseDecision {
    /// App beenden.
    Quit,
    /// Fenster verstecken, App läuft im Infobereich weiter.
    HideToTray,
}

/// Entscheidung: nur verstecken, wenn die Einstellung an ist UND das Symbol tatsächlich existiert
/// (sonst gäbe es keinen Weg zurück ins Fenster).
pub fn close_decision(prefs: TrayPrefs, tray_ready: bool) -> CloseDecision {
    if prefs.close_to_tray && tray_ready {
        CloseDecision::HideToTray
    } else {
        CloseDecision::Quit
    }
}

/// Hinweistext des Symbols.
#[cfg(any(windows, target_os = "macos"))]
pub const TOOLTIP: &str = "LENZFLIX";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_ist_aus() {
        assert!(!TrayPrefs::default().close_to_tray);
    }

    #[test]
    fn schliessen_nur_mit_symbol_verstecken() {
        let on = TrayPrefs {
            close_to_tray: true,
        };
        assert_eq!(close_decision(on, true), CloseDecision::HideToTray);
        // Symbol nicht anlegbar: lieber beenden als ein unauffindbares Fenster.
        assert_eq!(close_decision(on, false), CloseDecision::Quit);
        assert_eq!(
            close_decision(TrayPrefs::default(), true),
            CloseDecision::Quit
        );
    }

    #[test]
    fn menue_kennungen_rundlauf() {
        for a in TrayAction::ALL {
            assert_eq!(TrayAction::from_id(a.id()), Some(a));
        }
        assert_eq!(TrayAction::from_id("fremd"), None);
    }

    #[test]
    fn pause_beschriftung() {
        assert_eq!(pause_label(false, false), "Wiedergabe pausieren");
        assert_eq!(pause_label(true, false), "Wiedergabe pausieren");
        assert_eq!(pause_label(true, true), "Wiedergabe fortsetzen");
    }

    #[test]
    fn speichern_und_laden() {
        let dir = tempfile::tempdir().unwrap();
        let p = TrayPrefs::path(dir.path());
        assert_eq!(TrayPrefs::load(&p), TrayPrefs::default());
        TrayPrefs {
            close_to_tray: true,
        }
        .save(&p)
        .unwrap();
        assert!(TrayPrefs::load(&p).close_to_tray);
        std::fs::write(&p, b"kaputt").unwrap();
        assert_eq!(TrayPrefs::load(&p), TrayPrefs::default());
    }
}
