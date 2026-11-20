//! Golden-Tests gegen die Original-JS-Funktionen des Web-Themes.
//! Vektoren: `tests/golden/web.json` (erzeugt von `tests/golden/gen.mjs`, nur erfundene Daten).

use lfx_api::{BaseItem, ImageType, ItemKind};
use lfx_core::billboard::{self, HeroItem, PoolOpts};
use lfx_core::datasaver::{ImageScales, scaled};
use lfx_core::dayseed::{self, GenreInput};
use lfx_core::images::{pick_landscape, pick_logo};
use lfx_core::library::{
    LfxRoute, LibSort, LibraryKind, LibraryRoute, MyListRoute, MyListType, ViewMode, parse_route,
    sort_params,
};
use lfx_core::newpopular;
use lfx_core::rows::logic;
use serde_json::Value;
use std::collections::HashSet;

fn g() -> Value {
    serde_json::from_str(include_str!("golden/web.json")).expect("web.json")
}

fn arr<'a>(v: &'a Value, k: &str) -> &'a Vec<Value> {
    v[k].as_array().unwrap_or_else(|| panic!("{k} fehlt"))
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap()
}

fn items(g: &Value) -> Vec<(Value, BaseItem)> {
    arr(g, "items")
        .iter()
        .map(|v| (v.clone(), serde_json::from_value(v.clone()).unwrap()))
        .collect()
}

fn now(g: &Value) -> f64 {
    g["now"].as_f64().unwrap()
}

fn opt_str(v: &Value) -> Option<&str> {
    v.as_str()
}

fn ids(list: &[BaseItem]) -> Vec<String> {
    list.iter().map(|i| i.id.clone()).collect()
}

fn id_list(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| s(x).to_owned())
        .collect()
}

#[test]
fn hash_and_random_match_dayseed_js() {
    let g = g();
    for e in arr(&g, "hash") {
        assert_eq!(
            dayseed::hash_string(s(&e[0])),
            e[1].as_u64().unwrap() as u32,
            "{}",
            e[0]
        );
    }
    for e in arr(&g, "random") {
        let mut r = dayseed::seeded_random(s(&e[0]));
        for (i, want) in e[1].as_array().unwrap().iter().enumerate() {
            assert_eq!(r.next(), want.as_f64().unwrap(), "seed {} #{i}", e[0]);
        }
    }
}

#[test]
fn shuffle_matches_both_js_variants() {
    let g = g();
    let cases = arr(&g, "shuffle");
    assert!(cases.len() >= 30);
    for c in cases {
        let n = c["n"].as_u64().unwrap() as usize;
        let list: Vec<usize> = (0..n).collect();
        let got = dayseed::seeded_shuffle(&list, s(&c["seed"]));
        let want: Vec<usize> = c["out"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_u64().unwrap() as usize)
            .collect();
        assert_eq!(got, want, "n={n} seed={}", c["seed"]);
    }
}

#[test]
fn pick_genres_matches_js_including_german_collation() {
    let g = g();
    let cases = arr(&g, "pickGenres");
    assert!(!cases.is_empty());
    for c in cases {
        let names: Vec<GenreInput> = c["input"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| GenreInput::named(s(n)))
            .collect();
        let got = dayseed::pick_genres(
            &names,
            s(&c["seed"]),
            c["count"].as_u64().unwrap() as usize,
            dayseed::MIN_GENRE_TITLES,
        );
        assert_eq!(
            got,
            id_list(&c["out"]),
            "seed={} input={}",
            c["seed"],
            c["input"]
        );
    }
    for c in arr(&g, "pickGenresObjects") {
        let input: Vec<GenreInput> = c["input"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| match n.as_str() {
                Some(t) => GenreInput::named(t),
                None => GenreInput {
                    name: n["Name"].as_str().unwrap_or("").to_owned(),
                    movie_count: n["MovieCount"].as_i64(),
                    series_count: n["SeriesCount"].as_i64(),
                },
            })
            .collect();
        let got = dayseed::pick_genres(&input, s(&c["seed"]), 6, dayseed::MIN_GENRE_TITLES);
        assert_eq!(got, id_list(&c["out"]));
    }
}

#[test]
fn day_key_matches_js() {
    for e in arr(&g(), "dayKey") {
        let (y, m, d) = (
            e[0].as_i64().unwrap(),
            e[1].as_i64().unwrap(),
            e[2].as_i64().unwrap(),
        );
        assert_eq!(dayseed::day_key(y, m, d), s(&e[3]));
    }
}

#[test]
fn billboard_format_matches_js() {
    let g = g();
    for e in arr(&g, "formatRuntime") {
        let t = e[0].as_f64().map(|f| f as i64);
        assert_eq!(billboard::format_runtime(t), s(&e[1]), "{}", e[0]);
    }
    for e in arr(&g, "formatRating") {
        assert_eq!(
            billboard::format_rating(Some(s(&e[0]))),
            s(&e[1]),
            "{}",
            e[0]
        );
    }
    for e in arr(&g, "formatSeasons") {
        assert_eq!(billboard::format_seasons(e[0].as_i64()), s(&e[1]));
    }
    for e in arr(&g, "genreLine") {
        let item = BaseItem {
            genres: id_list(&e[0]),
            ..Default::default()
        };
        assert_eq!(billboard::genre_line(&item), s(&e[1]));
    }
    let all = items(&g);
    for e in arr(&g, "metaParts") {
        let item = all.iter().find(|(raw, _)| raw["Id"] == e[0]["Id"]).unwrap();
        let got: Vec<(String, bool)> = billboard::meta_parts(&item.1)
            .into_iter()
            .map(|p| (p.text, p.rating))
            .collect();
        let want: Vec<(String, bool)> = e[1]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                (
                    s(&p["text"]).to_owned(),
                    p["rating"].as_bool().unwrap_or(false),
                )
            })
            .collect();
        assert_eq!(got, want, "{}", e[0]["Id"]);
    }
    for (i, e) in arr(&g, "bbBadge").iter().enumerate() {
        assert_eq!(
            billboard::badge_for(&all[i].1, now(&g)),
            opt_str(&e[1]),
            "{}",
            e[0]
        );
    }
    for (i, e) in arr(&g, "hasProgress").iter().enumerate() {
        assert_eq!(
            billboard::has_progress(&all[i].1),
            e[1].as_bool().unwrap(),
            "{}",
            e[0]
        );
    }
}

fn heroes(g: &Value) -> Vec<HeroItem> {
    arr(g, "items")
        .iter()
        .map(|v| HeroItem::from_json(v).unwrap())
        .collect()
}

#[test]
fn pools_match_js() {
    let g = g();
    let h = heroes(&g);
    for c in arr(&g, "saverPool") {
        let n = c["n"].as_u64().unwrap() as usize;
        let got = billboard::saver_pool(&h[..n], opt_str(&c["keep"]));
        let got: Vec<String> = got.iter().map(|i| i.item.id.clone()).collect();
        assert_eq!(got, id_list(&c["out"]), "n={n} keep={}", c["keep"]);
    }
    let cases = arr(&g, "buildPool");
    assert!(cases.len() >= 12);
    for c in cases {
        let mut rng = dayseed::seeded_random(s(&c["seed"]));
        let opts = PoolOpts::home(opt_str(&c["avoid"]).map(str::to_owned));
        let got = billboard::build_pool(&h[..40], &h[40..100], &opts, &mut || rng.next());
        let got: Vec<String> = got.iter().map(|i| i.item.id.clone()).collect();
        assert_eq!(
            got,
            id_list(&c["out"]),
            "seed={} avoid={}",
            c["seed"],
            c["avoid"]
        );

        let mut rng = dayseed::seeded_random(s(&c["seed"]));
        let got = billboard::build_pool(&h[..40], &h[40..100], &PoolOpts::library(), &mut || {
            rng.next()
        });
        let got: Vec<String> = got.iter().map(|i| i.item.id.clone()).collect();
        assert_eq!(got, id_list(&c["lib"]), "lib seed={}", c["seed"]);
    }
}

#[test]
fn row_logic_matches_js() {
    let g = g();
    let all = items(&g);
    let n = now(&g);
    for (i, e) in arr(&g, "isRecent").iter().enumerate() {
        let it = &all[i].1;
        assert_eq!(
            logic::is_recent(it.date_created.as_deref(), n, 14.0),
            e[1].as_bool().unwrap(),
            "{}",
            e[0]
        );
        assert_eq!(
            logic::is_recent(it.premiere_date.as_deref(), n, 365.0),
            e[2].as_bool().unwrap(),
            "{}",
            e[0]
        );
    }
    let base: Vec<BaseItem> = all.iter().map(|x| x.1.clone()).collect();
    let set: HashSet<String> = logic::new_series_ids_from(&base, n);
    let mut got: Vec<String> = set.iter().cloned().collect();
    got.sort();
    assert_eq!(got, id_list(&g["newSeriesIds"]));
    for (i, e) in arr(&g, "rowBadge").iter().enumerate() {
        assert_eq!(
            logic::badge_for(&base[i], Some(&set), n),
            opt_str(&e[1]),
            "{}",
            e[0]
        );
    }
    let lists = [&base[..], &base[..30], &base[100..]];
    for (l, want) in lists.iter().zip(arr(&g, "top10")) {
        assert_eq!(ids(&logic::select_top10(l)), id_list(want));
    }
    for (i, e) in arr(&g, "progress").iter().enumerate() {
        assert_eq!(
            logic::progress_percent(&base[i]),
            e[1].as_f64().unwrap(),
            "{}",
            e[0]
        );
    }
}

#[test]
fn new_popular_matches_js() {
    let g = g();
    let all = items(&g);
    let base: Vec<BaseItem> = all.iter().map(|x| x.1.clone()).collect();
    let n = now(&g);
    let set = logic::new_series_ids_from(&base, n);
    for (i, e) in arr(&g, "dateParts").iter().enumerate() {
        let got = newpopular::date_parts(base[i].date_created.as_deref(), 0);
        match got {
            Some((m, d)) => {
                assert_eq!(
                    (m, d.as_str()),
                    (s(&e[1]["month"]), s(&e[1]["day"])),
                    "{}",
                    e[0]
                );
            }
            None => assert!(e[1].is_null(), "{}", e[0]),
        }
    }
    for (i, e) in arr(&g, "sinceLabel").iter().enumerate() {
        assert_eq!(
            newpopular::since_label(&base[i], Some(&set), n, 0),
            s(&e[1]),
            "{}",
            e[0]
        );
    }
    let dup: Vec<BaseItem> = base.iter().chain(base[..20].iter()).cloned().collect();
    for c in arr(&g, "uniqueTitles") {
        let lim = c["n"].as_u64().unwrap() as usize;
        assert_eq!(
            ids(&newpopular::unique_titles(&dup, lim)),
            id_list(&c["out"])
        );
    }
    let lists = [&base[..5], &base[..40], &base[..0]];
    for (l, want) in lists.iter().zip(arr(&g, "hasPlayCounts")) {
        assert_eq!(newpopular::has_play_counts(l), want.as_bool().unwrap());
    }

    let info = newpopular::new_content_info(Some(&base), Some(&base), n);
    // Das Web zählt Filme/Serien für die Neu-Info nur über `isRecent` mit Typ.
    let mut movies: Vec<String> = info.new_movie_ids.iter().cloned().collect();
    movies.sort();
    assert_eq!(movies, id_list(&g["newMovieIds"]));
    let mut series: Vec<String> = info.new_series_ids.iter().cloned().collect();
    series.sort();
    assert_eq!(series, id_list(&g["newSeriesIdsInfo"]));
    let want = g["newContent"].as_object().unwrap();
    assert_eq!(info.by_series.len(), want.len());
    for (k, v) in want {
        let kind = match info.by_series[k] {
            newpopular::NewKind::Season => "season",
            newpopular::NewKind::Episode => "episode",
        };
        assert_eq!(kind, v.as_str().unwrap(), "{k}");
    }
    for (i, e) in arr(&g, "newBadges").iter().enumerate() {
        assert_eq!(
            newpopular::new_badge(&base[i], &info),
            opt_str(&e[1]),
            "{}",
            e[0]
        );
        assert_eq!(
            newpopular::plus_badge(&base[i], &info),
            opt_str(&e[2]),
            "{}",
            e[0]
        );
    }
}

#[test]
fn images_and_scales_match_js() {
    let g = g();
    let all = items(&g);
    for (i, e) in arr(&g, "landscape").iter().enumerate() {
        let got = pick_landscape(&all[i].1);
        match (&got, e[1].is_null()) {
            (None, true) => {}
            (Some(p), false) => {
                assert_eq!(
                    (p.id.as_str(), p.kind.as_str(), p.tag.as_str()),
                    (s(&e[1]["id"]), s(&e[1]["type"]), s(&e[1]["tag"])),
                    "{}",
                    e[0]
                );
            }
            _ => panic!("{}: {got:?} vs {}", e[0], e[1]),
        }
    }
    for (i, e) in arr(&g, "logo").iter().enumerate() {
        match (pick_logo(&all[i].1), e[1].is_null()) {
            (None, true) => {}
            (Some(p), false) => {
                assert_eq!(p.kind, ImageType::Logo);
                let url = s(&e[1]["url"]);
                assert!(
                    url.contains(&format!("/Items/{}/Images/Logo", p.id)),
                    "{url}"
                );
                assert!(url.contains(&format!("tag={}", p.tag)), "{url}");
            }
            (got, _) => panic!("{}: {got:?} vs {}", e[0], e[1]),
        }
    }
    let bases = [64u32, 300, 400, 420, 600, 720, 1000, 1600];
    for c in arr(&g, "imageScale") {
        let scale = c["scale"].as_f64().unwrap();
        for (w, want) in bases.iter().zip(c["widths"].as_array().unwrap()) {
            assert_eq!(
                u64::from(scaled(*w, scale)),
                want.as_u64().unwrap(),
                "scale={scale} w={w}"
            );
        }
    }
    for c in arr(&g, "imgscaleStore") {
        let mut sc = ImageScales::default();
        sc.set(c["in"][0].as_f64(), c["in"][1].as_f64());
        assert_eq!(sc.normal(), c["normal"].as_f64().unwrap(), "{}", c["in"]);
        assert_eq!(sc.saver(), c["saver"].as_f64().unwrap(), "{}", c["in"]);
    }
}

#[test]
fn routes_and_sorts_match_js() {
    let g = g();
    for e in arr(&g, "libraryHref") {
        let kind = LibraryKind::from_slug(s(&e[0])).unwrap_or(LibraryKind::Movies);
        let o = &e[1];
        let route = LibraryRoute {
            kind,
            genre: o["genre"].as_str().map(str::to_owned),
            view: if o["view"] == "grid" {
                ViewMode::Grid
            } else {
                ViewMode::Rows
            },
            sort: o["sort"].as_str().and_then(LibSort::from_slug),
        };
        assert_eq!(route.href(), s(&e[2]), "{}", e[1]);
    }
    for e in arr(&g, "myListHref") {
        let o = &e[0];
        let route = MyListRoute {
            kind: match o["type"].as_str() {
                Some("movies") => Some(MyListType::Movies),
                Some("series") => Some(MyListType::Series),
                _ => None,
            },
            sort: o["sort"].as_str().and_then(LibSort::from_slug),
        };
        assert_eq!(route.href(), s(&e[1]), "{o}");
    }
    for e in arr(&g, "parseRoute") {
        let got = parse_route(&format!("#/home?{}", s(&e[0])));
        let want = &e[1];
        match want["kind"].as_str().unwrap_or("") {
            "series" | "movies" => {
                let Some(LfxRoute::Library(r)) = got else {
                    panic!("{}: {got:?}", e[0])
                };
                assert_eq!(r.kind.slug(), want["kind"].as_str().unwrap());
                assert_eq!(r.genre.as_deref(), opt_str(&want["genre"]), "{}", e[0]);
                assert_eq!(r.view == ViewMode::Grid, want["view"] == "grid");
                assert_eq!(r.sort.map(LibSort::slug), opt_str(&want["sort"]));
            }
            "mylist" => {
                let Some(LfxRoute::MyList(r)) = got else {
                    panic!("{}: {got:?}", e[0])
                };
                assert_eq!(
                    r.kind.map(MyListType::slug),
                    opt_str(&want["type"]),
                    "{}",
                    e[0]
                );
                assert_eq!(r.sort.map(LibSort::slug), opt_str(&want["sort"]));
            }
            _ => assert!(got.is_none(), "{}: {got:?}", e[0]),
        }
    }
    for e in arr(&g, "sortParams") {
        let sort = e[0]
            .as_str()
            .and_then(LibSort::from_slug)
            .unwrap_or(LibSort::Az);
        let (by, order) = sort_params(sort, s(&e[1]) == "Series");
        assert_eq!(
            (by, order),
            (s(&e[2]["SortBy"]), s(&e[2]["SortOrder"])),
            "{e}"
        );
    }
}

#[test]
fn remote_trailer_rule_matches_js() {
    for e in arr(&g(), "remoteTrailer") {
        let urls = id_list(&e[0]);
        assert_eq!(
            billboard::direct_remote_trailer_url(&urls),
            opt_str(&e[1]),
            "{}",
            e[0]
        );
    }
}

#[test]
fn item_kinds_in_vectors_are_decoded() {
    // Schutz gegen leere Vektoren: die Mischung muss mehrere Arten enthalten.
    let kinds: HashSet<ItemKind> = items(&g()).iter().map(|x| x.1.kind).collect();
    assert!(kinds.len() >= 4, "{kinds:?}");
}
