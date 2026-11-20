//! Tages-Seed und deterministische Mischung (Port von `core/dayseed.js` und `rows/logic.js`).
//!
//! `hashString` ist FNV-1a über UTF-16-Code-Units, `seededRandom` mulberry32; beide Varianten im Web
//! (`dayseed.js` und `rows/logic.js`) rechnen identisch, hier gibt es eine Umsetzung (Golden-Tests prüfen beide).

use crate::collate::de_compare;
use lfx_tv_schedule::hash::{Mulberry32, fnv1a, mulberry32};

/// `dayKey`: `YYYY-MM-DD` aus einem lokalen Kalenderdatum (Jahr ohne Auffüllen wie `getFullYear()`).
pub fn day_key(year: i64, month: i64, day: i64) -> String {
    format!("{year}-{month:02}-{day:02}")
}

/// `dayKey` aus Unix-Millisekunden und Zeitzonenversatz in Minuten (Ost positiv), ohne Systemuhr.
pub fn day_key_from_epoch_ms(epoch_ms: i64, utc_offset_min: i64) -> String {
    let local = epoch_ms + utc_offset_min * 60_000;
    let days = local.div_euclid(86_400_000);
    let (y, m, d) = lfx_tv_schedule::time::civil_from_days(days);
    day_key(y, m, d)
}

/// FNV-1a (32 Bit) über UTF-16-Code-Units (`hashString`).
pub fn hash_string(s: &str) -> u32 {
    fnv1a(s)
}

/// Zufallsfolge [0,1) aus einem Seed-String (`seededRandom`).
pub fn seeded_random(seed: &str) -> Mulberry32 {
    mulberry32(f64::from(hash_string(seed)))
}

/// Neue, per Seed gemischte Kopie (Fisher-Yates, `seededShuffle`).
pub fn seeded_shuffle<T: Clone>(list: &[T], seed: &str) -> Vec<T> {
    let mut out = list.to_vec();
    let mut rnd = seeded_random(seed);
    for i in (1..out.len()).rev() {
        let j = (rnd.next() * (i as f64 + 1.0)).floor() as usize;
        out.swap(i, j);
    }
    out
}

/// Eingabe für [`pick_genres`]: Name und optional Titelzahlen (wie `{Name, MovieCount?, SeriesCount?}`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenreInput {
    /// Genre-Name.
    pub name: String,
    /// Anzahl Filme, falls bekannt.
    pub movie_count: Option<i64>,
    /// Anzahl Serien, falls bekannt.
    pub series_count: Option<i64>,
}

impl GenreInput {
    /// Nur Name (keine Zahlen bekannt).
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            movie_count: None,
            series_count: None,
        }
    }
}

/// Standardzahl Genre-Zeilen der Startseite.
pub const DEFAULT_GENRE_COUNT: usize = 6;
/// Mindestzahl Titel, damit ein Genre mit bekannten Zahlen zählt.
pub const MIN_GENRE_TITLES: i64 = 8;

/// Wählt deterministisch (je Tag) Genre-Namen: eindeutig, mit Zahlen nur ab `min_titles`, nach
/// deutscher Sortierung geordnet, per Seed gemischt, die ersten `count`.
pub fn pick_genres(
    genres: &[GenreInput],
    seed: &str,
    count: usize,
    min_titles: i64,
) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for g in genres {
        if g.name.is_empty() {
            continue;
        }
        let known = g.movie_count.is_some() || g.series_count.is_some();
        if known && g.movie_count.unwrap_or(0) + g.series_count.unwrap_or(0) < min_titles {
            continue;
        }
        if !names.contains(&g.name) {
            names.push(g.name.clone());
        }
    }
    names.sort_by(|a, b| de_compare(a, b));
    seeded_shuffle(&names, seed)
        .into_iter()
        .take(count)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn day_key_formats() {
        assert_eq!(day_key(2026, 3, 9), "2026-03-09");
        // 2026-11-20 23:30 UTC ist in MEZ (+60) schon der 21.
        let t = 1_795_217_400_000; // 2026-11-20T23:30:00Z
        assert_eq!(day_key_from_epoch_ms(t, 0), "2026-11-20");
        assert_eq!(day_key_from_epoch_ms(t, 60), "2026-11-21");
    }

    #[test]
    fn shuffle_is_deterministic_permutation() {
        let v: Vec<u32> = (0..20).collect();
        let a = seeded_shuffle(&v, "2026-11-20");
        assert_eq!(a, seeded_shuffle(&v, "2026-11-20"));
        let mut s = a.clone();
        s.sort_unstable();
        assert_eq!(s, v);
        assert_ne!(a, v);
    }

    #[test]
    fn pick_genres_filters_small_and_dedupes() {
        let g = vec![
            GenreInput::named("Drama"),
            GenreInput::named("Drama"),
            GenreInput {
                name: "Klein".into(),
                movie_count: Some(3),
                series_count: Some(2),
            },
            GenreInput {
                name: "Groß".into(),
                movie_count: Some(3),
                series_count: Some(5),
            },
        ];
        let mut p = pick_genres(&g, "x", 6, MIN_GENRE_TITLES);
        p.sort();
        assert_eq!(p, vec!["Drama".to_string(), "Groß".to_string()]);
    }
}
