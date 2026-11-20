//! Merkt die zuletzt benutzte Eingabeart (Signal für den Couch-Look).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputKind {
    Mouse,
    Keyboard,
    Gamepad,
    /// Fernbedienung (Kanal-/Medientasten, Info, Menü ...).
    Remote,
}

/// Mausbewegungen unterhalb dieser Strecke (Pixel) gelten als Zittern.
const MOUSE_JITTER_PX: f32 = 4.0;

#[derive(Debug)]
pub struct InputTracker {
    last: InputKind,
    mouse_at: Option<(f32, f32)>,
}

impl Default for InputTracker {
    fn default() -> Self {
        Self {
            last: InputKind::Mouse,
            mouse_at: None,
        }
    }
}

impl InputTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn last(&self) -> InputKind {
        self.last
    }

    /// Couch-Look: zuletzt Gamepad oder Fernbedienung.
    pub fn couch_look(&self) -> bool {
        matches!(self.last, InputKind::Gamepad | InputKind::Remote)
    }

    /// Meldet eine Eingabe; `true`, wenn sich die Art geändert hat.
    pub fn note(&mut self, kind: InputKind) -> bool {
        let changed = self.last != kind;
        self.last = kind;
        changed
    }

    /// Mausposition; kleine Bewegungen (Zittern, Tastatur-Cursor) zählen nicht.
    pub fn note_mouse_moved(&mut self, x: f32, y: f32) -> bool {
        let Some((px, py)) = self.mouse_at else {
            self.mouse_at = Some((x, y));
            return false;
        };
        if (x - px).hypot(y - py) < MOUSE_JITTER_PX {
            return false;
        }
        self.mouse_at = Some((x, y));
        self.note(InputKind::Mouse)
    }
}
