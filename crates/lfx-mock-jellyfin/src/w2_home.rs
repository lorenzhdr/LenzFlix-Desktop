//! Erweiterungen des Mocks für die Startseite (W2-home), alle nur auf Wunsch (Standard: aus, Verhalten
//! unverändert): frische Erstellungsdaten (Neu-Badges), abwechslungsreiche Platzhalter-Cover, ein lokaler
//! Trailer je Film, eine volle Chart-Liste und vorbelegte Weiterschauen-Positionen.
//!
//! Aktivierung vor dem Start des Mocks per [`set_options`] (gilt prozessweit; Tests laufen einzeln).

use super::catalog::{Item, TICKS_PER_SECOND, UserState};
use axum::Json;
use axum::extract::Path;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Mutex;

/// Schalter der Startseiten-Erweiterungen.
#[derive(Debug, Clone, Copy, Default)]
pub struct W2Options {
    /// `DateCreated` relativ zu „jetzt“ (einige Titel jünger als 14 Tage: „Neu“-Badges).
    pub fresh_dates: bool,
    /// Verschiedenfarbige Platzhalter-Cover je Titel (Standard: ein einheitliches Bild).
    pub varied_covers: bool,
    /// Jeder Film hat einen lokalen Trailer (`LocalTrailerCount`, `LocalTrailers`, `stream.mp4`).
    pub trailers: bool,
    /// Chart-Listen mit allen Filmen (statt drei) für die Top-10-Zeile.
    pub charts_full: bool,
    /// Drei Filme mit Weiterschauen-Position.
    pub resume_demo: bool,
}

static OPTS: Mutex<W2Options> = Mutex::new(W2Options {
    fresh_dates: false,
    varied_covers: false,
    trailers: false,
    charts_full: false,
    resume_demo: false,
});

/// Setzt die Schalter (vor [`crate::start_with`] aufrufen).
pub fn set_options(o: W2Options) {
    *OPTS.lock().unwrap_or_else(|e| e.into_inner()) = o;
}

fn opts() -> W2Options {
    *OPTS.lock().unwrap_or_else(|e| e.into_inner())
}

fn iso_from_days_ago(days: i64, hour: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let d = now.div_euclid(86_400) - days;
    // Bürgerliches Datum aus Tagen seit 1970 (Howard Hinnant).
    let z = d + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{day:02}T{hour:02}:00:00.0000000Z")
}

fn nth(id: &str) -> i64 {
    i64::from_str_radix(&id[id.len().saturating_sub(4)..], 16).unwrap_or(0)
}

/// Nachbearbeitung eines gerenderten Titels (nach `catalog::render`).
pub fn adjust_item(v: &mut Value, it: &Item) {
    let o = opts();
    if o.fresh_dates && matches!(it.kind, "Movie" | "Series") {
        // Jeder dritte Titel ist neu (1 bis 10 Tage), der Rest älter als 14 Tage.
        let n = nth(&it.id);
        let days = if n % 3 == 0 { 1 + n % 10 } else { 20 + n % 40 };
        v["DateCreated"] = json!(iso_from_days_ago(days, 10));
    }
    if o.trailers && it.kind == "Movie" {
        v["LocalTrailerCount"] = json!(1);
    }
}

/// `/Items/{id}/LocalTrailers`: ein Trailer, dessen Strom der Film selbst ist (Mock).
pub async fn local_trailers(Path(id): Path<String>) -> Response {
    if !opts().trailers {
        return Json(json!([])).into_response();
    }
    Json(
        json!([{ "Id": id, "Name": "Trailer", "Type": "Trailer", "MediaType": "Video",
                  "RunTimeTicks": 40 * TICKS_PER_SECOND }]),
    )
    .into_response()
}

/// Chart-Antwort mit allen Filmen und beiden Serien, wenn eingeschaltet.
pub fn charts_json(catalog: &[Item]) -> Option<Value> {
    if !opts().charts_full {
        return None;
    }
    let entry = |i: &Item, rank: i64| json!({ "id": i.id, "type": i.kind, "viewers": 20 - rank, "plays": 30 - rank, "seconds": 3600 * (20 - rank) });
    let movies: Vec<Value> = catalog
        .iter()
        .filter(|i| i.kind == "Movie")
        .enumerate()
        .map(|(n, i)| entry(i, n as i64))
        .take(10)
        .collect();
    let series: Vec<Value> = catalog
        .iter()
        .filter(|i| i.kind == "Series")
        .enumerate()
        .map(|(n, i)| entry(i, n as i64))
        .collect();
    Some(
        json!({ "version": 1, "generatedAt": "2026-01-01T00:00:00Z", "windowDays": 30,
                 "metric": "viewers", "movies": movies, "series": series, "all": movies }),
    )
}

/// Platzhalter-Cover je Titel (nur mit `varied_covers`); Größe nach Bildart und `maxWidth`.
pub fn cover(id: &str, kind: &str, params: &HashMap<String, String>) -> Option<Vec<u8>> {
    if !opts().varied_covers {
        return None;
    }
    let get = |k: &str| {
        params
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(k))
            .and_then(|(_, v)| v.parse::<u32>().ok())
    };
    let seed = nth(id) as u32;
    match kind {
        "Primary" => {
            let w = get("maxWidth").unwrap_or(300).clamp(60, 360);
            Some(super::media::cover_png(w, w * 3 / 2, seed))
        }
        "Backdrop" | "Thumb" => {
            let w = get("maxWidth").unwrap_or(640).clamp(120, 720);
            Some(super::media::cover_png(w, w * 9 / 16, seed))
        }
        _ => None,
    }
}

/// Startzustand: drei Filme mit Weiterschauen-Position (nur mit `resume_demo`).
pub fn seed_state(users: &mut UserState, catalog: &[Item]) {
    if !opts().resume_demo {
        return;
    }
    for (n, it) in catalog
        .iter()
        .filter(|i| i.kind == "Movie")
        .take(3)
        .enumerate()
    {
        let runtime = it.runtime_min * 60 * TICKS_PER_SECOND;
        users
            .positions
            .insert(it.id.clone(), runtime * (2 + n as i64) / 6);
    }
}
