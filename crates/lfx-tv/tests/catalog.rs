//! Katalog laden und cachen (Semantik aus `tv-hull-data.test.js`), ohne Netz: Quelle im Speicher
//! und Adapter gegen den Mock-Server (127.0.0.1).

use std::cell::RefCell;

use lfx_api::{Client, ClientInfo};
use lfx_mock_jellyfin::{DEMO_PASSWORD, DEMO_USER, start};
use lfx_tv::api_source::ApiCatalogSource;
use lfx_tv::catalog::{
    CatalogCache, CatalogKind, CatalogSource, EPISODE_PAGE_SIZE, MemoryCatalogCache, PAGE_SIZE,
    PageRequest, RawCatalog, RawPage, broadcast_day_key, cache_key, clear_catalog_cache, compact,
    fetch_catalog, image_tag_map, load_catalog,
};
use lfx_tv::error::TvError;
use lfx_tv_schedule::time::{days_from_civil, wall_to_utc};
use serde_json::{Value, json};

struct MemSource {
    movies: usize,
    series: usize,
    episodes: usize,
    boxsets: usize,
    calls: RefCell<Vec<PageRequest>>,
    fail: bool,
    with_total: bool,
}

impl MemSource {
    fn new(movies: usize, series: usize, episodes: usize, boxsets: usize) -> Self {
        Self {
            movies,
            series,
            episodes,
            boxsets,
            calls: RefCell::new(Vec::new()),
            fail: false,
            with_total: true,
        }
    }

    fn item(kind: CatalogKind, i: usize) -> Value {
        match kind {
            CatalogKind::Movies => json!({
                "Id": format!("m{i}"), "Name": format!("Erfundener Film {i}"), "RunTimeTicks": 54_000_000_000i64,
                "Genres": ["Komödie"], "OfficialRating": null, "ProductionYear": 1999, "PremiereDate": "1999-01-01T00:00:00.0000000Z",
                "DateCreated": "2020-01-01T00:00:00.0000000Z", "LocalTrailerCount": 1,
                "ImageTags": {"Primary": "tp", "Thumb": "tt", "Logo": "tl"}, "BackdropImageTags": ["b1", "b2"],
                "UserData": {"Played": true}, "Overview": "nicht übernehmen"
            }),
            CatalogKind::Series => {
                json!({"Id": format!("s{i}"), "Name": format!("Serie {i}"), "Genres": [], "ImageTags": {}})
            }
            CatalogKind::Episodes => {
                json!({"Id": format!("e{i}"), "Name": "Folge", "SeriesId": "s0", "IndexNumber": 1, "ParentIndexNumber": 1})
            }
            CatalogKind::Boxsets => {
                json!({"Id": format!("b{i}"), "Name": "Reihe", "ChildCount": 3})
            }
        }
    }
}

impl CatalogSource for MemSource {
    async fn fetch_page(&self, req: &PageRequest) -> Result<RawPage, TvError> {
        self.calls.borrow_mut().push(req.clone());
        if self.fail {
            return Err(TvError::Source("Serverfehler HTTP 500".into()));
        }
        let n = match req.kind {
            CatalogKind::Movies => self.movies,
            CatalogKind::Series => self.series,
            CatalogKind::Episodes => self.episodes,
            CatalogKind::Boxsets => self.boxsets,
        };
        let end = (req.start_index + req.limit).min(n);
        let items = (req.start_index.min(n)..end)
            .map(|i| Self::item(req.kind, i))
            .collect();
        Ok(RawPage {
            items,
            total: self.with_total.then_some(n as i64),
        })
    }
}

fn now() -> i64 {
    wall_to_utc(days_from_civil(2026, 11, 20), 12 * 60)
}

#[tokio::test]
async fn laedt_alle_vier_teilkataloge_mit_schlanken_parametern() {
    let src = MemSource::new(3, 2, 5, 1);
    let raw = fetch_catalog(&src, now(), None).await.unwrap();
    assert_eq!(
        (
            raw.movies.len(),
            raw.series.len(),
            raw.episodes.len(),
            raw.boxsets.len()
        ),
        (3, 2, 5, 1)
    );
    assert_eq!(raw.loaded_at, now());
    let calls = src.calls.borrow();
    assert_eq!(calls.len(), 4);
    assert_eq!(calls[0].kind, CatalogKind::Movies);
    assert_eq!(calls[2].limit, EPISODE_PAGE_SIZE);
    assert_eq!(calls[0].limit, PAGE_SIZE);
    assert_eq!(CatalogKind::Movies.item_type(), "Movie");
    assert!(CatalogKind::Movies.with_image_tags() && CatalogKind::Series.with_image_tags());
    assert!(!CatalogKind::Episodes.with_image_tags() && !CatalogKind::Boxsets.with_image_tags());
    assert_eq!(
        CatalogKind::Episodes.fields(),
        "DateCreated,PremiereDate,RunTimeTicks,OfficialRating"
    );
    assert_eq!(CatalogKind::Boxsets.fields(), "ChildCount");
}

#[tokio::test]
async fn uebernimmt_nur_vertragsfelder_in_keep_reihenfolge() {
    let raw = fetch_catalog(&MemSource::new(1, 1, 1, 1), now(), None)
        .await
        .unwrap();
    let m = raw.movies[0].as_object().unwrap();
    let keys: Vec<&str> = m.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        [
            "Id",
            "Name",
            "RunTimeTicks",
            "Genres",
            "ProductionYear",
            "PremiereDate",
            "DateCreated",
            "LocalTrailerCount",
            "ImageTags",
            "BackdropImageTags"
        ],
        "OfficialRating (null) entfällt, UserData/Overview fehlen, Reihenfolge wie KEEP"
    );
    assert_eq!(
        raw.movies[0]["ImageTags"],
        json!({"Thumb": "tt", "Primary": "tp"})
    );
    assert_eq!(raw.movies[0]["BackdropImageTags"], json!(["b1"]));
    assert!(raw.series[0]["ImageTags"].as_object().unwrap().is_empty());
}

#[test]
fn compact_laesst_fehlende_felder_weg() {
    let v = compact(
        &json!({"Id": "x", "Name": null, "Foo": 1}),
        CatalogKind::Boxsets,
    );
    assert_eq!(v, json!({"Id": "x"}));
}

#[tokio::test]
async fn blaettert_bis_total_entfernt_doppelte_und_idlose() {
    struct Dup;
    impl CatalogSource for Dup {
        async fn fetch_page(&self, req: &PageRequest) -> Result<RawPage, TvError> {
            // Seite 1: 1000 Einträge (ein Duplikat, einer ohne Id), Seite 2: Rest
            let items: Vec<Value> = if req.start_index == 0 {
                let mut v: Vec<Value> = (0..PAGE_SIZE - 2)
                    .map(|i| json!({"Id": format!("s{i}"), "Name": "x"}))
                    .collect();
                v.push(json!({"Id": "s0", "Name": "doppelt"}));
                v.push(json!({"Name": "ohne Id"}));
                v
            } else {
                vec![json!({"Id": "ende", "Name": "y"})]
            };
            Ok(RawPage {
                items,
                total: Some(PAGE_SIZE as i64 + 1),
            })
        }
    }
    let mut parts = Vec::new();
    let out = lfx_tv::catalog::fetch_kind(&Dup, CatalogKind::Series, &mut |f| parts.push(f))
        .await
        .unwrap();
    assert_eq!(out.len(), PAGE_SIZE - 2 + 1);
    assert_eq!(out[0]["Name"], "x", "erster gewinnt");
    assert!(
        parts.windows(2).all(|w| w[0] <= w[1]) && *parts.last().unwrap() == 1.0 && parts[0] < 1.0
    );
}

#[tokio::test]
async fn exakt_volle_letzte_seite_kein_ueberfluessiger_abruf() {
    let src = MemSource::new(PAGE_SIZE, 0, 0, 0);
    let mut sink = |_: f64| {};
    let out = lfx_tv::catalog::fetch_kind(&src, CatalogKind::Movies, &mut sink)
        .await
        .unwrap();
    assert_eq!(out.len(), PAGE_SIZE);
    assert_eq!(src.calls.borrow().len(), 1);
}

#[tokio::test]
async fn ohne_total_blaettert_bis_kurze_seite() {
    let mut src = MemSource::new(PAGE_SIZE + 5, 0, 0, 0);
    src.with_total = false;
    let mut sink = |_: f64| {};
    let out = lfx_tv::catalog::fetch_kind(&src, CatalogKind::Movies, &mut sink)
        .await
        .unwrap();
    assert_eq!(out.len(), PAGE_SIZE + 5);
    assert_eq!(src.calls.borrow().len(), 2);
}

#[tokio::test]
async fn fortschritt_monoton_bis_1() {
    let src = MemSource::new(2, 2, 2, 2);
    let mut seen: Vec<f64> = Vec::new();
    let mut cb = |f: f64, label: &str| {
        assert_eq!(label, "Sendersuchlauf …");
        seen.push(f);
    };
    fetch_catalog(&src, now(), Some(&mut cb)).await.unwrap();
    assert!(seen.windows(2).all(|w| w[0] <= w[1]), "{seen:?}");
    assert_eq!(*seen.last().unwrap(), 1.0);
}

#[tokio::test]
async fn zweiter_aufruf_kommt_aus_dem_cache() {
    let src = MemSource::new(2, 2, 2, 2);
    let mut cache = MemoryCatalogCache::new();
    let a = load_catalog(&src, &mut cache, "nutzer1", now(), None)
        .await
        .unwrap();
    let n = src.calls.borrow().len();
    let b = load_catalog(&src, &mut cache, "nutzer1", now(), None)
        .await
        .unwrap();
    assert_eq!(src.calls.borrow().len(), n, "kein neuer Abruf");
    assert_eq!(a, b);
    let mut got = Vec::new();
    let mut cb = |f: f64, _: &str| got.push(f);
    load_catalog(&src, &mut cache, "nutzer1", now(), Some(&mut cb))
        .await
        .unwrap();
    assert_eq!(got, vec![1.0]);
}

#[tokio::test]
async fn neuer_sendetag_oder_anderer_benutzer_laedt_neu_und_ersetzt_aelteres() {
    let src = MemSource::new(1, 1, 1, 1);
    let mut cache = MemoryCatalogCache::new();
    load_catalog(&src, &mut cache, "u1", now(), None)
        .await
        .unwrap();
    let n1 = src.calls.borrow().len();
    // 04:59 gehört noch zum alten Sendetag, 05:01 des nächsten Tages nicht
    let before = wall_to_utc(days_from_civil(2026, 11, 21), 4 * 60 + 59);
    load_catalog(&src, &mut cache, "u1", before, None)
        .await
        .unwrap();
    assert_eq!(src.calls.borrow().len(), n1);
    let next = wall_to_utc(days_from_civil(2026, 11, 21), 5 * 60 + 1);
    load_catalog(&src, &mut cache, "u1", next, None)
        .await
        .unwrap();
    assert!(src.calls.borrow().len() > n1);
    assert_eq!(
        cache.keys(),
        vec![cache_key("u1", next)],
        "ältere Einträge des Benutzers ersetzt"
    );
    load_catalog(&src, &mut cache, "u2", next, None)
        .await
        .unwrap();
    assert_eq!(
        cache.keys().len(),
        2,
        "anderer Benutzer hat eigenen Eintrag"
    );
    clear_catalog_cache(&mut cache);
    assert!(cache.keys().is_empty());
}

#[tokio::test]
async fn fehler_wird_nicht_gecacht_und_traegt_keinen_schluessel() {
    let mut src = MemSource::new(1, 1, 1, 1);
    src.fail = true;
    let mut cache = MemoryCatalogCache::new();
    let e = load_catalog(&src, &mut cache, "u", now(), None)
        .await
        .unwrap_err();
    assert!(!format!("{e}").contains("ApiKey"));
    assert!(cache.keys().is_empty());
    assert!(cache.get(&cache_key("u", now())).is_none());
}

#[test]
fn schluessel_sendetag_in_planzone() {
    assert_eq!(
        broadcast_day_key(wall_to_utc(days_from_civil(2026, 11, 20), 12 * 60)),
        "2026-11-20"
    );
    assert_eq!(
        broadcast_day_key(wall_to_utc(days_from_civil(2026, 11, 21), 4 * 60)),
        "2026-11-20"
    );
    assert_eq!(
        broadcast_day_key(wall_to_utc(days_from_civil(2026, 11, 21), 5 * 60)),
        "2026-11-21"
    );
    assert_eq!(cache_key("", now()), "anon:2026-11-20");
}

#[test]
fn raw_catalog_json_runde_und_validierung() {
    let raw = RawCatalog {
        movies: vec![json!({"Id": "a"})],
        series: vec![],
        episodes: vec![],
        boxsets: vec![],
        loaded_at: 5,
    };
    assert_eq!(RawCatalog::from_json(&raw.to_json()), Some(raw.clone()));
    assert_eq!(RawCatalog::from_json(&json!({"movies": []})), None);
    let _ = raw.to_jsval();
}

#[tokio::test]
async fn bild_tags_je_id_aus_dem_katalog() {
    let raw = fetch_catalog(&MemSource::new(2, 1, 0, 0), now(), None)
        .await
        .unwrap();
    let m = image_tag_map(&raw);
    assert_eq!(m["m0"].primary.as_deref(), Some("tp"));
    assert_eq!(m["m0"].thumb.as_deref(), Some("tt"));
    assert_eq!(m["m0"].backdrop.as_deref(), Some("b1"));
    assert!(!m.contains_key("s0"), "Serie ohne Tags");
}

// ---------------------------------------------------------------- Adapter gegen den Mock

#[tokio::test(flavor = "multi_thread")]
async fn adapter_laedt_den_katalog_vom_mock_ohne_schluessel_in_urls() {
    let (addr, handle) = start().await;
    let client = Client::new(
        &format!("http://{addr}"),
        ClientInfo::new("Test-Rechner", "00000000-0000-4000-8000-000000000000"),
    )
    .unwrap();
    client.login(DEMO_USER, DEMO_PASSWORD).await.unwrap();
    let src = ApiCatalogSource::new(&client);
    let raw = fetch_catalog(&src, now(), None).await.unwrap();
    assert!(!raw.movies.is_empty(), "Mock liefert Filme");
    for it in &raw.movies {
        assert!(it["Id"].is_string() && it["Name"].is_string());
    }
    for r in handle.requests() {
        let dbg = format!("{r:?}").to_lowercase();
        assert!(!dbg.contains("api_key="), "Schlüssel nie in der URL");
    }
    // der Katalog lässt sich für den Planer normalisieren
    let cat = lfx_tv_schedule::normalize_catalog(&raw.to_jsval());
    assert!(!cat.movies.is_empty() || !cat.series.is_empty());
}
