//! Katalog-Normalisierung (Rohdaten des Servers zu `Catalog`). Rein, deterministisch und
//! unabhängig von der Eingabereihenfolge.
//!
//! Die Rohdaten kommen als [`JsVal`] (JSON des Servers im Feldumfang von `KEEP` aus
//! `features/tvdata/data.js`); alle Umwandlungen folgen den JS-Regeln des Originals.

use std::cmp::Ordering;
use std::sync::Arc;

use indexmap::IndexMap;
use unicode_normalization::UnicodeNormalization;

use crate::hash::cmp_str;
use crate::jsval::{JsVal, js_number, js_string, js_string_or_empty, js_trim, json_stringify};
use crate::rating::{Age, max_age, rating_age};
use crate::time::{HOUR, MIN, parse_date, ticks_to_ms, year_of_ms};

const MIN_DUR: i64 = 5 * MIN;
const MAX_DUR: i64 = 5 * HOUR;

/// Spielfilm.
#[derive(Debug, Clone, PartialEq)]
pub struct Movie {
    pub id: String,
    pub title: String,
    pub dur_ms: i64,
    /// Normalisierte Genres (aufsteigend nach Code-Units).
    pub genres: Vec<String>,
    pub year: Option<i64>,
    pub premiere: Option<f64>,
    pub added_at: f64,
    pub age: Age,
    pub trailer: bool,
}

/// Serienfolge.
#[derive(Debug, Clone, PartialEq)]
pub struct Episode {
    pub id: String,
    pub title: String,
    pub dur_ms: i64,
    pub series_id: String,
    pub series_title: String,
    pub season: f64,
    pub episode: f64,
    /// Fehlendes Datum = `+Infinity`.
    pub premiere: f64,
    pub added_at: f64,
    pub age: Age,
    pub genres: Vec<String>,
    pub year: Option<i64>,
}

/// Serie mit ihren planbaren Folgen.
#[derive(Debug, Clone, PartialEq)]
pub struct Series {
    pub id: String,
    pub title: String,
    pub genres: Vec<String>,
    pub year: Option<i64>,
    pub added_at: f64,
    /// Nur die eigene Freigabe.
    pub age: Age,
    /// Strengste Freigabe inklusive Folgen.
    pub age_max: Age,
    pub eps: Vec<Arc<Episode>>,
    pub ep_added_sorted: Vec<f64>,
    pub first_ep_added: f64,
    pub total_ms: i64,
    /// Unterer Median der Folgenlängen.
    pub dur_ms: i64,
}

/// Eintrag in `Catalog::items`.
#[derive(Debug, Clone)]
pub enum Item {
    Movie(Arc<Movie>),
    Episode(Arc<Episode>),
    Series(Arc<Series>),
}

impl Item {
    /// `addedAt` des Eintrags.
    pub fn added_at(&self) -> f64 {
        match self {
            Item::Movie(m) => m.added_at,
            Item::Episode(e) => e.added_at,
            Item::Series(s) => s.added_at,
        }
    }
}

/// Filmreihe (Sammlung).
#[derive(Debug, Clone, PartialEq)]
pub struct Boxset {
    pub id: String,
    pub name: String,
    pub child_count: f64,
}

/// Genre mit Anzeigelabel und Titelanzahl.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Genre {
    pub label: String,
    pub count: i64,
    pub norm: String,
}

/// Normalisierter Katalog.
#[derive(Debug, Clone)]
pub struct Catalog {
    /// Nach Id sortiert.
    pub movies: Vec<Arc<Movie>>,
    /// Nach Id sortiert, nur Serien mit planbaren Folgen.
    pub series: Vec<Arc<Series>>,
    /// In Einfügereihenfolge (wie die JS-`Map`).
    pub items: IndexMap<String, Item>,
    pub boxsets: Vec<Boxset>,
    pub genres: Vec<Genre>,
    labels: IndexMap<String, String>,
    pub loaded_at: f64,
}

impl Catalog {
    /// Anzeigelabel zu einem normalisierten Genre (Rückfall: das Genre selbst).
    pub fn genre_label(&self, norm: &str) -> String {
        self.labels
            .get(norm)
            .cloned()
            .unwrap_or_else(|| norm.to_string())
    }

    /// Eintrag zur Id.
    pub fn item(&self, id: &str) -> Option<&Item> {
        self.items.get(id)
    }
}

/// Genre normalisieren (Vergleichsschlüssel).
pub fn norm_genre(g: &JsVal) -> String {
    js_trim(&js_string_or_empty(g))
        .nfc()
        .collect::<String>()
        .to_lowercase()
}

/// Unterer Median (ganzzahlig); `arr` aufsteigend sortiert.
pub fn lower_median(arr: &[i64]) -> i64 {
    if arr.is_empty() {
        0
    } else {
        arr[(arr.len() - 1) / 2]
    }
}

/// `Number(v)`, nur endliche Werte.
fn num(v: &JsVal) -> Option<f64> {
    let n = js_number(v);
    n.is_finite().then_some(n)
}

fn year_of(production_year: &JsVal, premiere: Option<f64>) -> Option<i64> {
    if let Some(y) = num(production_year)
        && y != 0.0
        && y > 1800.0
        && y < 3000.0
    {
        return Some(y.floor() as i64);
    }
    match premiere {
        Some(p) if p.is_finite() => Some(year_of_ms(p)),
        _ => None,
    }
}

fn genres_of(
    list: &JsVal,
    label_counts: &mut IndexMap<String, IndexMap<String, i64>>,
) -> Vec<String> {
    let mut norm: Vec<String> = Vec::new();
    for g in list.as_array() {
        let label: String = js_trim(&js_string_or_empty(g)).nfc().collect();
        let n = label.to_lowercase();
        if n.is_empty() || norm.contains(&n) {
            continue;
        }
        norm.push(n.clone());
        *label_counts.entry(n).or_default().entry(label).or_insert(0) += 1;
    }
    norm.sort_by(|a, b| cmp_str(a, b));
    norm
}

/// Rohe Einträge mit eindeutigen Ids; bei Doppelten gewinnt der in Code-Unit-Reihenfolge
/// kleinere Roh-Eintrag (JSON).
fn dedupe(list: &JsVal) -> Vec<(&JsVal, String)> {
    let mut arr: Vec<(&JsVal, String, String)> = list
        .as_array()
        .iter()
        .filter(|x| matches!(x, JsVal::Obj(_)))
        .filter(|x| {
            let id = x.get("Id");
            !id.is_nullish() && *id != JsVal::Str(String::new())
        })
        .map(|x| (x, js_string(x.get("Id")), json_stringify(x)))
        .collect();
    arr.sort_by(|a, b| cmp_str(&a.1, &b.1).then_with(|| cmp_str(&a.2, &b.2)));
    let mut out: Vec<(&JsVal, String)> = Vec::new();
    for (x, id, _) in arr {
        if out.last().is_none_or(|l| l.1 != id) {
            out.push((x, id));
        }
    }
    out
}

struct RawEpisode {
    id: String,
    title: String,
    dur_ms: Option<i64>,
    series_id: String,
    series_title: String,
    season: f64,
    episode: f64,
    premiere: f64,
    added_at: f64,
    own_age: Age,
}

fn cmp_ep(a: &RawEpisode, b: &RawEpisode) -> Ordering {
    let f = |x: f64, y: f64| x.partial_cmp(&y).unwrap_or(Ordering::Equal);
    f(a.season, b.season)
        .then_with(|| f(a.episode, b.episode))
        .then_with(|| f(a.premiere, b.premiere))
        .then_with(|| cmp_str(&a.id, &b.id))
}

/// `normalizeCatalog(raw)`.
pub fn normalize_catalog(raw: &JsVal) -> Catalog {
    let mut label_counts: IndexMap<String, IndexMap<String, i64>> = IndexMap::new();
    let mut items: IndexMap<String, Item> = IndexMap::new();

    let mut movies: Vec<Arc<Movie>> = Vec::new();
    for (x, id) in dedupe(raw.get("movies")) {
        let dur = ticks_to_ms(x.get("RunTimeTicks"));
        if !(dur >= MIN_DUR as f64 && dur <= MAX_DUR as f64) {
            continue;
        }
        let premiere = parse_date(x.get("PremiereDate"));
        let it = Arc::new(Movie {
            id: id.clone(),
            title: js_string_or_empty(x.get("Name")),
            dur_ms: dur as i64,
            genres: genres_of(x.get("Genres"), &mut label_counts),
            year: year_of(x.get("ProductionYear"), premiere),
            premiere,
            added_at: parse_date(x.get("DateCreated")).unwrap_or(0.0),
            age: rating_age(x.get("OfficialRating")),
            trailer: num(x.get("LocalTrailerCount")).unwrap_or(0.0) > 0.0,
        });
        movies.push(it.clone());
        items.insert(id, Item::Movie(it));
    }

    let raw_series = dedupe(raw.get("series"));
    let mut eps_by: IndexMap<String, Vec<RawEpisode>> = IndexMap::new();
    for (x, id) in dedupe(raw.get("episodes")) {
        let sid = if x.get("SeriesId").is_nullish() {
            String::new()
        } else {
            js_string(x.get("SeriesId"))
        };
        let Some(series_raw) = raw_series.iter().find(|(_, i)| *i == sid) else {
            continue;
        };
        let season = num(x.get("ParentIndexNumber"));
        if season == Some(0.0) {
            continue; // Staffel 0 (Extras) raus
        }
        let rt = x.get("RunTimeTicks");
        let has_rt = !rt.is_nullish() && js_number(rt) > 0.0;
        let dur = if has_rt { Some(ticks_to_ms(rt)) } else { None };
        if let Some(d) = dur
            && !(d >= MIN_DUR as f64 && d <= MAX_DUR as f64)
        {
            continue;
        }
        let series_name = x.get("SeriesName");
        let series_title = if series_name.is_nullish() {
            js_string_or_empty(series_raw.0.get("Name"))
        } else {
            js_string(series_name)
        };
        eps_by.entry(sid.clone()).or_default().push(RawEpisode {
            id,
            title: js_string_or_empty(x.get("Name")),
            dur_ms: dur.map(|d| d as i64),
            series_id: sid,
            series_title,
            season: season.unwrap_or(9999.0),
            episode: num(x.get("IndexNumber")).unwrap_or(9999.0),
            premiere: parse_date(x.get("PremiereDate")).unwrap_or(f64::INFINITY),
            added_at: parse_date(x.get("DateCreated")).unwrap_or(0.0),
            own_age: rating_age(x.get("OfficialRating")),
        });
    }

    let mut series: Vec<Arc<Series>> = Vec::new();
    for (x, id) in &raw_series {
        let mut eps: Vec<RawEpisode> = eps_by.swap_remove(id).unwrap_or_default();
        let mut durs: Vec<i64> = eps.iter().filter_map(|e| e.dur_ms).collect();
        durs.sort_unstable();
        let med = lower_median(&durs);
        let mut keep: Vec<RawEpisode> = if durs.is_empty() {
            Vec::new()
        } else {
            for e in &mut eps {
                if e.dur_ms.is_none() {
                    e.dur_ms = Some(med);
                }
            }
            eps
        };
        keep.sort_by(cmp_ep);
        let own = rating_age(x.get("OfficialRating"));
        let genres = genres_of(x.get("Genres"), &mut label_counts);
        let year = year_of(x.get("ProductionYear"), None);
        let mut age_max = own;
        let mut done: Vec<Arc<Episode>> = Vec::with_capacity(keep.len());
        for e in keep {
            let age = max_age(own, e.own_age);
            age_max = max_age(age_max, age);
            let ep = Arc::new(Episode {
                id: e.id,
                title: e.title,
                dur_ms: e.dur_ms.unwrap_or(0),
                series_id: e.series_id,
                series_title: e.series_title,
                season: e.season,
                episode: e.episode,
                premiere: e.premiere,
                added_at: e.added_at,
                age,
                genres: genres.clone(),
                year,
            });
            items.insert(ep.id.clone(), Item::Episode(ep.clone()));
            done.push(ep);
        }
        let mut added: Vec<f64> = done.iter().map(|e| e.added_at).collect();
        added.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
        let s = Arc::new(Series {
            id: id.clone(),
            title: js_string_or_empty(x.get("Name")),
            genres,
            year,
            added_at: parse_date(x.get("DateCreated")).unwrap_or(0.0),
            age: own,
            age_max,
            first_ep_added: added.first().copied().unwrap_or(0.0),
            total_ms: done.iter().map(|e| e.dur_ms).sum(),
            dur_ms: med,
            ep_added_sorted: added,
            eps: done,
        });
        items.insert(id.clone(), Item::Series(s.clone()));
        if !s.eps.is_empty() {
            series.push(s);
        }
    }

    let boxsets: Vec<Boxset> = dedupe(raw.get("boxsets"))
        .into_iter()
        .map(|(x, id)| Boxset {
            id,
            name: js_string_or_empty(x.get("Name")),
            child_count: num(x.get("ChildCount"))
                .filter(|c| *c != 0.0)
                .unwrap_or(0.0),
        })
        .collect();

    // Genre-Zählung und Anzeigelabel (häufigste Originalschreibweise, Gleichstand: kleinste Code-Unit-Folge)
    let mut labels: IndexMap<String, String> = IndexMap::new();
    for (n, m) in &label_counts {
        let mut best: Option<&String> = None;
        let mut bc = -1i64;
        for (lab, c) in m {
            let better =
                *c > bc || (*c == bc && best.is_some_and(|b| cmp_str(lab, b) == Ordering::Less));
            if better {
                best = Some(lab);
                bc = *c;
            }
        }
        if let Some(b) = best {
            labels.insert(n.clone(), b.clone());
        }
    }
    let mut counts: IndexMap<String, i64> = IndexMap::new();
    for m in &movies {
        for g in &m.genres {
            *counts.entry(g.clone()).or_insert(0) += 1;
        }
    }
    for s in &series {
        for g in &s.genres {
            *counts.entry(g.clone()).or_insert(0) += 1;
        }
    }
    let mut genres: Vec<Genre> = counts
        .into_iter()
        .map(|(n, count)| Genre {
            label: labels.get(&n).cloned().unwrap_or_default(),
            count,
            norm: n,
        })
        .collect();
    genres.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| cmp_str(&a.norm, &b.norm))
    });

    Catalog {
        movies,
        series,
        items,
        boxsets,
        genres,
        labels,
        loaded_at: num(raw.get("loadedAt")).unwrap_or(0.0),
    }
}

/// Planbarer Titel: Film oder Folge.
#[derive(Debug, Clone)]
pub enum PlayItem {
    Movie(Arc<Movie>),
    Episode(Arc<Episode>),
}

impl PlayItem {
    pub fn id(&self) -> &str {
        match self {
            PlayItem::Movie(m) => &m.id,
            PlayItem::Episode(e) => &e.id,
        }
    }
    pub fn dur_ms(&self) -> i64 {
        match self {
            PlayItem::Movie(m) => m.dur_ms,
            PlayItem::Episode(e) => e.dur_ms,
        }
    }
    pub fn age(&self) -> Age {
        match self {
            PlayItem::Movie(m) => m.age,
            PlayItem::Episode(e) => e.age,
        }
    }
    pub fn series_id(&self) -> Option<&str> {
        match self {
            PlayItem::Movie(_) => None,
            PlayItem::Episode(e) => Some(&e.series_id),
        }
    }
    pub fn year(&self) -> Option<i64> {
        match self {
            PlayItem::Movie(m) => m.year,
            PlayItem::Episode(e) => e.year,
        }
    }
    /// Premierendatum; fehlend = `+Infinity`.
    pub fn premiere(&self) -> f64 {
        match self {
            PlayItem::Movie(m) => m.premiere.unwrap_or(f64::INFINITY),
            PlayItem::Episode(e) => e.premiere,
        }
    }
    /// Staffel (Filme: 0).
    pub fn season(&self) -> f64 {
        match self {
            PlayItem::Movie(_) => 0.0,
            PlayItem::Episode(e) => e.season,
        }
    }
    /// Folgennummer (Filme: 0).
    pub fn episode(&self) -> f64 {
        match self {
            PlayItem::Movie(_) => 0.0,
            PlayItem::Episode(e) => e.episode,
        }
    }
    pub fn added_at(&self) -> f64 {
        match self {
            PlayItem::Movie(m) => m.added_at,
            PlayItem::Episode(e) => e.added_at,
        }
    }
}
