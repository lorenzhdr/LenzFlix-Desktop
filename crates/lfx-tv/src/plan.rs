//! Schnittstelle zum Planer: [`PlanSource`] ist die kleine Fläche, die der TV-Modus braucht.
//!
//! Das echte [`Scheduler`] implementiert sie; Tests nutzen eine Attrappe mit gebauten Plänen
//! (Füller, Sendepause, Nachtfüllung lassen sich mit dem echten Planer nicht gezielt erzeugen).

use std::rc::Rc;

use lfx_tv_schedule::catalog::Item;
use lfx_tv_schedule::jsval::js_trim;
use lfx_tv_schedule::{Channel, NowOn, PlanEntry, PlanError, Scheduler};
use unicode_normalization::UnicodeNormalization;

/// Art eines Katalogeintrags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemType {
    Movie,
    Episode,
    Series,
}

impl ItemType {
    /// Name wie im Web (`kind`).
    pub fn as_str(self) -> &'static str {
        match self {
            ItemType::Movie => "movie",
            ItemType::Episode => "episode",
            ItemType::Series => "series",
        }
    }
}

/// Normalisierte Sicht auf einen Katalogeintrag (Felder wie `CatalogItem` im Web).
#[derive(Debug, Clone, PartialEq)]
pub struct ItemInfo {
    pub id: String,
    pub kind: ItemType,
    pub title: String,
    pub series_title: Option<String>,
    pub series_id: Option<String>,
    /// Staffel; 9999 = unbekannt.
    pub season: Option<f64>,
    /// Folge; 9999 = unbekannt.
    pub episode: Option<f64>,
    pub year: Option<i64>,
    /// Normalisierte Genres (klein geschrieben).
    pub genres: Vec<String>,
    pub dur_ms: i64,
    /// Inhaltsangabe, falls schon bekannt (der Planer-Katalog trägt keine).
    pub overview: Option<String>,
}

/// Was der TV-Modus vom Planer braucht.
pub trait PlanSource {
    /// Eintrag zum Zeitpunkt `t` auf Sender `no`.
    fn now_on(&mut self, no: i64, t: i64) -> Result<NowOn, PlanError>;
    /// Plan eines Sendetags.
    fn plan_day(&mut self, no: i64, d: i64) -> Result<Rc<Vec<PlanEntry>>, PlanError>;
    /// Sender (18) für den Sendetag `d`.
    fn channels(&mut self, d: i64) -> Rc<Vec<Channel>>;
    /// Katalogeintrag.
    fn item(&self, id: &str) -> Option<ItemInfo>;
    /// Anzeigelabels der Genres mit Titelzahl.
    fn genres(&self) -> Vec<(String, i64)>;
}

impl PlanSource for Scheduler {
    fn now_on(&mut self, no: i64, t: i64) -> Result<NowOn, PlanError> {
        Scheduler::now_on(self, no, t)
    }

    fn plan_day(&mut self, no: i64, d: i64) -> Result<Rc<Vec<PlanEntry>>, PlanError> {
        Scheduler::plan_day(self, no, d)
    }

    fn channels(&mut self, d: i64) -> Rc<Vec<Channel>> {
        Scheduler::channels(self, d)
    }

    fn item(&self, id: &str) -> Option<ItemInfo> {
        Scheduler::item(self, id).map(|it| match it {
            Item::Movie(m) => ItemInfo {
                id: m.id.clone(),
                kind: ItemType::Movie,
                title: m.title.clone(),
                series_title: None,
                series_id: None,
                season: None,
                episode: None,
                year: m.year,
                genres: m.genres.clone(),
                dur_ms: m.dur_ms,
                overview: None,
            },
            Item::Episode(e) => ItemInfo {
                id: e.id.clone(),
                kind: ItemType::Episode,
                title: e.title.clone(),
                series_title: Some(e.series_title.clone()),
                series_id: Some(e.series_id.clone()),
                season: Some(e.season),
                episode: Some(e.episode),
                year: e.year,
                genres: e.genres.clone(),
                dur_ms: e.dur_ms,
                overview: None,
            },
            Item::Series(s) => ItemInfo {
                id: s.id.clone(),
                kind: ItemType::Series,
                title: s.title.clone(),
                series_title: None,
                series_id: None,
                season: None,
                episode: None,
                year: s.year,
                genres: s.genres.clone(),
                dur_ms: s.dur_ms,
                overview: None,
            },
        })
    }

    fn genres(&self) -> Vec<(String, i64)> {
        Scheduler::genres(self)
    }
}

/// Normalisierter Genre-Schlüssel eines Labels (`trim().normalize('NFC').toLowerCase()`).
pub fn genre_key(label: &str) -> String {
    js_trim(label).nfc().collect::<String>().to_lowercase()
}
