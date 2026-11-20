//! W5-tray: reine Logik der Update-Oberfläche (Phasen, Knöpfe, Texte). Ohne Fenster und Netz testbar.

use lfx_update::UpdateError;

/// Phase der Update-Anzeige.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Phase {
    /// Nichts bekannt.
    #[default]
    Idle,
    /// Prüfung läuft.
    Checking,
    /// Aktuell, kein Update.
    UpToDate,
    /// Update gefunden (Dialog mit Hinweisen).
    Available,
    /// Paket wird geladen.
    Downloading,
    /// Paket geprüft, Installation wird gestartet.
    Installing,
    /// Prüfung, Download oder Installation fehlgeschlagen.
    Error,
}

impl Phase {
    /// Kennung für Tests und Automation.
    pub fn name(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Checking => "checking",
            Self::UpToDate => "uptodate",
            Self::Available => "available",
            Self::Downloading => "downloading",
            Self::Installing => "installing",
            Self::Error => "error",
        }
    }
}

/// Aktion eines Dialogknopfs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    /// „Jetzt installieren“ bzw. „Erneut versuchen“.
    Install,
    /// „Später“ (24 Stunden nicht erinnern).
    Later,
    /// „Überspringen“ (diese Version nie mehr melden).
    Skip,
    /// Download abbrechen.
    Cancel,
    /// Dialog schließen (nach Fehler).
    Close,
}

impl Button {
    /// Beschriftung.
    pub fn label(self, phase: Phase) -> &'static str {
        match self {
            Self::Install if phase == Phase::Error => "Erneut versuchen",
            Self::Install => "Jetzt installieren",
            Self::Later => "Später",
            Self::Skip => "Überspringen",
            Self::Cancel => "Abbrechen",
            Self::Close => "Schließen",
        }
    }

    /// Kennung für die Automation.
    pub fn name(self) -> &'static str {
        match self {
            Self::Install => "install",
            Self::Later => "later",
            Self::Skip => "skip",
            Self::Cancel => "cancel",
            Self::Close => "close",
        }
    }
}

/// Knöpfe des Dialogs je Phase, in Anzeigereihenfolge (der erste ist die Hauptaktion).
pub fn buttons(phase: Phase, can_retry: bool) -> Vec<Button> {
    match phase {
        Phase::Available => vec![Button::Install, Button::Later, Button::Skip],
        Phase::Downloading => vec![Button::Cancel],
        Phase::Installing => vec![],
        Phase::Error if can_retry => vec![Button::Install, Button::Close],
        Phase::Error => vec![Button::Close],
        _ => vec![],
    }
}

/// Fortschritt als Bruchteil 0..1 (0 bei unbekannter Gesamtgröße).
pub fn fraction(done: u64, total: u64) -> f32 {
    if total == 0 {
        0.0
    } else {
        (done as f64 / total as f64).clamp(0.0, 1.0) as f32
    }
}

/// „12,3 von 45,6 MB“.
pub fn progress_text(done: u64, total: u64) -> String {
    let mb = |b: u64| format!("{:.1}", b as f64 / 1_048_576.0).replace('.', ",");
    if total == 0 {
        format!("{} MB", mb(done))
    } else {
        format!("{} von {} MB", mb(done), mb(total))
    }
}

/// Kürzt Versionshinweise für den Dialog (an Zeilengrenzen, höchstens `max_chars` Zeichen).
pub fn trim_notes(notes: &str, max_chars: usize) -> String {
    let notes = notes.trim();
    if notes.chars().count() <= max_chars {
        return notes.to_owned();
    }
    let mut out = String::new();
    for line in notes.lines() {
        if out.chars().count() + line.chars().count() + 1 > max_chars {
            break;
        }
        out.push_str(line);
        out.push('\n');
    }
    if out.is_empty() {
        out = notes.chars().take(max_chars).collect();
    }
    format!("{} …", out.trim_end())
}

/// Verständliche Fehlermeldung (ohne technische Einzelheiten und ohne Adressen).
pub fn error_text(e: &UpdateError) -> String {
    match e {
        UpdateError::KeyNotConfigured => {
            "Updates sind in dieser Version noch nicht eingerichtet (Signaturschlüssel fehlt)."
                .into()
        }
        UpdateError::Http(_) | UpdateError::Status(_) => {
            "Keine Verbindung zum Update-Server. Bitte später erneut versuchen.".into()
        }
        UpdateError::HashMismatch | UpdateError::SizeMismatch | UpdateError::Signature(_) => {
            "Das geladene Paket ist nicht vertrauenswürdig und wurde verworfen.".into()
        }
        UpdateError::NoAssetForPlatform(_) => {
            "Für dieses System gibt es noch kein Update-Paket.".into()
        }
        other => other.to_string(),
    }
}

/// Statuszeile in den Einstellungen.
pub fn status_line(phase: Phase, version: &str, error: &str) -> String {
    match phase {
        Phase::Idle => "Noch nicht geprüft".into(),
        Phase::Checking => "Prüfung läuft …".into(),
        Phase::UpToDate => "LENZFLIX ist auf dem neuesten Stand".into(),
        Phase::Available => format!("Version {version} ist verfügbar"),
        Phase::Downloading => format!("Version {version} wird geladen …"),
        Phase::Installing => "Installation wird gestartet …".into(),
        Phase::Error => error.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knoepfe_je_phase() {
        assert_eq!(
            buttons(Phase::Available, true),
            vec![Button::Install, Button::Later, Button::Skip]
        );
        assert_eq!(buttons(Phase::Downloading, true), vec![Button::Cancel]);
        assert!(buttons(Phase::Installing, true).is_empty());
        assert_eq!(buttons(Phase::Error, false), vec![Button::Close]);
        assert_eq!(Button::Install.label(Phase::Error), "Erneut versuchen");
        assert_eq!(
            Button::Install.label(Phase::Available),
            "Jetzt installieren"
        );
    }

    #[test]
    fn fortschritt() {
        assert_eq!(fraction(50, 100), 0.5);
        assert_eq!(fraction(5, 0), 0.0);
        assert_eq!(fraction(200, 100), 1.0);
        assert_eq!(progress_text(1_572_864, 3_145_728), "1,5 von 3,0 MB");
        assert_eq!(progress_text(1_048_576, 0), "1,0 MB");
    }

    #[test]
    fn hinweise_kuerzen() {
        assert_eq!(trim_notes("  kurz  ", 50), "kurz");
        let long = "Zeile eins\nZeile zwei\nZeile drei";
        assert_eq!(trim_notes(long, 24), "Zeile eins\nZeile zwei …");
        assert!(trim_notes(&"x".repeat(100), 10).starts_with("xxxxxxxxxx"));
    }

    #[test]
    fn fehlertexte_ohne_adressen() {
        assert!(error_text(&UpdateError::KeyNotConfigured).contains("nicht eingerichtet"));
        assert!(error_text(&UpdateError::Http("https://x.invalid".into())).contains("Verbindung"));
        assert!(!error_text(&UpdateError::Http("https://x.invalid".into())).contains("x.invalid"));
        assert!(error_text(&UpdateError::HashMismatch).contains("verworfen"));
    }

    #[test]
    fn statuszeilen() {
        assert_eq!(
            status_line(Phase::Available, "0.2.0", ""),
            "Version 0.2.0 ist verfügbar"
        );
        assert_eq!(status_line(Phase::Error, "", "Fehler"), "Fehler");
    }
}
