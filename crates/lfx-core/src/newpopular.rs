//! „Neu und beliebt“ und Neu-Erkennung (Port von `features/newpopular.js`, `newpopular/format.js`,
//! `shared/newcontent.js`).

use crate::call::{ApiCall, Endpoint};
use crate::rows::items_call;
use crate::rows::logic::{NEW_DAYS, is_recent};
use crate::util::{DAY_MS, parse_opt};
use lfx_api::{BaseItem, ItemKind, ItemsQuery, SortBy, SortOrder};
use std::collections::{HashMap, HashSet};

/// Mindestzahl Chart-Titel für eine Sektion.
pub const MIN_CHART_TITLES: usize = 3;
/// Länge der „Neu bei …“-Liste und von „Am häufigsten geschaut“.
pub const LIST_LIMIT: u32 = 20;

/// Sektionen der Seite.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    /// Neu bei LENZFLIX.
    New,
    /// Am häufigsten geschaut.
    Watched,
    /// Top 10 Serien.
    TopSeries,
    /// Top 10 Filme.
    TopMovies,
}

impl Section {
    /// Reihenfolge der Seite.
    pub const ALL: [Section; 4] = [Self::New, Self::Watched, Self::TopSeries, Self::TopMovies];

    /// Kennung (`id`).
    pub fn id(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Watched => "watched",
            Self::TopSeries => "top-series",
            Self::TopMovies => "top-movies",
        }
    }

    /// Überschrift (`brand` = Markenname).
    pub fn label(self, brand: &str) -> String {
        match self {
            Self::New => format!("Neu bei {brand}"),
            Self::Watched => "Am häufigsten geschaut".into(),
            Self::TopSeries => "Top 10 Serien".into(),
            Self::TopMovies => "Top 10 Filme".into(),
        }
    }
}

/// Untertitel unter Chart-Sektionen.
pub fn chart_subtitle(brand: &str, window_days: Option<u32>) -> String {
    format!(
        "Meistgesehen auf {brand} · letzte {} Tage",
        window_days.filter(|d| *d > 0).unwrap_or(30)
    )
}

/// Ersatz-Untertitel ohne Charts („Neueste Serien/Filme“).
pub fn fallback_subtitle(section: Section) -> Option<&'static str> {
    match section {
        Section::TopSeries => Some("Neueste Serien"),
        Section::TopMovies => Some("Neueste Filme"),
        _ => None,
    }
}

fn np_call(types: &[ItemKind], extra: impl FnOnce(ItemsQuery) -> ItemsQuery) -> ApiCall {
    items_call(types, extra)
}

const BOTH: [ItemKind; 2] = [ItemKind::Movie, ItemKind::Series];

/// „Neu bei …“: die 20 neuesten.
pub fn new_call() -> ApiCall {
    np_call(&BOTH, |q| {
        q.sort_by(&[SortBy::DateCreated])
            .order(SortOrder::Descending)
            .limit(LIST_LIMIT)
    })
}

/// „Am häufigsten geschaut“, erster Versuch: nach `PlayCount` (pro Benutzer).
pub fn most_watched_call() -> ApiCall {
    np_call(&BOTH, |q| {
        q.sort_by(&[SortBy::PlayCount])
            .order(SortOrder::Descending)
            .limit(LIST_LIMIT)
    })
}

/// Rückfall, wenn alle Zähler 0 sind: nach `CommunityRating`.
pub fn most_watched_fallback_call() -> ApiCall {
    np_call(&BOTH, |q| {
        q.sort_by(&[SortBy::CommunityRating])
            .order(SortOrder::Descending)
            .limit(LIST_LIMIT)
    })
}

/// Rückfall der Top-10-Sektionen ohne Charts: die 10 neuesten der Art.
pub fn top_call(kind: ItemKind) -> ApiCall {
    np_call(&[kind], |q| {
        q.sort_by(&[SortBy::DateCreated])
            .order(SortOrder::Descending)
            .limit(10)
    })
}

/// Cache-Schlüssel einer Abfrage (`home:<user>:np:<Parameter>`; gleiche Invalidierung wie die Startseite).
pub fn cache_key(user_id: &str, call: &ApiCall) -> String {
    let params: Vec<String> = call
        .params
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    crate::cache::home_key(user_id, &["np", &params.join("&")])
}

/// Ergebnis der „Am häufigsten geschaut“-Wahl.
#[derive(Debug, Clone, PartialEq)]
pub enum WatchedPlan {
    /// Echte Charts (mindestens 3 Titel).
    Charts,
    /// Eigene `PlayCount`.
    PlayCount,
    /// Zähler alle 0: Bewertung.
    Rating,
}

/// Wahl nach der Web-Regel: Charts, sonst `PlayCount` (wenn mindestens ein Zähler > 0), sonst Bewertung.
pub fn watched_plan(chart_titles: usize, by_plays: &[BaseItem]) -> WatchedPlan {
    if chart_titles >= MIN_CHART_TITLES {
        WatchedPlan::Charts
    } else if has_play_counts(by_plays) {
        WatchedPlan::PlayCount
    } else {
        WatchedPlan::Rating
    }
}

/// Chart-Sektion gilt nur mit mindestens 3 Titeln.
pub fn chart_usable(titles: usize) -> bool {
    titles >= MIN_CHART_TITLES
}

/// Mindestens ein Titel hat `PlayCount > 0`.
pub fn has_play_counts(items: &[BaseItem]) -> bool {
    items
        .iter()
        .any(|i| i.user_data.as_ref().is_some_and(|u| u.play_count > 0))
}

/// Dedupliziert und begrenzt auf Filme/Serien.
pub fn unique_titles(items: &[BaseItem], limit: usize) -> Vec<BaseItem> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for it in items {
        if it.id.is_empty()
            || !matches!(it.kind, ItemKind::Movie | ItemKind::Series)
            || !seen.insert(it.id.as_str())
        {
            continue;
        }
        out.push(it.clone());
        if out.len() >= limit {
            break;
        }
    }
    out
}

// ---------------------------------------------------------------- Datum

const MONTHS_ABBR: [&str; 12] = [
    "JAN", "FEB", "MÄR", "APR", "MAI", "JUN", "JUL", "AUG", "SEP", "OKT", "NOV", "DEZ",
];
const MONTHS_LONG: [&str; 12] = [
    "Januar",
    "Februar",
    "März",
    "April",
    "Mai",
    "Juni",
    "Juli",
    "August",
    "September",
    "Oktober",
    "November",
    "Dezember",
];

fn local_ymd(ms: f64, utc_offset_min: i64) -> (i64, usize, i64) {
    let local = ms as i64 + utc_offset_min * 60_000;
    let (y, m, d) = lfx_tv_schedule::time::civil_from_days(local.div_euclid(86_400_000));
    (y, (m - 1) as usize, d)
}

/// ISO-Datum → (`NOV`, `08`) in lokaler Zeit (`utc_offset_min` Minuten östlich von UTC) oder `None`.
pub fn date_parts(iso: Option<&str>, utc_offset_min: i64) -> Option<(&'static str, String)> {
    let (_, m, d) = local_ymd(parse_opt(iso)?, utc_offset_min);
    Some((MONTHS_ABBR[m], format!("{d:02}")))
}

/// ISO-Datum → „8. November 2026“ (wie `toLocaleDateString('de-DE', …)`) oder leer.
pub fn long_date_de(iso: Option<&str>, utc_offset_min: i64) -> String {
    match parse_opt(iso) {
        Some(t) => {
            let (y, m, d) = local_ymd(t, utc_offset_min);
            format!("{d}. {} {y}", MONTHS_LONG[m])
        }
        None => String::new(),
    }
}

/// Statuszeile: Serien mit neuer Folge (und nicht neu angelegt) → „Neue Folgen“, sonst „Neu seit <Datum>“.
pub fn since_label(
    item: &BaseItem,
    new_series_ids: Option<&HashSet<String>>,
    now_ms: f64,
    utc_offset_min: i64,
) -> String {
    let recent =
        parse_opt(item.date_created.as_deref()).is_some_and(|c| now_ms - c < NEW_DAYS * DAY_MS);
    let has = new_series_ids.is_some_and(|s| s.contains(&item.id));
    if item.kind == ItemKind::Series && has && !recent {
        return "Neue Folgen".into();
    }
    let d = long_date_de(item.date_created.as_deref(), utc_offset_min);
    if d.is_empty() {
        "Neu".into()
    } else {
        format!("Neu seit {d}")
    }
}

// ---------------------------------------------------------------- Neu-Info

/// Art des neuen Inhalts einer Serie.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewKind {
    /// Neue Staffel.
    Season,
    /// Neue Folge.
    Episode,
}

/// Neu-Info (`getNewContentInfo`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewContentInfo {
    /// Serien-Id → neue Staffel/Folge.
    pub by_series: HashMap<String, NewKind>,
    /// Neue Filme.
    pub new_movie_ids: HashSet<String>,
    /// Neue Serien.
    pub new_series_ids: HashSet<String>,
}

/// Abfragen der Neu-Info: (Folgen, Titel). Jellyfin 12.4 ignoriert `MinDateCreated`: nach `DateCreated`
/// absteigend sortieren und clientseitig filtern.
pub fn new_content_calls() -> (ApiCall, ApiCall) {
    let eps = ApiCall::new(
        Endpoint::Items,
        ItemsQuery::new()
            .include_types(&[ItemKind::Episode])
            .recursive(true)
            .sort_by(&[SortBy::DateCreated])
            .order(SortOrder::Descending)
            .limit(500)
            .fields(&[
                "DateCreated",
                "ParentIndexNumber",
                "IndexNumber",
                "SeriesId",
            ])
            .enable_images(false)
            .enable_user_data(false),
    );
    let titles = ApiCall::new(
        Endpoint::Items,
        ItemsQuery::new()
            .include_types(&BOTH)
            .recursive(true)
            .sort_by(&[SortBy::DateCreated])
            .order(SortOrder::Descending)
            .limit(100)
            .fields(&["DateCreated"])
            .enable_images(false)
            .enable_user_data(false),
    );
    (eps, titles)
}

/// Serien-Id → `Season` | `Episode`. Neue Folge im Fenster mit `IndexNumber == 1` und `ParentIndexNumber > 1`
/// = neue Staffel, sonst neue Folge; `Season` gewinnt.
pub fn new_content_from(episodes: &[BaseItem], now_ms: f64, days: f64) -> HashMap<String, NewKind> {
    let mut map = HashMap::new();
    for e in episodes {
        let Some(sid) = e.series_id.as_deref().filter(|s| !s.is_empty()) else {
            continue;
        };
        if !is_recent(e.date_created.as_deref(), now_ms, days) {
            continue;
        }
        let season = e.index_number == Some(1) && e.parent_index_number.is_some_and(|p| p > 1);
        if season {
            map.insert(sid.to_owned(), NewKind::Season);
        } else {
            map.entry(sid.to_owned()).or_insert(NewKind::Episode);
        }
    }
    map
}

/// Neu-Info aus den beiden Antworten; `None` = Abruf gescheitert (nur beide gescheitert ist ein Fehler).
pub fn new_content_info(
    episodes: Option<&[BaseItem]>,
    titles: Option<&[BaseItem]>,
    now_ms: f64,
) -> NewContentInfo {
    let mut info = NewContentInfo::default();
    if let Some(e) = episodes {
        info.by_series = new_content_from(e, now_ms, NEW_DAYS);
    }
    for it in titles.unwrap_or_default() {
        if it.id.is_empty() || !is_recent(it.date_created.as_deref(), now_ms, NEW_DAYS) {
            continue;
        }
        match it.kind {
            ItemKind::Movie => {
                info.new_movie_ids.insert(it.id.clone());
            }
            ItemKind::Series => {
                info.new_series_ids.insert(it.id.clone());
            }
            _ => {}
        }
    }
    info
}

fn new_kind(item: &BaseItem, info: &NewContentInfo) -> Option<&'static str> {
    match item.kind {
        ItemKind::Movie => info.new_movie_ids.contains(&item.id).then_some("movie"),
        ItemKind::Series | ItemKind::Episode => {
            let sid = if item.kind == ItemKind::Episode {
                item.series_id.as_deref().unwrap_or("")
            } else {
                item.id.as_str()
            };
            if item.kind == ItemKind::Series && info.new_series_ids.contains(sid) {
                return Some("series");
            }
            match info.by_series.get(sid)? {
                NewKind::Season => Some("season"),
                NewKind::Episode => Some("episode"),
            }
        }
        _ => None,
    }
}

/// Badge in Großbuchstaben („NEUER FILM“ …) für LENZFLIX/LENZ//84.
pub fn new_badge(item: &BaseItem, info: &NewContentInfo) -> Option<&'static str> {
    Some(match new_kind(item, info)? {
        "movie" => "NEUER FILM",
        "series" => "NEUE SERIE",
        "season" => "NEUE STAFFEL",
        _ => "NEUE FOLGE",
    })
}

/// Badge in Satzschreibung („Neuer Film“ …) für LENZ+/LENZPRIME.
pub fn plus_badge(item: &BaseItem, info: &NewContentInfo) -> Option<&'static str> {
    Some(match new_kind(item, info)? {
        "movie" => "Neuer Film",
        "series" => "Neue Serie",
        "season" => "Neue Staffel",
        _ => "Neue Folge",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use lfx_api::UserItemData;

    const NOW: f64 = 1_795_217_400_000.0;

    fn it(id: &str, kind: ItemKind, created: &str) -> BaseItem {
        BaseItem {
            id: id.into(),
            kind,
            date_created: Some(created.into()),
            ..Default::default()
        }
    }

    #[test]
    fn sections_and_labels() {
        assert_eq!(
            Section::ALL.map(Section::id),
            ["new", "watched", "top-series", "top-movies"]
        );
        assert_eq!(Section::New.label("LENZFLIX"), "Neu bei LENZFLIX");
        assert_eq!(
            chart_subtitle("LENZFLIX", Some(7)),
            "Meistgesehen auf LENZFLIX · letzte 7 Tage"
        );
        assert_eq!(
            chart_subtitle("LENZFLIX", None),
            "Meistgesehen auf LENZFLIX · letzte 30 Tage"
        );
        assert_eq!(fallback_subtitle(Section::TopMovies), Some("Neueste Filme"));
    }

    #[test]
    fn queries_match_web() {
        let n = new_call();
        assert_eq!(
            (
                n.param("IncludeItemTypes"),
                n.param("SortBy"),
                n.param("Limit")
            ),
            (Some("Movie,Series"), Some("DateCreated"), Some("20"))
        );
        assert_eq!(most_watched_call().param("SortBy"), Some("PlayCount"));
        assert_eq!(
            most_watched_fallback_call().param("SortBy"),
            Some("CommunityRating")
        );
        let t = top_call(ItemKind::Series);
        assert_eq!(
            (t.param("IncludeItemTypes"), t.param("Limit")),
            (Some("Series"), Some("10"))
        );
        let (e, ti) = new_content_calls();
        assert_eq!(
            (e.param("Limit"), ti.param("Limit")),
            (Some("500"), Some("100"))
        );
        assert_eq!(e.param("EnableImages"), Some("false"));
        assert!(cache_key("u", &n).starts_with("home:u:np:"));
    }

    #[test]
    fn watched_rule() {
        let played = BaseItem {
            user_data: Some(UserItemData {
                play_count: 2,
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(watched_plan(3, &[]), WatchedPlan::Charts);
        assert_eq!(watched_plan(2, &[played]), WatchedPlan::PlayCount);
        assert_eq!(watched_plan(0, &[BaseItem::default()]), WatchedPlan::Rating);
        assert!(!chart_usable(2) && chart_usable(3));
    }

    #[test]
    fn unique_limits_and_filters() {
        let v = vec![
            it("a", ItemKind::Movie, ""),
            it("a", ItemKind::Movie, ""),
            it("e", ItemKind::Episode, ""),
            it("b", ItemKind::Series, ""),
            it("c", ItemKind::Series, ""),
        ];
        let ids: Vec<String> = unique_titles(&v, 2).into_iter().map(|i| i.id).collect();
        assert_eq!(ids, ["a", "b"]);
    }

    #[test]
    fn german_dates() {
        assert_eq!(
            long_date_de(Some("2026-11-08T10:00:00Z"), 0),
            "8. November 2026"
        );
        assert_eq!(
            long_date_de(Some("2026-03-31T23:30:00Z"), 60),
            "1. April 2026"
        );
        assert_eq!(long_date_de(Some("x"), 0), "");
        assert_eq!(
            date_parts(Some("2026-03-08T10:00:00Z"), 0),
            Some(("MÄR", "08".to_owned()))
        );
        assert_eq!(date_parts(None, 0), None);
    }

    #[test]
    fn since_labels() {
        let set = HashSet::from(["s".to_owned()]);
        let old = it("s", ItemKind::Series, "2026-03-08T10:00:00Z");
        assert_eq!(since_label(&old, Some(&set), NOW, 0), "Neue Folgen");
        assert_eq!(since_label(&old, None, NOW, 0), "Neu seit 8. März 2026");
        let fresh = it("s", ItemKind::Series, "2026-11-18T10:00:00Z");
        assert_eq!(
            since_label(&fresh, Some(&set), NOW, 0),
            "Neu seit 18. November 2026"
        );
        assert_eq!(since_label(&BaseItem::default(), None, NOW, 0), "Neu");
    }

    #[test]
    fn new_content_rules() {
        let ep = |sid: &str, idx: i32, pidx: i32, created: &str| BaseItem {
            id: format!("{sid}{idx}{pidx}"),
            kind: ItemKind::Episode,
            series_id: Some(sid.into()),
            index_number: Some(idx),
            parent_index_number: Some(pidx),
            date_created: Some(created.into()),
            ..Default::default()
        };
        let eps = vec![
            ep("a", 3, 1, "2026-11-19T00:00:00Z"),
            ep("a", 1, 2, "2026-11-19T00:00:00Z"),
            ep("b", 2, 1, "2026-11-19T00:00:00Z"),
            ep("c", 1, 1, "2026-01-01T00:00:00Z"),
        ];
        let m = new_content_from(&eps, NOW, 14.0);
        assert_eq!(m.get("a"), Some(&NewKind::Season), "Staffel gewinnt");
        assert_eq!(m.get("b"), Some(&NewKind::Episode));
        assert!(!m.contains_key("c"));

        let titles = vec![
            it("m1", ItemKind::Movie, "2026-11-19T00:00:00Z"),
            it("s1", ItemKind::Series, "2026-11-19T00:00:00Z"),
            it("m2", ItemKind::Movie, "2020-01-01T00:00:00Z"),
        ];
        let info = new_content_info(Some(&eps), Some(&titles), NOW);
        assert_eq!(new_badge(&titles[0], &info), Some("NEUER FILM"));
        assert_eq!(new_badge(&titles[1], &info), Some("NEUE SERIE"));
        assert_eq!(new_badge(&titles[2], &info), None);
        let series_a = it("a", ItemKind::Series, "2020-01-01T00:00:00Z");
        assert_eq!(plus_badge(&series_a, &info), Some("Neue Staffel"));
        let ep_b = ep("b", 2, 1, "2026-11-19T00:00:00Z");
        assert_eq!(plus_badge(&ep_b, &info), Some("Neue Folge"));
    }
}
