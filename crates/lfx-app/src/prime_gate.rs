//! W3-prime: Vorschau-Gate von LENZPRIME (Web: `features/prime/preview-gate.js`). Videos bleiben sparsam: das
//! Hero-Video pausiert, solange es zu weniger als 20 % sichtbar ist, das Fenster verborgen ist oder eine
//! Vorschau-Karte (Hover) offen ist, und läuft danach weiter. Rein rechnerisch, damit testbar.

use std::rc::Rc;

use crate::app::App;
use crate::cli::DesignId;

/// Mindestanteil, den das Video sichtbar sein muss (Web: `VISIBLE_RATIO`).
pub const VISIBLE_RATIO: f32 = 0.2;

/// Eingaben des Gates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Gate {
    /// Sichtbarer Anteil des Videos (0..1).
    pub visible: f32,
    /// Fenster verborgen oder minimiert.
    pub hidden: bool,
    /// Eine Vorschau-Karte ist offen.
    pub held: bool,
}

impl Gate {
    /// Darf das Video laufen?
    pub fn want_play(&self) -> bool {
        self.visible >= VISIBLE_RATIO && !self.hidden && !self.held
    }
}

impl App {
    /// Ist LENZPRIME das aktive Design?
    pub fn is_prime(&self) -> bool {
        self.st.borrow().design == DesignId::Prime
    }

    /// Eine Vorschau-Karte öffnet (`held`) oder schließt: das Hero-Video pausiert bzw. startet wieder.
    pub fn prime_preview_hold(self: &Rc<Self>, held: bool) {
        if !self.is_prime() {
            return;
        }
        if held {
            self.stop_trailer();
        } else if matches!(self.st.borrow().router.current(), crate::route::Route::Home) {
            self.schedule_trailer();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(visible: f32, hidden: bool, held: bool) -> Gate {
        Gate {
            visible,
            hidden,
            held,
        }
    }

    #[test]
    fn plays_only_when_visible_shown_and_not_held() {
        assert!(g(1.0, false, false).want_play());
        assert!(g(0.2, false, false).want_play());
        assert!(!g(0.19, false, false).want_play());
        assert!(!g(1.0, true, false).want_play());
        assert!(!g(1.0, false, true).want_play());
    }
}
