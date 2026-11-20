//! Ereignisse der Seiten (`emitFx(name)` im Web) und Vorlieben.

use crate::synth::FxSound;

/// Vorlieben je Benutzer (Web: `lfx.retro.<name>.<user>`). Sound ist standardmäßig AUS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FxPrefs {
    pub sound: bool,
    pub crt: bool,
}

impl Default for FxPrefs {
    fn default() -> Self {
        Self {
            sound: false,
            crt: true,
        }
    }
}

/// Ereignisname einer Seite (`hover`, `click`, `type`, `open`, `close`, `nav`, `error`) auf den
/// Klang abbilden. Unbekannte Namen liefern `None` (wie `SOUNDS[name]` im Web).
pub fn sound_for_event(name: &str) -> Option<FxSound> {
    match name {
        "hover" | "blip" => Some(FxSound::Blip),
        "click" => Some(FxSound::Click),
        "type" => Some(FxSound::Type),
        "open" => Some(FxSound::Open),
        "close" => Some(FxSound::Close),
        "nav" => Some(FxSound::Nav),
        "error" => Some(FxSound::Error),
        "boot" => Some(FxSound::Boot),
        "jingle" => Some(FxSound::Jingle),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_sound_aus_crt_an() {
        let p = FxPrefs::default();
        assert!(!p.sound);
        assert!(p.crt);
    }

    #[test]
    fn ereignis_zuordnung() {
        assert_eq!(sound_for_event("hover"), Some(FxSound::Blip));
        assert_eq!(sound_for_event("error"), Some(FxSound::Error));
        assert_eq!(sound_for_event("nichts"), None);
    }
}
