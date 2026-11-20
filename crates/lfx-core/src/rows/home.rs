//! Zeilenliste der Startseite (Port von `computeRowDefs` in `rows/sources.js`).

use super::logic::base_query;
use super::{RowDef, RowKind, RowSource, items_call};
use crate::call::{ApiCall, Endpoint};
use crate::dayseed::{DEFAULT_GENRE_COUNT, GenreInput, MIN_GENRE_TITLES, pick_genres};
use lfx_api::{BaseItem, ItemKind, ItemsQuery, SortBy, SortOrder};

/// Mindestzahl auflösbarer Titel, damit eine Chart-Zeile erscheint (`MIN_CHART_ROW`).
pub const MIN_CHART_ROW: usize = 5;
/// Zeilen-Cache der Ansichten (`VIEWS_TTL`).
pub const VIEWS_TTL_MS: u64 = 5 * 60_000;

/// Schon aufgelöste Chart-Listen (je Schlüssel höchstens 10 Titel); Auflösung ist Sache des Aufrufers.
#[derive(Debug, Clone, Default)]
pub struct ChartLists {
    /// Top-Serien.
    pub series: Vec<BaseItem>,
    /// Top-Filme.
    pub movies: Vec<BaseItem>,
    /// Alle Titel (für „Am häufigsten geschaut“).
    pub all: Vec<BaseItem>,
}

/// Eingabe für [`build_home_defs`].
#[derive(Debug, Clone)]
pub struct HomeInput<'a> {
    /// Ansichten des Benutzers (`/UserViews`); leer, wenn der Abruf scheiterte.
    pub views: &'a [BaseItem],
    /// Genres (`/Genres`); leer, wenn der Abruf scheiterte.
    pub genres: &'a [BaseItem],
    /// Abruf der Ansichten und Genres gelang.
    pub lookups_ok: bool,
    /// Tages-Seed (`dayKey`).
    pub seed: &'a str,
    /// Anzahl Genre-Zeilen (Standard 6).
    pub genre_count: usize,
    /// Markenname der Oberfläche („LENZFLIX“ …).
    pub brand: &'a str,
    /// Ein Charts-Dienst ist konfiguriert.
    pub charts_configured: bool,
    /// Aufgelöste Charts (`None` = nicht erreichbar/keine).
    pub charts: Option<&'a ChartLists>,
}

/// Ergebnis: Zeilen und ob die Liste „gebrochen“ ist (dann nicht cachen: `degraded`).
#[derive(Debug, Clone)]
pub struct HomeDefs {
    /// Zeilen in Anzeigereihenfolge.
    pub rows: Vec<RowDef>,
    /// Ohne Bibliotheken/Genres oder Charts-Ersatz: nicht cachen.
    pub degraded: bool,
}

/// Titel der Chart-Zeilen (`chartSpecs`).
pub fn chart_title(key: &str, brand: &str) -> String {
    match key {
        "series" => format!("Top 10 Serien auf {brand}"),
        _ => format!("Top 10 Filme auf {brand}"),
    }
}

/// Titel der Ersatzzeile ohne Charts.
pub fn top_new_title(brand: &str) -> String {
    format!("Top 10: Neu auf {brand}")
}

/// Ersatzabfrage der Top 10: die 10 neuesten Filme/Serien (`newTop10`).
pub fn top_new_call() -> ApiCall {
    items_call(&[ItemKind::Movie, ItemKind::Series], |q| {
        q.sort_by(&[SortBy::DateCreated])
            .order(SortOrder::Descending)
            .limit(10)
    })
}

fn row(id: &str, title: Option<String>, kind: RowKind, source: RowSource) -> RowDef {
    RowDef {
        id: id.to_owned(),
        title,
        kind,
        source,
        no_cache: false,
    }
}

fn query(id: &str, title: &str, kind: RowKind, call: ApiCall) -> RowDef {
    row(
        id,
        Some(title.to_owned()),
        kind,
        RowSource::Query {
            call,
            min_items: 0,
            type_filter: None,
        },
    )
}

/// Baut die Zeilen der Startseite. Reihenfolge: Weiterschauen, Als Nächstes, Top 10 (Charts oder Ersatz),
/// Neu hinzugefügt (Filme, Serien), Meine Liste, „Weil du …“, Genre-Zeilen, Hochbewertet.
pub fn build_home_defs(input: &HomeInput<'_>) -> HomeDefs {
    let movie_view = input
        .views
        .iter()
        .find(|v| v.collection_type.as_deref() == Some("movies"));
    let tv_view = input
        .views
        .iter()
        .find(|v| v.collection_type.as_deref() == Some("tvshows"));
    let both = [ItemKind::Movie, ItemKind::Series];
    let mut rows = Vec::new();
    let mut charts_missing = false;

    rows.push(query(
        "resume",
        "Weiterschauen",
        RowKind::Resume,
        ApiCall::new(
            Endpoint::Resume,
            base_query().media_types(&["Video"]).limit(20),
        ),
    ));
    rows.push(query(
        "nextup",
        "Als Nächstes",
        RowKind::Landscape,
        ApiCall::new(
            Endpoint::NextUp,
            base_query()
                .limit(20)
                .raw("enableResumable", "false")
                .raw("enableRewatching", "false"),
        ),
    ));

    let mut chart_rows = Vec::new();
    if let Some(c) = input.charts {
        for (id, key, list) in [
            ("top10-series", "series", &c.series),
            ("top10-movies", "movies", &c.movies),
        ] {
            if list.len() >= MIN_CHART_ROW {
                chart_rows.push(row(
                    id,
                    Some(chart_title(key, input.brand)),
                    RowKind::Top10,
                    RowSource::Fixed(list.clone()),
                ));
            }
        }
    }
    if chart_rows.is_empty() {
        let mut fallback = row(
            "top10",
            Some(top_new_title(input.brand)),
            RowKind::Top10,
            RowSource::TopNew {
                call: top_new_call(),
            },
        );
        charts_missing = input.charts_configured;
        fallback.no_cache = false;
        rows.push(fallback);
    } else {
        rows.extend(chart_rows);
    }

    for (id, title, view) in [
        ("latest-movies", "Neu hinzugefügt: Filme", movie_view),
        ("latest-series", "Neu hinzugefügt: Serien", tv_view),
    ] {
        if let Some(v) = view {
            rows.push(query(
                id,
                title,
                RowKind::Landscape,
                ApiCall::new(
                    Endpoint::Latest,
                    base_query()
                        .parent_id(&v.id)
                        .raw("groupItems", "true")
                        .limit(20),
                ),
            ));
        }
    }

    rows.push(query(
        "favorites",
        "Meine Liste",
        RowKind::Landscape,
        items_call(&both, |q| {
            q.is_favorite(true).sort_by(&[SortBy::SortName]).limit(50)
        }),
    ));
    rows.push(row(
        "because",
        None,
        RowKind::Landscape,
        RowSource::Because {
            played: items_call(&[ItemKind::Movie, ItemKind::Episode], |q| {
                q.filters(&[lfx_api::ItemFilter::IsPlayed])
                    .sort_by(&[SortBy::DatePlayed])
                    .order(SortOrder::Descending)
                    .limit(1)
            }),
            similar_limit: 20,
            type_filter: None,
        },
    ));

    let names: Vec<GenreInput> = input
        .genres
        .iter()
        .map(|g| GenreInput::named(g.name.clone()))
        .collect();
    let count = if input.genre_count == 0 {
        DEFAULT_GENRE_COUNT
    } else {
        input.genre_count
    };
    for name in pick_genres(&names, input.seed, count, MIN_GENRE_TITLES) {
        let call = items_call(&both, |q| {
            q.genres(&[name.as_str()])
                .sort_by(&[SortBy::Random])
                .limit(20)
        });
        rows.push(row(
            &format!("genre-{name}"),
            Some(name.clone()),
            RowKind::Landscape,
            RowSource::Query {
                call,
                min_items: 8,
                type_filter: None,
            },
        ));
    }

    rows.push(query(
        "toprated",
        "Hochbewertet",
        RowKind::Landscape,
        items_call(&both, |q| {
            q.sort_by(&[SortBy::CommunityRating])
                .order(SortOrder::Descending)
                .min_community_rating(7.5)
                .limit(20)
        }),
    ));

    HomeDefs {
        degraded: charts_missing || !input.lookups_ok,
        rows,
    }
}

/// Abfrage der Genres der Startseite (`/Genres`, Film und Serie, rekursiv).
pub fn genres_call() -> ApiCall {
    ApiCall::new(
        Endpoint::Genres,
        ItemsQuery::new()
            .include_types(&[ItemKind::Movie, ItemKind::Series])
            .recursive(true),
    )
}

/// Abfrage der Ansichten (`/UserViews`).
pub fn views_call() -> ApiCall {
    ApiCall::new(Endpoint::Views, ItemsQuery::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input<'a>(
        views: &'a [BaseItem],
        genres: &'a [BaseItem],
        charts: Option<&'a ChartLists>,
    ) -> HomeInput<'a> {
        HomeInput {
            views,
            genres,
            lookups_ok: true,
            seed: "2026-11-20",
            genre_count: 6,
            brand: "LENZFLIX",
            charts_configured: false,
            charts,
        }
    }

    fn view(id: &str, ty: &str) -> BaseItem {
        BaseItem {
            id: id.into(),
            collection_type: Some(ty.into()),
            ..Default::default()
        }
    }

    fn genre(n: &str) -> BaseItem {
        BaseItem {
            id: n.into(),
            name: n.into(),
            ..Default::default()
        }
    }

    fn ids(d: &HomeDefs) -> Vec<&str> {
        d.rows.iter().map(|r| r.id.as_str()).collect()
    }

    fn call(d: &HomeDefs, id: &str) -> ApiCall {
        match &d.rows.iter().find(|r| r.id == id).unwrap().source {
            RowSource::Query { call, .. } | RowSource::TopNew { call } => call.clone(),
            RowSource::Because { played, .. } => played.clone(),
            other => panic!("keine Abfrage: {other:?}"),
        }
    }

    #[test]
    fn order_and_exact_params() {
        let views = [view("vm", "movies"), view("vs", "tvshows")];
        let genres: Vec<BaseItem> = ["A", "B", "C", "D", "E", "F", "G", "H"]
            .iter()
            .map(|n| genre(n))
            .collect();
        let d = build_home_defs(&input(&views, &genres, None));
        let got = ids(&d);
        assert_eq!(&got[..2], ["resume", "nextup"]);
        assert_eq!(got[2], "top10");
        assert_eq!(
            &got[3..7],
            ["latest-movies", "latest-series", "favorites", "because"]
        );
        assert_eq!(*got.last().unwrap(), "toprated");
        assert_eq!(got.iter().filter(|i| i.starts_with("genre-")).count(), 6);
        assert!(!d.degraded);

        let r = call(&d, "resume");
        assert_eq!(r.endpoint, Endpoint::Resume);
        assert_eq!(
            (r.param("MediaTypes"), r.param("Limit")),
            (Some("Video"), Some("20"))
        );
        let n = call(&d, "nextup");
        assert_eq!(
            (n.param("EnableResumable"), n.param("EnableRewatching")),
            (Some("false"), Some("false"))
        );
        let l = call(&d, "latest-movies");
        assert_eq!(
            (l.param("ParentId"), l.param("GroupItems"), l.param("Limit")),
            (Some("vm"), Some("true"), Some("20"))
        );
        let f = call(&d, "favorites");
        assert_eq!(f.param("IsFavorite"), Some("true"));
        assert_eq!(
            (
                f.param("SortBy"),
                f.param("Limit"),
                f.param("IncludeItemTypes")
            ),
            (Some("SortName"), Some("50"), Some("Movie,Series"))
        );
        let b = call(&d, "because");
        assert_eq!(
            (
                b.param("Filters"),
                b.param("SortBy"),
                b.param("SortOrder"),
                b.param("Limit")
            ),
            (
                Some("IsPlayed"),
                Some("DatePlayed"),
                Some("Descending"),
                Some("1")
            )
        );
        assert_eq!(b.param("IncludeItemTypes"), Some("Movie,Episode"));
        let t = call(&d, "toprated");
        assert_eq!(
            (
                t.param("MinCommunityRating"),
                t.param("SortBy"),
                t.param("SortOrder")
            ),
            (Some("7.5"), Some("CommunityRating"), Some("Descending"))
        );
        let g = d.rows.iter().find(|r| r.id.starts_with("genre-")).unwrap();
        let RowSource::Query {
            call: gc,
            min_items,
            ..
        } = &g.source
        else {
            panic!()
        };
        assert_eq!(*min_items, 8);
        assert_eq!(
            (gc.param("SortBy"), gc.param("Limit")),
            (Some("Random"), Some("20"))
        );
        assert!(gc.param("Genres").is_some());
        let top = call(&d, "top10");
        assert_eq!(
            (top.param("SortBy"), top.param("Limit")),
            (Some("DateCreated"), Some("10"))
        );
    }

    #[test]
    fn missing_views_skip_latest_rows() {
        let d = build_home_defs(&input(&[], &[], None));
        assert!(!ids(&d).iter().any(|i| i.starts_with("latest-")));
    }

    #[test]
    fn charts_rules() {
        let lists = ChartLists {
            series: (0..5).map(|i| genre(&format!("s{i}"))).collect(),
            movies: (0..4).map(|i| genre(&format!("m{i}"))).collect(),
            all: vec![],
        };
        let d = build_home_defs(&input(&[], &[], Some(&lists)));
        assert!(ids(&d).contains(&"top10-series"), "5 Titel reichen");
        assert!(!ids(&d).contains(&"top10-movies"), "4 Titel nicht");
        assert!(!ids(&d).contains(&"top10"));
        assert_eq!(
            d.rows[2].title.as_deref(),
            Some("Top 10 Serien auf LENZFLIX")
        );

        let empty = ChartLists::default();
        let mut i = input(&[], &[], Some(&empty));
        i.charts_configured = true;
        let d = build_home_defs(&i);
        assert!(ids(&d).contains(&"top10"));
        assert!(d.degraded, "konfiguriert, aber ohne Charts: nicht cachen");
        i.lookups_ok = false;
        i.charts_configured = false;
        assert!(build_home_defs(&i).degraded);
    }
}
