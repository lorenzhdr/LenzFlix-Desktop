//! W2 (Bibliothek): zusätzliche erfundene Titel für Raster-, Paging- und Suchtests (opt-in).
//!
//! Standardmäßig ändert sich am Katalog nichts. Wer viele Titel braucht, ruft vor dem Start des Mocks
//! [`set_extra_titles`] auf. Dann kommen zusätzliche Filme und Serien, eine Ansicht „Sammlungen“ mit
//! Boxsets (`CollectionType = boxsets`) und Kindtiteln hinzu.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::catalog::{Item, VIEW_MOVIES, VIEW_SERIES};

static EXTRA: AtomicUsize = AtomicUsize::new(0);

/// Id der Ansicht „Sammlungen“ (nur mit zusätzlichen Titeln).
pub const VIEW_BOXSETS: &str = "00000000000000000000000000500009";

/// Zahl der zusätzlichen Filme (Serien: die Hälfte). 0 = aus (Standard). Wirkt auf später gestartete Mocks.
pub fn set_extra_titles(n: usize) {
    EXTRA.store(n, Ordering::SeqCst);
}

const ADJ: [&str; 12] = [
    "Stille",
    "Gläserne",
    "Verlorene",
    "Goldene",
    "Eisige",
    "Flüsternde",
    "Nächtliche",
    "Ferne",
    "Rostige",
    "Leuchtende",
    "Wilde",
    "Sanfte",
];
const NOUN: [&str; 12] = [
    "Brücke",
    "Küste",
    "Wüste",
    "Stadt",
    "Insel",
    "Bibliothek",
    "Werkstatt",
    "Karawane",
    "Quelle",
    "Straße",
    "Mühle",
    "Lichtung",
];
const GENRES: [&str; 7] = [
    "Drama",
    "Abenteuer",
    "Komödie",
    "Krimi",
    "Science-Fiction",
    "Thriller",
    "Fantasy",
];

fn xid(prefix: u128, n: u128) -> String {
    format!("{:032x}", (prefix << 16) + n)
}

fn blank(id: String, name: String, kind: &'static str, parent: &str) -> Item {
    Item {
        id,
        name,
        kind,
        parent_id: parent.to_owned(),
        series_id: None,
        season_id: None,
        series_name: None,
        season_name: None,
        genres: Vec::new(),
        year: 2000,
        rating: 0.0,
        runtime_min: 0,
        index: None,
        parent_index: None,
        created: String::new(),
        overview: String::new(),
    }
}

/// Hängt die zusätzlichen Titel an den Katalog (nur wenn [`set_extra_titles`] > 0 gesetzt wurde).
pub(crate) fn extend(catalog: &mut Vec<Item>) {
    let n = EXTRA.load(Ordering::SeqCst);
    if n == 0 {
        return;
    }
    for i in 0..n {
        let name = format!("Die {} {}", ADJ[i % 12], NOUN[(i / 12 + i) % 12]);
        let mut it = blank(xid(0x15, i as u128 + 1), name.clone(), "Movie", VIEW_MOVIES);
        it.genres = vec![GENRES[i % GENRES.len()]];
        it.year = 1990 + (i * 7 % 35) as i32;
        it.rating = 5.0 + (i * 13 % 40) as f64 / 10.0;
        it.runtime_min = 80 + (i * 11 % 60) as i64;
        it.created = format!("2026-{:02}-{:02}T10:00:00.0000000Z", 5 + i % 6, 1 + i % 27);
        it.overview = format!("Erfundene Handlung zu „{name}“ für Testzwecke.");
        catalog.push(it);
    }
    for i in 0..(n / 2) {
        let name = format!("{} {} (Serie)", ADJ[(i + 5) % 12], NOUN[(i * 5 + 1) % 12]);
        let mut it = blank(
            xid(0x25, i as u128 + 1),
            name.clone(),
            "Series",
            VIEW_SERIES,
        );
        it.genres = vec![GENRES[(i + 2) % GENRES.len()]];
        it.year = 1995 + (i * 5 % 30) as i32;
        it.rating = 5.5 + (i * 7 % 35) as f64 / 10.0;
        it.created = format!("2026-{:02}-{:02}T11:00:00.0000000Z", 5 + i % 6, 1 + i % 27);
        it.overview = format!("Erfundene Serie „{name}“ für Testzwecke.");
        catalog.push(it);
    }
    // Ansicht „Sammlungen“ mit Boxsets; jedes Boxset hat drei Kindfilme.
    catalog.push(blank(
        VIEW_BOXSETS.to_owned(),
        "Sammlungen".to_owned(),
        "CollectionFolder",
        "root",
    ));
    for b in 0..8usize {
        let bid = xid(0x16, b as u128 + 1);
        let bname = format!("{}-Sammlung", ADJ[b]);
        let mut set = blank(bid.clone(), bname.clone(), "BoxSet", VIEW_BOXSETS);
        set.year = 2000 + b as i32;
        set.rating = 7.0;
        set.created = format!("2026-06-{:02}T09:00:00.0000000Z", b + 1);
        set.genres = vec![GENRES[b % GENRES.len()]];
        catalog.push(set);
        for c in 0..3usize {
            let mut kid = blank(
                xid(0x17, (b * 10 + c) as u128 + 1),
                format!("{bname}: Teil {}", c + 1),
                "Movie",
                &bid,
            );
            kid.genres = vec![GENRES[b % GENRES.len()]];
            kid.year = 2000 + b as i32 + c as i32;
            kid.rating = 6.0 + c as f64;
            kid.runtime_min = 100;
            kid.created = format!("2026-06-{:02}T09:30:00.0000000Z", b + 1);
            catalog.push(kid);
        }
    }
}

/// Namen der Genres in der Reihenfolge von `/Genres` (sortiert, ohne Doppelte): Grundlage der Genre-Ids.
fn genre_names(catalog: &[Item]) -> Vec<&'static str> {
    let mut names: Vec<&str> = catalog
        .iter()
        .flat_map(|i| i.genres.iter().copied())
        .collect();
    names.sort_unstable();
    names.dedup();
    names
}

/// Ergänzt die Genre-Namen um die Treffer des Parameters `genreIds` (Genre-Id zu Name wie in `/Genres`).
pub(crate) fn merge_genre_ids(
    catalog: &[Item],
    params: &HashMap<String, String>,
    mut genres: Vec<String>,
) -> Vec<String> {
    let Some(ids) = params.get("genreids") else {
        return genres;
    };
    let names = genre_names(catalog);
    for id in ids.split([',', '|']).filter(|s| !s.is_empty()) {
        let found = u32::from_str_radix(id, 16)
            .ok()
            .and_then(|v| v.checked_sub(0x7000_0000))
            .and_then(|n| names.get(n as usize));
        // Unbekannte Id: kein Treffer (der Name \0 filtert alles weg).
        genres.push(found.map_or_else(|| "\u{0}".to_owned(), |n| n.to_lowercase()));
    }
    genres
}

impl crate::MockHandle {
    /// Setzt oder entfernt einen Favoriten des Demo-Kontos (für „Meine Liste“).
    pub fn set_favorite_item(&self, id: &str, on: bool) {
        let mut d = self.shared.lock();
        if on {
            d.users.favorites.insert(id.to_owned());
        } else {
            d.users.favorites.remove(id);
        }
    }
}
