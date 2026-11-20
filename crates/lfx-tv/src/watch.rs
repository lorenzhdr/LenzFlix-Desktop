//! Gesehen-Regel (Port von `player/watch.js`).
//!
//! Eine Ausstrahlung gilt als gesehen, wenn (a) spätestens 10 min nach `entry.start` eingeschaltet
//! wurde und (b) die Sendeposition mindestens 90 % der Sendedauer erreicht hat. Unterbrechungen
//! (Wegzappen, Aus) unter 60 s sind erlaubt, ab 60 s ist die Ausstrahlung verworfen. Einmal pro
//! Ausstrahlung (`item_id` + `start`). Die Zeit kommt als Parameter; „als gesehen markieren“ liefert
//! der Tracker als Item-Id zurück, das Setzen übernimmt der Aufrufer.

use indexmap::IndexMap;
use lfx_tv_schedule::{EntryType, PlanEntry};

const LATE_MS: i64 = 10 * 60_000;
const GAP_MS: i64 = 60_000;
const MAX_RECORDS: usize = 60;

#[derive(Debug, Clone)]
struct Rec {
    ok: bool,
    left_at: Option<i64>,
    done: bool,
}

#[derive(Debug, Clone)]
struct Cur {
    key: String,
    start: i64,
    end: i64,
    item_id: String,
}

/// Zählt Ausstrahlungen als gesehen.
#[derive(Debug, Default)]
pub struct WatchTracker {
    records: IndexMap<String, Rec>,
    cur: Option<Cur>,
}

fn eligible(t: EntryType) -> bool {
    matches!(
        t,
        EntryType::Program | EntryType::Fill | EntryType::Nightfill
    )
}

impl WatchTracker {
    /// Leerer Tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Läuft gerade eine zählbare Ausstrahlung?
    pub fn is_tracking(&self) -> bool {
        self.cur.is_some()
    }

    fn check(&mut self, now: i64) -> Vec<String> {
        let mut out = Vec::new();
        let Some(cur) = &self.cur else { return out };
        let Some(rec) = self.records.get_mut(&cur.key) else {
            return out;
        };
        if rec.done || !rec.ok {
            return out;
        }
        let run = cur.end - cur.start;
        let pos = now.min(cur.end) - cur.start;
        if run > 0 && (pos as f64) >= 0.9 * run as f64 {
            rec.done = true;
            out.push(cur.item_id.clone());
        }
        out
    }

    /// Verlassen. `silent` = ohne Prüfung (Störbild/Ausfall: nie als gesehen werten).
    pub fn leave(&mut self, silent: bool, now: i64) -> Vec<String> {
        if self.cur.is_none() {
            return Vec::new();
        }
        let out = if silent { Vec::new() } else { self.check(now) };
        if let Some(cur) = self.cur.take()
            && let Some(rec) = self.records.get_mut(&cur.key)
        {
            rec.left_at = Some(now);
        }
        out
    }

    /// Einschalten (nach erfolgreichem Start). `offset_ms` = Position in der Sendung.
    pub fn tune(&mut self, entry: &PlanEntry, offset_ms: i64, now: i64) -> Vec<String> {
        let key = entry
            .item_id
            .as_ref()
            .map(|id| format!("{id}@{}", entry.start));
        if let (Some(c), Some(k)) = (&self.cur, &key)
            && &c.key == k
        {
            return Vec::new();
        }
        let out = self.leave(false, now);
        let Some(key) = key else { return out };
        if !eligible(entry.kind) {
            return out;
        }
        match self.records.get_mut(&key) {
            None => {
                self.records.insert(
                    key.clone(),
                    Rec {
                        ok: offset_ms <= LATE_MS,
                        left_at: None,
                        done: false,
                    },
                );
                if self.records.len() > MAX_RECORDS {
                    self.records.shift_remove_index(0);
                }
            }
            Some(rec) => {
                if let Some(l) = rec.left_at
                    && now - l >= GAP_MS
                {
                    rec.ok = false;
                }
                rec.left_at = None;
            }
        }
        if let Some(rec) = self.records.get_mut(&key) {
            rec.left_at = None;
        }
        self.cur = Some(Cur {
            key,
            start: entry.start,
            end: entry.end,
            item_id: entry.item_id.clone().unwrap_or_default(),
        });
        out
    }

    /// Takt (1/s): prüft die laufende Ausstrahlung.
    pub fn tick(&mut self, now: i64) -> Vec<String> {
        self.check(now)
    }

    /// Beendet das Zählen.
    pub fn dispose(&mut self) {
        self.cur = None;
        self.records.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60_000;

    fn entry(kind: EntryType, id: Option<&str>, start: i64, end: i64) -> PlanEntry {
        PlanEntry {
            start,
            end,
            kind,
            reason: None,
            item_id: id.map(str::to_string),
            series_id: None,
            rerun: false,
            premiere: None,
            theme: None,
            age: None,
            label: None,
        }
    }

    fn prog() -> PlanEntry {
        entry(EntryType::Program, Some("m1"), 0, 100 * MIN)
    }

    #[test]
    fn puenktlich_gesehen_bei_90_prozent_genau_einmal() {
        let mut w = WatchTracker::new();
        let e = prog();
        assert!(w.tune(&e, 0, 0).is_empty());
        assert!(w.tick(89 * MIN).is_empty());
        assert_eq!(w.tick(90 * MIN), vec!["m1".to_string()]);
        assert!(w.tick(95 * MIN).is_empty());
        assert!(w.leave(false, 99 * MIN).is_empty());
    }

    #[test]
    fn leave_bei_90_prozent_genuegt() {
        let mut w = WatchTracker::new();
        w.tune(&prog(), 0, 0);
        assert_eq!(w.leave(false, 91 * MIN), vec!["m1".to_string()]);
    }

    #[test]
    fn verspaetung_9_59_zaehlt_10_01_nicht() {
        let mut w = WatchTracker::new();
        w.tune(&prog(), 9 * MIN + 59_000, 9 * MIN + 59_000);
        assert_eq!(w.tick(95 * MIN).len(), 1);
        let mut w = WatchTracker::new();
        w.tune(&prog(), 10 * MIN + 1000, 10 * MIN + 1000);
        assert!(w.tick(95 * MIN).is_empty());
    }

    #[test]
    fn unterbrechung_59s_erlaubt_61s_verwirft() {
        let mut w = WatchTracker::new();
        let e = prog();
        w.tune(&e, 0, 0);
        w.leave(false, 10 * MIN);
        w.tune(&e, 0, 10 * MIN + 59_000);
        assert_eq!(w.tick(95 * MIN).len(), 1);

        let mut w = WatchTracker::new();
        w.tune(&e, 0, 0);
        w.leave(false, 10 * MIN);
        w.tune(&e, 0, 10 * MIN + 61_000);
        assert!(w.tick(95 * MIN).is_empty());
        // Verwurf bleibt nach erneutem Wegzappen bestehen
        w.leave(false, 96 * MIN);
        w.tune(&e, 0, 96 * MIN + 1000);
        assert!(w.tick(99 * MIN).is_empty());
    }

    #[test]
    fn stilles_verlassen_zaehlt_nie() {
        let mut w = WatchTracker::new();
        w.tune(&prog(), 0, 0);
        assert!(w.leave(true, 99 * MIN).is_empty());
    }

    #[test]
    fn typen() {
        let mut w = WatchTracker::new();
        for e in [
            entry(EntryType::Filler, Some("x"), 0, MIN),
            entry(EntryType::Closedown, None, 0, MIN),
            entry(EntryType::Program, None, 0, MIN),
        ] {
            w.tune(&e, 0, 0);
            assert!(!w.is_tracking());
        }
        for kind in [EntryType::Fill, EntryType::Nightfill, EntryType::Program] {
            w.tune(&entry(kind, Some("y"), 0, MIN), 0, 0);
            assert!(w.is_tracking());
            w.leave(true, 0);
        }
    }

    #[test]
    fn einmal_pro_ausstrahlung_neue_zaehlt_neu() {
        let mut w = WatchTracker::new();
        let a = entry(EntryType::Program, Some("m1"), 0, 10 * MIN);
        w.tune(&a, 0, 0);
        assert_eq!(w.tick(10 * MIN).len(), 1);
        w.leave(false, 10 * MIN);
        w.tune(&a, 0, 10 * MIN + 1);
        assert!(w.tick(10 * MIN).is_empty());
        let b = entry(
            EntryType::Program,
            Some("m1"),
            24 * 60 * MIN,
            24 * 60 * MIN + 10 * MIN,
        );
        w.tune(&b, 0, 24 * 60 * MIN);
        assert_eq!(w.tick(24 * 60 * MIN + 10 * MIN).len(), 1);
    }

    #[test]
    fn dispose_beendet_das_zaehlen() {
        let mut w = WatchTracker::new();
        w.tune(&prog(), 0, 0);
        w.dispose();
        assert!(w.tick(99 * MIN).is_empty());
    }

    #[test]
    fn hoechstens_60_eintraege() {
        let mut w = WatchTracker::new();
        for i in 0..70 {
            w.tune(
                &entry(EntryType::Program, Some("m"), i * 1000, i * 1000 + 500),
                0,
                i * 1000,
            );
        }
        assert!(w.records.len() <= MAX_RECORDS);
    }
}
