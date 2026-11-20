//! Ziffern-Eingabe der Sendernummer 1 bis 18 (Port von `tvmode/digits.js`), ohne Timer: der
//! Aufrufer ruft [`Digits::poll`] mit der Uhrzeit auf (`next_deadline` sagt, wann).
//!
//! `0` wartet auf die zweite Ziffer (Zeitablauf verwirft), `1` wartet (Zeitablauf schaltet Sender
//! 1), `2` bis `9` schalten sofort. Zweite Ziffer: 1 bis 18 gültig, sonst Hinweis.

/// Größte Sendernummer.
pub const MAX_CHANNEL: u32 = 18;
/// Standard-Zeitablauf.
pub const DEFAULT_TIMEOUT_MS: i64 = 1500;

/// Ergebnis einer Eingabe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DigitsEvent {
    /// Sender gewählt.
    Commit(u32),
    /// Anzeige der Eingabe (`1_`); `None` leert sie.
    Preview(Option<String>),
    /// Ungültige Nummer.
    Invalid(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Armed {
    Reset,
    Commit,
}

/// Zustand der Ziffern-Eingabe.
#[derive(Debug, Clone)]
pub struct Digits {
    buf: String,
    at: i64,
    timer: Option<(i64, Armed)>,
    timeout_ms: i64,
    disposed: bool,
}

impl Default for Digits {
    fn default() -> Self {
        Self::new(DEFAULT_TIMEOUT_MS)
    }
}

impl Digits {
    /// Neue Eingabe mit Zeitablauf.
    pub fn new(timeout_ms: i64) -> Self {
        Self {
            buf: String::new(),
            at: 0,
            timer: None,
            timeout_ms,
            disposed: false,
        }
    }

    /// Offener Puffer (`""` = nichts offen).
    pub fn pending(&self) -> &str {
        &self.buf
    }

    /// Zeitpunkt des nächsten Zeitablaufs.
    pub fn next_deadline(&self) -> Option<i64> {
        self.timer.map(|t| t.0)
    }

    fn reset(&mut self, out: &mut Vec<DigitsEvent>) {
        self.timer = None;
        if !self.buf.is_empty() {
            self.buf.clear();
            out.push(DigitsEvent::Preview(None));
        }
    }

    fn commit_pending(&mut self, out: &mut Vec<DigitsEvent>) {
        let no: u32 = self.buf.parse().unwrap_or(0);
        self.reset(out);
        if (1..=MAX_CHANNEL).contains(&no) {
            out.push(DigitsEvent::Commit(no));
        }
    }

    fn arm(&mut self, now: i64, kind: Armed) {
        self.at = now;
        self.timer = Some((now + self.timeout_ms, kind));
    }

    /// Nimmt eine Ziffer 0 bis 9 entgegen; andere Zeichen werden ignoriert.
    pub fn push(&mut self, d: char, now: i64) -> Vec<DigitsEvent> {
        let mut out = Vec::new();
        if self.disposed || !d.is_ascii_digit() {
            return out;
        }
        if !self.buf.is_empty() && now - self.at > self.timeout_ms {
            // Zeitgeber wurde gedrosselt: Eingabe ist abgelaufen
            if self.buf == "1" {
                self.commit_pending(&mut out);
            } else {
                self.reset(&mut out);
            }
        }
        if self.buf.is_empty() {
            match d {
                '0' => {
                    self.buf.push('0');
                    out.push(DigitsEvent::Preview(Some("0_".into())));
                    self.arm(now, Armed::Reset);
                }
                '1' => {
                    self.buf.push('1');
                    out.push(DigitsEvent::Preview(Some("1_".into())));
                    self.arm(now, Armed::Commit);
                }
                _ => out.push(DigitsEvent::Commit(d.to_digit(10).unwrap_or(0))),
            }
            return out;
        }
        let no: u32 = format!("{}{d}", self.buf).parse().unwrap_or(0);
        self.reset(&mut out);
        if (1..=MAX_CHANNEL).contains(&no) {
            out.push(DigitsEvent::Commit(no));
        } else {
            out.push(DigitsEvent::Invalid("Dieser Sender existiert nicht".into()));
        }
        out
    }

    /// Lässt abgelaufene Zeitgeber wirken.
    pub fn poll(&mut self, now: i64) -> Vec<DigitsEvent> {
        let mut out = Vec::new();
        if let Some((dl, kind)) = self.timer
            && now >= dl
        {
            self.timer = None;
            match kind {
                Armed::Reset => self.reset(&mut out),
                Armed::Commit => self.commit_pending(&mut out),
            }
        }
        out
    }

    /// Verwirft die Eingabe.
    pub fn clear(&mut self) -> Vec<DigitsEvent> {
        let mut out = Vec::new();
        if !self.disposed {
            self.reset(&mut out);
        }
        out
    }

    /// Beendet die Eingabe endgültig.
    pub fn dispose(&mut self) -> Vec<DigitsEvent> {
        let mut out = Vec::new();
        if !self.disposed {
            self.reset(&mut out);
            self.disposed = true;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use DigitsEvent::*;

    fn commits(ev: &[DigitsEvent]) -> Vec<u32> {
        ev.iter()
            .filter_map(|e| if let Commit(n) = e { Some(*n) } else { None })
            .collect()
    }

    #[test]
    fn zwei_bis_neun_sofort() {
        let mut d = Digits::default();
        for n in 2..=9u32 {
            let c = char::from_digit(n, 10).unwrap();
            assert_eq!(d.push(c, 0), vec![Commit(n)]);
            assert_eq!(d.pending(), "");
        }
    }

    #[test]
    fn eins_wartet_und_schaltet_nach_zeitablauf() {
        let mut d = Digits::default();
        assert_eq!(d.push('1', 0), vec![Preview(Some("1_".into()))]);
        assert_eq!(d.pending(), "1");
        assert!(d.poll(1499).is_empty());
        assert_eq!(d.poll(1500), vec![Preview(None), Commit(1)]);
        assert_eq!(d.pending(), "");
        assert_eq!(d.next_deadline(), None);
    }

    #[test]
    fn zweistellig_gueltig_bis_18() {
        for (second, ok) in [('0', true), ('8', true), ('9', false)] {
            let mut d = Digits::default();
            d.push('1', 0);
            let ev = d.push(second, 100);
            if ok {
                assert_eq!(commits(&ev).len(), 1);
            } else {
                assert!(ev.contains(&Invalid("Dieser Sender existiert nicht".into())));
            }
        }
        let mut d = Digits::default();
        d.push('1', 0);
        assert_eq!(commits(&d.push('8', 1)), vec![18]);
    }

    #[test]
    fn null_allein_verfaellt_null_plus_ziffer_einstellig() {
        let mut d = Digits::default();
        d.push('0', 0);
        assert_eq!(d.poll(1500), vec![Preview(None)]);
        d.push('0', 2000);
        assert_eq!(commits(&d.push('5', 2100)), vec![5]);
        d.push('0', 3000);
        let ev = d.push('0', 3100);
        assert!(ev.contains(&Invalid("Dieser Sender existiert nicht".into())));
    }

    #[test]
    fn eigenes_timeout_clear_dispose() {
        let mut d = Digits::new(300);
        d.push('1', 0);
        assert_eq!(d.next_deadline(), Some(300));
        assert_eq!(d.clear(), vec![Preview(None)]);
        assert_eq!(d.next_deadline(), None);
        d.push('1', 10);
        d.dispose();
        assert!(d.push('2', 20).is_empty());
    }

    #[test]
    fn abgelaufene_eingabe_zaehlt_nicht_als_zweite_ziffer() {
        let mut d = Digits::default();
        d.push('1', 0);
        // gedrosselter Zeitgeber: Zeit ist weit vorbei, push('2') darf nicht 12 ergeben
        let ev = d.push('2', 5000);
        assert_eq!(commits(&ev), vec![1, 2]);
        let mut d = Digits::default();
        d.push('0', 0);
        let ev = d.push('5', 5000);
        assert_eq!(commits(&ev), vec![5]);
    }

    #[test]
    fn nichtziffern_ignoriert() {
        let mut d = Digits::default();
        assert!(d.push('x', 0).is_empty());
        assert_eq!(d.pending(), "");
    }
}
