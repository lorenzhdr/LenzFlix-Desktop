//! Injizierte Uhr: der TV-Modus liest Zeit nie selbst, sondern nur über [`Clock`].

use std::cell::Cell;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Uhr in Millisekunden seit der Unix-Epoche (UTC).
pub trait Clock {
    /// Aktueller Zeitpunkt.
    fn now_ms(&self) -> i64;
}

/// Echte Systemzeit.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
    }
}

/// Von Hand gestellte Uhr (Tests, Harness). Klone teilen den Stand.
#[derive(Debug, Clone, Default)]
pub struct ManualClock(Rc<Cell<i64>>);

impl ManualClock {
    /// Uhr mit Startzeit.
    pub fn new(t: i64) -> Self {
        Self(Rc::new(Cell::new(t)))
    }

    /// Stellt die Uhr.
    pub fn set(&self, t: i64) {
        self.0.set(t);
    }

    /// Rückt die Uhr vor.
    pub fn advance(&self, ms: i64) {
        self.0.set(self.0.get() + ms);
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> i64 {
        self.0.get()
    }
}
