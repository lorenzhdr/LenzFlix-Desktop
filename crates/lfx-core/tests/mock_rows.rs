//! Integrationstests: Zeilenabfragen gegen den Mock-Server (nur 127.0.0.1, erfundene Daten).
//!
//! Mock-Lücken (nicht geändert, im Bericht): `GenreIds`, `ImageTypes`, `GroupItems` werden ignoriert;
//! nur der erste `SortBy`-Schlüssel zählt und `SortOrder=Descending,Ascending` gilt nicht als absteigend;
//! `Random` liefert Katalogreihenfolge; `/Genres` ohne Zähler; 2 bis 3 Titel je Genre.

use lfx_api::{BaseItem, Client, ClientInfo, ItemKind};
use lfx_core::billboard::{self, HeroItem, PoolOpts};
use lfx_core::library::{
    self, GridPager, LibSort, LibraryInput, LibraryKind, MyListRoute, build_library_rows,
};
use lfx_core::newpopular;
use lfx_core::rows::home::{HomeDefs, HomeInput, build_home_defs, genres_call, views_call};
use lfx_core::rows::{RowDef, RowKind, RowResult, RowSource, load_row};
use lfx_mock_jellyfin::{DEMO_PASSWORD, DEMO_USER, sample_movie_id, sample_series_id, start};

async fn logged_in() -> (Client, lfx_mock_jellyfin::MockHandle) {
    let (addr, handle) = start().await;
    let client = Client::new(
        &format!("http://{addr}"),
        ClientInfo::new("Test-Rechner", "00000000-0000-4000-8000-000000000000"),
    )
    .unwrap();
    client.login(DEMO_USER, DEMO_PASSWORD).await.unwrap();
    (client, handle)
}

async fn home(client: &Client) -> HomeDefs {
    let views = views_call().execute(client).await.unwrap();
    let genres = genres_call().execute(client).await.unwrap();
    build_home_defs(&HomeInput {
        views: &views,
        genres: &genres,
        lookups_ok: true,
        seed: "2026-11-20",
        genre_count: 6,
        brand: "LENZFLIX",
        charts_configured: false,
        charts: None,
    })
}

async fn load(client: &Client, defs: &[RowDef], id: &str) -> RowResult {
    let def = defs
        .iter()
        .find(|d| d.id == id)
        .unwrap_or_else(|| panic!("Zeile {id}"));
    load_row(client, def).await.unwrap()
}

fn heroes(values: &[serde_json::Value]) -> Vec<HeroItem> {
    values.iter().filter_map(HeroItem::from_json).collect()
}

#[tokio::test]
async fn home_rows_load_with_web_rules() {
    let (client, _h) = logged_in().await;
    let defs = home(&client).await;
    let rows = &defs.rows;
    assert!(!defs.degraded);
    let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
    assert!(
        ids.contains(&"latest-movies") && ids.contains(&"latest-series"),
        "{ids:?}"
    );

    // Neu hinzugefügt: nur Filme, neueste zuerst (Mock: Erstellungsdatum steigt mit der Nummer).
    let lm = load(&client, rows, "latest-movies").await;
    assert!(lm.items.len() >= 10 && lm.items.iter().all(|i| i.kind == ItemKind::Movie));
    let created: Vec<_> = lm
        .items
        .iter()
        .map(|i| i.date_created.clone().unwrap())
        .collect();
    assert!(created.windows(2).all(|w| w[0] >= w[1]), "neueste zuerst");

    // Top-10-Ersatz: höchstens 10, neueste zuerst.
    let top = load(&client, rows, "top10").await;
    assert_eq!(top.items.len(), 10);
    let d: Vec<_> = top
        .items
        .iter()
        .map(|i| i.date_created.clone().unwrap())
        .collect();
    assert!(d.windows(2).all(|w| w[0] >= w[1]));

    // Hochbewertet: Bewertung >= 7,5, absteigend.
    let tr = load(&client, rows, "toprated").await;
    assert!(!tr.items.is_empty());
    assert!(tr.items.iter().all(|i| i.community_rating.unwrap() >= 7.5));
    let r: Vec<f64> = tr
        .items
        .iter()
        .map(|i| i.community_rating.unwrap())
        .collect();
    assert!(r.windows(2).all(|w| w[0] >= w[1]));

    // Genre-Zeilen: Mock hat je Genre weniger als 8 Titel, also leer (Regel „mindestens 8“).
    for d in rows.iter().filter(|d| d.id.starts_with("genre-")) {
        assert!(
            load_row(&client, d).await.unwrap().items.is_empty(),
            "{}",
            d.id
        );
    }

    // Leerer Konto-Zustand: Meine Liste leer, „Weil du“ ohne Gesehenes leer.
    assert!(load(&client, rows, "favorites").await.items.is_empty());
    assert!(load(&client, rows, "because").await.items.is_empty());
}

#[tokio::test]
async fn user_state_rows_follow_account() {
    let (client, _h) = logged_in().await;
    let defs = home(&client).await;
    let fav = sample_movie_id(2);
    client.set_favorite(&fav, true).await.unwrap();
    let f = load(&client, &defs.rows, "favorites").await;
    assert_eq!(f.items.len(), 1);
    assert_eq!(f.items[0].id, fav);

    // „Weil du … angesehen hast“: Titel aus dem zuletzt Gesehenen, ohne den Ausgangstitel selbst.
    let seen = sample_movie_id(0);
    client.set_played(&seen, true).await.unwrap();
    let b = load(&client, &defs.rows, "because").await;
    assert_eq!(
        b.title.as_deref(),
        Some("Weil du Die Nebelkuste angesehen hast")
    );
    assert!(!b.items.is_empty() && b.items.iter().all(|i| i.id != seen));

    // „Als Nächstes“: nach einer gesehenen Folge die erste ungesehene.
    let series = sample_series_id(0);
    let eps = client.episodes(&series, None, &[]).await.unwrap();
    client.set_played(&eps.items[0].id, true).await.unwrap();
    let n = load(&client, &defs.rows, "nextup").await;
    assert_eq!(n.items.len(), 1);
    assert_eq!(n.items[0].id, eps.items[1].id);
}

#[tokio::test]
async fn library_rows_and_pool_against_mock() {
    let (client, _h) = logged_in().await;
    let genres = library::genres_from(
        &library::genres_call(LibraryKind::Movies)
            .execute(&client)
            .await
            .unwrap(),
    );
    assert!(!genres.is_empty());
    let input = LibraryInput {
        kind: LibraryKind::Movies,
        genre: None,
        genres: &genres,
        day_key: "2026-11-20",
        brand: "LENZFLIX",
        charts_configured: false,
        charts: None,
    };
    let defs = build_library_rows(&input);
    let latest = defs
        .iter()
        .find(|d| d.id == "lib:movies:all:latest")
        .unwrap();
    let r = load_row(&client, latest).await.unwrap();
    assert!(!r.items.is_empty() && r.items.iter().all(|i| i.kind == ItemKind::Movie));
    let top = defs
        .iter()
        .find(|d| d.id == "lib:movies:all:top10")
        .unwrap();
    assert_eq!(top.kind, RowKind::Top10);
    assert!(matches!(top.source, RowSource::TopNew { .. }));
    assert_eq!(load_row(&client, top).await.unwrap().items.len(), 10);

    // Serien-Seite: Weiterschauen liefert nur Folgen (Art-Filter).
    let sdefs = build_library_rows(&LibraryInput {
        kind: LibraryKind::Series,
        ..input.clone()
    });
    let resume = sdefs
        .iter()
        .find(|d| d.id == "lib:series:all:resume")
        .unwrap();
    assert!(
        load_row(&client, resume)
            .await
            .unwrap()
            .items
            .iter()
            .all(|i| i.kind == ItemKind::Episode)
    );

    // Hero-Pool: 3 neue + 7 zufällige, keine Doppelten.
    let (lc, rc) = billboard::library_pool_calls(ItemKind::Movie, None);
    let l = heroes(&lc.execute_values(&client).await.unwrap());
    let rr = heroes(&rc.execute_values(&client).await.unwrap());
    let mut n = 0.0;
    let pool = billboard::build_pool(&l, &rr, &PoolOpts::library(), &mut || {
        n = (n + 0.37) % 1.0;
        n
    });
    assert_eq!(pool.len(), 3 + 7);
    let mut seen: Vec<&str> = pool.iter().map(|p| p.item.id.as_str()).collect();
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(seen.len(), pool.len());
}

#[tokio::test]
async fn home_pool_and_new_content() {
    let (client, _h) = logged_in().await;
    let (lc, rc) = billboard::home_pool_calls();
    let l = heroes(&lc.execute_values(&client).await.unwrap());
    let r = heroes(&rc.execute_values(&client).await.unwrap());
    assert_eq!(l.len(), 12);
    let pool = billboard::build_pool(&l, &r, &PoolOpts::home(None), &mut || 0.25);
    assert!(pool.len() >= 8);
    assert!(pool.iter().all(|p| p.item.image_tags.contains_key("Logo")));

    let (ec, tc) = newpopular::new_content_calls();
    let eps = ec.execute(&client).await.unwrap();
    let titles = tc.execute(&client).await.unwrap();
    assert!(eps.iter().all(|e| e.kind == ItemKind::Episode));
    // Mock-Daten stammen aus 2026-03/04: mit „jetzt“ im Mai ist nichts neu, 10. April schon.
    let may = 1_779_000_000_000.0;
    assert!(
        newpopular::new_content_info(Some(&eps), Some(&titles), may)
            .new_movie_ids
            .is_empty()
    );
    let april = 1775822400000.0;
    assert!(
        !newpopular::new_content_info(Some(&eps), Some(&titles), april)
            .by_series
            .is_empty()
    );

    let new = newpopular::new_call().execute(&client).await.unwrap();
    assert_eq!(new.len(), 14, "12 Filme + 2 Serien im Mock");
}

#[tokio::test]
async fn grid_paging_and_mylist() {
    let (client, _h) = logged_in().await;
    let mut pager = GridPager::new();
    let mut all: Vec<BaseItem> = Vec::new();
    let page = 5;
    while pager.can_load() {
        let seq = pager.begin();
        let call =
            library::grid_page_call(LibraryKind::Movies, None, LibSort::Za, pager.count, page);
        let (items, total) = call.execute_page(&client).await.unwrap();
        pager.apply(seq, items.len(), total, page);
        all.extend(items);
    }
    assert!(pager.done);
    assert_eq!(all.len(), pager.total.unwrap());
    assert_eq!(all.len(), 12, "Mock hat 12 Filme");
    let n: Vec<String> = all.iter().map(|i| i.name.to_lowercase()).collect();
    assert!(n.windows(2).all(|w| w[0] >= w[1]), "Z–A");
    let mut ids: Vec<&str> = all.iter().map(|i| i.id.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 12, "Seiten überlappen nicht");

    // Meine Liste: leer, dann mit Favoriten; Typfilter.
    let route = MyListRoute::default();
    let (none, total) = library::mylist_page_call(&route, 0, 40)
        .execute_page(&client)
        .await
        .unwrap();
    assert!(none.is_empty() && total == Some(0));
    client
        .set_favorite(&sample_movie_id(1), true)
        .await
        .unwrap();
    client
        .set_favorite(&sample_series_id(0), true)
        .await
        .unwrap();
    let (both, total) = library::mylist_page_call(&route, 0, 40)
        .execute_page(&client)
        .await
        .unwrap();
    assert_eq!((both.len(), total), (2, Some(2)));
    let only_series = MyListRoute {
        kind: Some(library::MyListType::Series),
        sort: None,
    };
    let (s, _) = library::mylist_page_call(&only_series, 0, 40)
        .execute_page(&client)
        .await
        .unwrap();
    assert_eq!(s.len(), 1);
    assert_eq!(s[0].kind, ItemKind::Series);
}

#[tokio::test]
async fn failing_server_surfaces_error() {
    let (client, handle) = logged_in().await;
    let defs = home(&client).await;
    handle.fail_next(10, 500);
    let def = defs.rows.iter().find(|d| d.id == "toprated").unwrap();
    assert!(load_row(&client, def).await.is_err());
}
