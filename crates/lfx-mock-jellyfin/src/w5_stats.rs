//! W5-stats: erfundene Statistikdaten für `GET /cw/me/stats` (Schema `docs/stats-api.md` des Web-Themes).
//! Feste Daten (Stand 20.11.2026), deterministisch, ohne Uhr: gleiche Anfrage = gleiche Antwort.

use crate::catalog;
use serde_json::{Value, json};

const END: (i64, i64, i64) = (2026, 11, 20);

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Pseudo-Wert 0..max (deterministisch aus dem Index).
fn pseudo(i: i64, max: i64) -> i64 {
    ((i * 7919 + 13) % 11) * max / 10
}

pub(crate) fn stats_json(range: &str) -> Option<Value> {
    let (unit, scale) = match range {
        "7d" => ("day", 1),
        "30d" => ("day", 4),
        "year" => ("week", 40),
        "all" => ("month", 120),
        _ => return None,
    };
    let end = days_from_civil(END.0, END.1, END.2);
    let mut buckets = Vec::new();
    match range {
        "7d" | "30d" => {
            let n = if range == "7d" { 7 } else { 30 };
            for i in 0..n {
                let (y, m, d) = civil_from_days(end - (n - 1) + i);
                let secs = pseudo(i, 9000);
                if secs > 0 && i % 6 != 4 {
                    buckets
                        .push(json!({ "key": format!("{y:04}-{m:02}-{d:02}"), "seconds": secs }));
                }
            }
        }
        "year" => {
            for w in 0..=46 {
                let secs = pseudo(w, 40_000);
                if secs > 0 && w % 7 != 3 {
                    buckets.push(json!({ "key": format!("2026-{w:02}"), "seconds": secs }));
                }
            }
        }
        _ => {
            for k in 0..30 {
                let (y, m) = (2024 + (k + 2) / 12, (k + 2) % 12 + 1);
                let secs = pseudo(k, 150_000);
                if secs > 0 && k % 5 != 2 {
                    buckets.push(json!({ "key": format!("{y:04}-{m:02}"), "seconds": secs }));
                }
            }
        }
    }
    let mut heat = Vec::new();
    for dow in 0..7i64 {
        let weekend = dow == 0 || dow == 6;
        for hour in (if weekend { 14 } else { 19 })..24i64 {
            let secs = (1 + (dow * 3 + hour) % 5) * 600 * scale / 2;
            heat.push(json!({ "dow": dow, "hour": hour, "seconds": secs }));
        }
    }
    let total = 7200 * scale * 3;
    Some(json!({
        "version": 1, "range": range, "computedAt": "2026-11-20T18:00:00Z", "stale": false,
        "approx": false, "ignored": false,
        "totals": { "seconds": total, "plays": 6 * scale / 2 + 2, "episodes": 5 * scale / 2 + 2, "movies": scale / 2 + 1,
                    "series": 3, "activeDays": (3 * scale / 2).clamp(2, 400),
                    "firstAt": "2026-10-12 20:11:03", "lastAt": "2026-11-19 22:40:11" },
        "topSeries": [
            { "id": catalog::series_id(0), "seconds": total / 2, "episodes": 3 * scale / 2 + 1 },
            { "id": catalog::series_id(1), "seconds": total / 4, "episodes": scale + 1 },
        ],
        "topMovies": [
            { "id": catalog::movie_id(0), "seconds": total / 8, "plays": 2 },
            { "id": catalog::movie_id(1), "seconds": total / 16, "plays": 1 },
        ],
        "timeline": { "unit": unit, "buckets": buckets },
        "heatmap": heat,
        "binge": { "topDay": { "date": "2026-11-14", "seconds": 21_000 },
                   "seriesDay": { "date": "2026-11-14", "seriesId": catalog::series_id(0), "episodes": 7 } },
        "devices": [
            { "client": "LENZFLIX", "device": "Dieser Rechner", "plays": 5 * scale / 2, "seconds": total * 6 / 10 },
            { "client": "Web", "device": "Browser", "plays": scale, "seconds": total * 3 / 10 },
            { "client": "Fernseher", "device": "Wohnzimmer", "plays": 1, "seconds": total / 10 },
        ],
        "methods": { "directPlay": { "plays": 4 * scale / 2 + 1, "seconds": total * 7 / 10 },
                     "directStream": { "plays": scale / 2, "seconds": total / 5 },
                     "transcode": { "plays": 1, "seconds": total / 10 }, "other": { "plays": 0, "seconds": 0 } },
        "compare": { "rank": 2, "of": 5, "avgSeconds": total * 8 / 10 },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_ranges_and_bad_range() {
        for r in ["7d", "30d", "year", "all"] {
            let v = stats_json(r).unwrap();
            assert_eq!(v["range"], r);
            assert!(!v["timeline"]["buckets"].as_array().unwrap().is_empty());
            assert!(!v["heatmap"].as_array().unwrap().is_empty());
        }
        assert!(stats_json("x").is_none());
    }
}
