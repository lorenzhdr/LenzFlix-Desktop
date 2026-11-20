//! Serien-Sichten (nur Folgen mit `added_at` vor dem Stichtag) und Folgen-Zeiger („Durchlauf“-Regel).

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use crate::catalog::{Episode, Series, lower_median};
use crate::rating::Age;
use crate::time::{GRID, MIN, admit_day, wall_to_utc};

/// Sicht auf eine Serie zum Stichtag.
#[derive(Debug, Clone)]
pub struct View {
    pub s: Arc<Series>,
    pub n: usize,
    /// Kanonisch sortiert, nur verfügbare Folgen.
    pub eps: Vec<Arc<Episode>>,
    pub median_ms: i64,
    pub p90_ms: i64,
    /// Strengste Freigabe.
    pub age: Age,
    /// Folgen je Auftritt (1 oder 2).
    pub b: usize,
    /// Länge eines Auftritts.
    pub auf_ms: i64,
    /// Feste Sendeplatzlänge (Sender 3).
    pub slot_len: i64,
    pub total_ms: i64,
    /// Erster Sendetag im Pool.
    pub admit: i64,
}

#[derive(Debug, Default)]
struct Walk {
    next: i64,
    cursor: usize,
    blocks: Vec<Vec<usize>>,
}

/// Zwischenspeicher für Sichten, Sendetagsbeginne und Folgen-Wanderungen.
#[derive(Debug, Default)]
pub struct SeriesTools {
    views: HashMap<String, Rc<View>>,
    starts: HashMap<i64, i64>,
    walks: HashMap<String, Walk>,
}

impl SeriesTools {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sendetagsbeginn (ms) mit Cache.
    pub fn day_start(&mut self, d: i64) -> i64 {
        *self.starts.entry(d).or_insert_with(|| wall_to_utc(d, 300))
    }

    /// Sicht auf eine Serie zum Stichtag `cut` (= `day_start(D)`); `None`, wenn die Serie dann
    /// nicht im Pool ist.
    pub fn view(&mut self, s: &Arc<Series>, cut: i64) -> Option<Rc<View>> {
        let cutf = cut as f64;
        if s.added_at >= cutf {
            return None;
        }
        let a = &s.ep_added_sorted;
        let lo = a.partition_point(|x| *x < cutf);
        if lo == 0 {
            return None;
        }
        let key = format!("{}|{}", s.id, lo);
        if let Some(v) = self.views.get(&key) {
            return Some(v.clone());
        }
        let eps: Vec<Arc<Episode>> = s
            .eps
            .iter()
            .filter(|e| e.added_at < cutf)
            .cloned()
            .collect();
        let mut durs: Vec<i64> = eps.iter().map(|e| e.dur_ms).collect();
        durs.sort_unstable();
        let n = durs.len();
        let median_ms = lower_median(&durs);
        let p90_ms = durs[(9 * n).div_ceil(10) - 1];
        let mut age = s.age;
        let mut total_ms = 0;
        for e in &eps {
            if let Some(ea) = e.age
                && age.is_none_or(|x| ea > x)
            {
                age = Some(ea);
            }
            total_ms += e.dur_ms;
        }
        let b = n.min(if median_ms <= 30 * MIN { 2 } else { 1 });
        let slot_len = ((p90_ms * b as i64) as f64 / GRID as f64).ceil() as i64 * GRID;
        let admit = admit_day(s.added_at.max(s.first_ep_added) as i64);
        let v = Rc::new(View {
            s: s.clone(),
            n,
            eps,
            median_ms,
            p90_ms,
            age,
            b,
            auf_ms: b as i64 * median_ms,
            slot_len,
            total_ms,
            admit,
        });
        self.views.insert(key, v.clone());
        Some(v)
    }

    /// Folgen des Auftritts `k` (`b` Folgen). Zeiger-Simulation: bei Auftritt k
    /// (Verfügbarkeitstag `day_of_k(k)`) wird je Folge die nächste verfügbare ab dem Zeiger
    /// gesendet; gibt es keine mehr, endet der Durchlauf (Zeiger zurück auf 0).
    /// `wkey` muss alle Parameter von `day_of_k` und `b` eindeutig kodieren.
    pub fn block(
        &mut self,
        s: &Arc<Series>,
        wkey: &str,
        k: i64,
        b: usize,
        day_of_k: &dyn Fn(i64) -> i64,
    ) -> Vec<Arc<Episode>> {
        let eps = &s.eps;
        while self.walks.entry(wkey.to_string()).or_default().next <= k {
            let next = self.walks[wkey].next;
            let cut_k = self.day_start(day_of_k(next)) as f64;
            let w = self.walks.get_mut(wkey).expect("Wanderung angelegt");
            let mut picks = Vec::new();
            for _ in 0..b {
                let mut j: Option<usize> = None;
                let mut pass = 0;
                while pass < 2 && j.is_none() {
                    j = eps
                        .iter()
                        .enumerate()
                        .skip(w.cursor)
                        .find(|(_, e)| e.added_at < cut_k)
                        .map(|(x, _)| x);
                    if j.is_none() {
                        w.cursor = 0;
                    }
                    pass += 1;
                }
                if let Some(j) = j {
                    w.cursor = j + 1;
                    picks.push(j);
                }
            }
            w.blocks.push(picks);
            w.next += 1;
        }
        let w = &self.walks[wkey];
        let mut seen: HashSet<usize> = HashSet::new();
        let mut out = Vec::new();
        for &j in w
            .blocks
            .get(usize::try_from(k).unwrap_or(usize::MAX))
            .map(Vec::as_slice)
            .unwrap_or(&[])
        {
            if seen.insert(j) {
                out.push(eps[j].clone());
            }
        }
        out
    }
}
