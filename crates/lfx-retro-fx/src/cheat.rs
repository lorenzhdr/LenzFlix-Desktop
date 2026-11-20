//! Cheat-Code (Pfeile + B A) und Mehrfach-Tipp-Geste (Port von `cheat.js` und `index.js`).

/// Die Tastenfolge (Web: `CHEAT_SEQUENCE`, `e.key`-Namen).
pub const CHEAT_SEQUENCE: [&str; 10] = [
    "ArrowUp",
    "ArrowUp",
    "ArrowDown",
    "ArrowDown",
    "ArrowLeft",
    "ArrowRight",
    "ArrowLeft",
    "ArrowRight",
    "b",
    "a",
];
/// Dauer des „Overdrive“ nach Auslösung.
pub const OVERDRIVE_MS: u32 = 20_000;
/// Anzahl schneller Tipps aufs Logo, die Overdrive auslösen.
pub const TAP_COUNT: u32 = 7;
/// Abstand zwischen Tipps in Millisekunden (kleiner = in Folge).
pub const TAP_GAP_MS: u64 = 400;
/// Langdruck aufs Logo öffnet das Terminal.
pub const LONG_PRESS_MS: u32 = 700;

/// Einzelbuchstaben klein, Rest unverändert (Web: `normalizeKey`).
fn normalize(key: &str) -> String {
    if key.chars().count() == 1 {
        key.to_lowercase()
    } else {
        key.to_string()
    }
}

/// Erkennt die Folge am Ende des Eingabestroms.
#[derive(Debug, Clone, Default)]
pub struct CheatSequence {
    buf: Vec<String>,
}

impl CheatSequence {
    pub fn new() -> Self {
        Self::default()
    }

    /// `true`, sobald die Folge vollständig am Ende steht (danach ist der Puffer leer).
    pub fn feed(&mut self, key: &str) -> bool {
        self.buf.push(normalize(key));
        if self.buf.len() > CHEAT_SEQUENCE.len() {
            self.buf.remove(0);
        }
        if self.buf.len() == CHEAT_SEQUENCE.len()
            && self.buf.iter().zip(CHEAT_SEQUENCE).all(|(a, b)| a == b)
        {
            self.buf.clear();
            return true;
        }
        false
    }

    pub fn reset(&mut self) {
        self.buf.clear();
    }
}

/// Zählt schnelle Tipps hintereinander.
#[derive(Debug, Clone, Default)]
pub struct TapCounter {
    count: u32,
    last: Option<u64>,
}

impl TapCounter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Tipp zum Zeitpunkt `now_ms`; Rückgabe: Anzahl in Folge (Abstand < 400 ms).
    pub fn tap(&mut self, now_ms: u64) -> u32 {
        let in_row = self
            .last
            .is_some_and(|l| now_ms.saturating_sub(l) < TAP_GAP_MS);
        self.count = if in_row { self.count + 1 } else { 1 };
        self.last = Some(now_ms);
        self.count
    }

    pub fn count(&self) -> u32 {
        self.count
    }

    pub fn reset(&mut self) {
        self.count = 0;
        self.last = None;
    }

    /// Tipp zählen und bei [`TAP_COUNT`] Treffern `true` melden (Zähler wird zurückgesetzt).
    pub fn tap_triggers(&mut self, now_ms: u64) -> bool {
        if self.tap(now_ms) >= TAP_COUNT {
            self.reset();
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folge_wird_erkannt() {
        let mut s = CheatSequence::new();
        let keys = [
            "ArrowUp",
            "ArrowUp",
            "ArrowDown",
            "ArrowDown",
            "ArrowLeft",
            "ArrowRight",
            "ArrowLeft",
            "ArrowRight",
            "B",
            "a",
        ];
        for (i, k) in keys.iter().enumerate() {
            assert_eq!(s.feed(k), i == keys.len() - 1, "bei {k}");
        }
        // Puffer ist leer: dieselbe Folge noch einmal
        for k in &keys[..9] {
            assert!(!s.feed(k));
        }
        assert!(s.feed("a"));
    }

    #[test]
    fn folge_mit_vorlauf_und_fehler() {
        let mut s = CheatSequence::new();
        for k in ["x", "y", "ArrowUp"] {
            s.feed(k);
        }
        let rest = [
            "ArrowUp",
            "ArrowDown",
            "ArrowDown",
            "ArrowLeft",
            "ArrowRight",
            "ArrowLeft",
            "ArrowRight",
            "b",
        ];
        for k in rest {
            assert!(!s.feed(k));
        }
        assert!(s.feed("a"), "Vorlauf stört nicht");
        s.reset();
        for k in ["ArrowUp", "ArrowDown", "b", "a"] {
            assert!(!s.feed(k));
        }
    }

    #[test]
    fn tipp_zaehler() {
        let mut t = TapCounter::new();
        assert_eq!(t.tap(1000), 1);
        assert_eq!(t.tap(1300), 2);
        assert_eq!(t.tap(1600), 3, "300 ms");
        assert_eq!(t.tap(2000), 1, "400 ms gilt nicht mehr als in Folge");
        assert_eq!(t.count(), 1);
        t.reset();
        assert_eq!(t.count(), 0);
    }

    #[test]
    fn sieben_tipps_loesen_aus() {
        let mut t = TapCounter::new();
        for i in 0..6 {
            assert!(!t.tap_triggers(i * 200));
        }
        assert!(t.tap_triggers(6 * 200));
        assert_eq!(t.count(), 0);
        // zu langsam
        for i in 0..10 {
            assert!(!t.tap_triggers(10_000 + i * 500));
        }
    }
}
