//! Glitch-Zufall (Port von `glitch.js`): gelegentlich (alle 20–40 s) flackert eine sichtbare
//! Überschrift 600 ms lang, dazu ein Streifen im Bildschirm-Effekt. Deterministisch per Seed.

use crate::rng::Rng;

/// Kürzester/längster Abstand zwischen zwei Glitches (Web-Voreinstellung).
pub const MIN_MS: u32 = 20_000;
pub const MAX_MS: u32 = 40_000;
/// Dauer der Klasse `is-glitching` am Ziel.
pub const GLITCH_MS: u32 = 600;

/// Ein geplanter Glitch.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlitchEvent {
    /// Zeitpunkt (ms seit Start des Zeitplans).
    pub at_ms: u64,
    /// Zufallswert [0,1) zur Wahl des Ziels unter den sichtbaren Kandidaten
    /// (`pick_index(n)`).
    pub pick: f64,
    /// Dauer des Effekts am Ziel.
    pub duration_ms: u32,
}

impl GlitchEvent {
    /// Index des Ziels unter `n` sichtbaren Kandidaten (None, wenn keiner da ist).
    pub fn pick_index(&self, n: usize) -> Option<usize> {
        if n == 0 {
            None
        } else {
            Some(((self.pick * n as f64) as usize).min(n - 1))
        }
    }
}

/// Zeitplan mit festem Seed.
#[derive(Debug, Clone)]
pub struct GlitchSchedule {
    rng: Rng,
    min_ms: u32,
    max_ms: u32,
    now_ms: u64,
}

impl GlitchSchedule {
    pub fn new(seed: u32) -> Self {
        Self::with_range(seed, MIN_MS, MAX_MS)
    }

    pub fn with_range(seed: u32, min_ms: u32, max_ms: u32) -> Self {
        Self {
            rng: Rng::new(seed),
            min_ms,
            max_ms: max_ms.max(min_ms),
            now_ms: 0,
        }
    }

    /// Nächster Glitch (Zeit läuft im Zeitplan weiter).
    pub fn next_event(&mut self) -> GlitchEvent {
        let delay = self
            .rng
            .range(f64::from(self.min_ms), f64::from(self.max_ms));
        self.now_ms += delay as u64;
        GlitchEvent {
            at_ms: self.now_ms,
            pick: self.rng.next_f64(),
            duration_ms: GLITCH_MS,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abstand_20_bis_40_s() {
        let mut s = GlitchSchedule::new(3);
        let mut last = 0;
        for _ in 0..200 {
            let e = s.next_event();
            let d = e.at_ms - last;
            assert!((20_000..40_000).contains(&d), "{d}");
            assert!((0.0..1.0).contains(&e.pick));
            assert_eq!(e.duration_ms, 600);
            last = e.at_ms;
        }
    }

    #[test]
    fn deterministisch() {
        let a: Vec<_> = {
            let mut s = GlitchSchedule::new(9);
            (0..10).map(|_| s.next_event()).collect()
        };
        let b: Vec<_> = {
            let mut s = GlitchSchedule::new(9);
            (0..10).map(|_| s.next_event()).collect()
        };
        assert_eq!(a, b);
        let mut c = GlitchSchedule::new(10);
        assert_ne!(a[0], c.next_event());
    }

    #[test]
    fn ziel_auswahl() {
        let e = GlitchEvent {
            at_ms: 0,
            pick: 0.999,
            duration_ms: 600,
        };
        assert_eq!(e.pick_index(0), None);
        assert_eq!(e.pick_index(4), Some(3));
        assert_eq!(GlitchEvent { pick: 0.0, ..e }.pick_index(4), Some(0));
    }
}
