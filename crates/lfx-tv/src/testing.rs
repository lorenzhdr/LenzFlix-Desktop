//! Testhilfen: Attrappe des Planers mit gebauten Plänen (erfundene Titel), damit Füller,
//! Sendepause und Nachtfüllung gezielt prüfbar sind. Nicht für die App gedacht.

use std::collections::HashMap;
use std::rc::Rc;

use lfx_tv_schedule::time::broadcast_day_of;
use lfx_tv_schedule::{Channel, EntryType, NowOn, PlanEntry, PlanError};

use crate::plan::{ItemInfo, ItemType, PlanSource};

/// Planer-Attrappe.
#[derive(Default)]
pub struct FakePlan {
    entries: HashMap<i64, Vec<PlanEntry>>,
    items: HashMap<String, ItemInfo>,
    genres: Vec<(String, i64)>,
    /// Anzahl der `now_on`-Aufrufe (für Tests).
    pub now_on_calls: usize,
}

/// Eintrag bauen.
pub fn entry(kind: EntryType, item: Option<&str>, start: i64, end: i64) -> PlanEntry {
    PlanEntry {
        start,
        end,
        kind,
        reason: None,
        item_id: item.map(str::to_string),
        series_id: None,
        rerun: false,
        premiere: None,
        theme: None,
        age: None,
        label: None,
    }
}

/// Programmeintrag.
pub fn program(item: &str, start: i64, end: i64) -> PlanEntry {
    entry(EntryType::Program, Some(item), start, end)
}

/// Füller.
pub fn filler(start: i64, end: i64) -> PlanEntry {
    entry(EntryType::Filler, None, start, end)
}

/// Sendepause.
pub fn closedown(start: i64, end: i64, reason: Option<&str>) -> PlanEntry {
    let mut e = entry(EntryType::Closedown, None, start, end);
    e.reason = reason.map(str::to_string);
    e
}

/// Film als Katalogeintrag.
pub fn movie(id: &str, title: &str, dur_ms: i64) -> ItemInfo {
    ItemInfo {
        id: id.into(),
        kind: ItemType::Movie,
        title: title.into(),
        series_title: None,
        series_id: None,
        season: None,
        episode: None,
        year: Some(1999),
        genres: vec!["komödie".into()],
        dur_ms,
        overview: None,
    }
}

/// Folge als Katalogeintrag.
pub fn episode(id: &str, series: &str, title: &str, s: f64, e: f64, series_id: &str) -> ItemInfo {
    ItemInfo {
        id: id.into(),
        kind: ItemType::Episode,
        title: title.into(),
        series_title: Some(series.into()),
        series_id: Some(series_id.into()),
        season: Some(s),
        episode: Some(e),
        year: Some(2005),
        genres: vec!["drama".into()],
        dur_ms: 45 * 60_000,
        overview: None,
    }
}

impl FakePlan {
    /// Leere Attrappe.
    pub fn new() -> Self {
        Self::default()
    }

    /// Plan eines Senders (lückenlos, nach Beginn sortiert).
    pub fn set(&mut self, no: i64, entries: Vec<PlanEntry>) -> &mut Self {
        self.entries.insert(no, entries);
        self
    }

    /// Katalogeintrag.
    pub fn item_info(&mut self, it: ItemInfo) -> &mut Self {
        self.items.insert(it.id.clone(), it);
        self
    }

    /// Genre-Label.
    pub fn genre(&mut self, label: &str, count: i64) -> &mut Self {
        self.genres.push((label.to_string(), count));
        self
    }

    /// Alle 18 Sender mit dem gleichen Plan füllen.
    pub fn all(&mut self, entries: &[PlanEntry]) -> &mut Self {
        for no in 1..=18 {
            self.entries.insert(no, entries.to_vec());
        }
        self
    }
}

impl PlanSource for FakePlan {
    fn now_on(&mut self, no: i64, t: i64) -> Result<NowOn, PlanError> {
        self.now_on_calls += 1;
        let list = self.entries.get(&no).ok_or(PlanError)?;
        let e = list
            .iter()
            .find(|e| e.start <= t && t < e.end)
            .ok_or(PlanError)?;
        Ok(NowOn {
            entry: e.clone(),
            offset_ms: t - e.start,
            d: broadcast_day_of(t),
        })
    }

    fn plan_day(&mut self, no: i64, _d: i64) -> Result<Rc<Vec<PlanEntry>>, PlanError> {
        self.entries
            .get(&no)
            .map(|l| Rc::new(l.clone()))
            .ok_or(PlanError)
    }

    fn channels(&mut self, _d: i64) -> Rc<Vec<Channel>> {
        Rc::new(
            (1..=18)
                .map(|no| Channel {
                    no,
                    key: format!("c{no}"),
                    name: format!("LIVE {no}"),
                    kind: "genre",
                    genre_label: None,
                })
                .collect(),
        )
    }

    fn item(&self, id: &str) -> Option<ItemInfo> {
        self.items.get(id).cloned()
    }

    fn genres(&self) -> Vec<(String, i64)> {
        self.genres.clone()
    }
}
