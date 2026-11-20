//! Zeit: austauschbare Uhr und ISO-8601-Umrechnung ohne Zusatzabhängigkeit (UTC, Millisekunden).

use std::time::{SystemTime, UNIX_EPOCH};

/// Uhr (für Tests austauschbar).
pub trait Clock: Send + Sync + 'static {
    /// Millisekunden seit 1970 (UTC).
    fn now_millis(&self) -> i64;
}

/// Systemuhr.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_millis(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }
}

/// Tage seit 1970 zu (Jahr, Monat, Tag), Verfahren nach Howard Hinnant.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = i64::from(if m > 2 { m - 3 } else { m + 9 });
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `2026-11-20T12:34:56.789Z` aus Millisekunden.
pub fn iso_from_millis(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let milli = ms.rem_euclid(1000);
    let days = secs.div_euclid(86_400);
    let rest = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{milli:03}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

/// Liest ISO-8601 (`…T…[.bruchteil][Z|±hh:mm]`; fehlende Zone gilt als UTC) in Millisekunden.
/// Der Server sendet bis zu sieben Nachkommastellen; mehr als drei werden abgeschnitten.
pub fn millis_from_iso(text: &str) -> Option<i64> {
    let t = text.trim();
    let (date, time) = t.split_once(['T', ' '])?;
    let mut dp = date.splitn(3, '-');
    let y: i64 = dp.next()?.parse().ok()?;
    let m: u32 = dp.next()?.parse().ok()?;
    let d: u32 = dp.next()?.parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let split = time.find(['Z', 'z', '+', '-']).unwrap_or(time.len());
    let (clock, zone) = time.split_at(split);
    let (hms, frac) = clock.split_once('.').unwrap_or((clock, ""));
    let mut tp = hms.splitn(3, ':');
    let hh: i64 = tp.next()?.parse().ok()?;
    let mm: i64 = tp.next()?.parse().ok()?;
    let ss: i64 = tp.next().unwrap_or("0").parse().ok()?;
    let mut milli = 0i64;
    for (i, c) in frac.chars().take(3).enumerate() {
        milli += i64::from(c.to_digit(10)?) * [100, 10, 1][i];
    }
    let offset = match zone.chars().next() {
        Some(sign @ ('+' | '-')) => {
            let body = &zone[1..];
            let (oh, om) = body.split_once(':').unwrap_or((body, "0"));
            let minutes = oh.parse::<i64>().ok()? * 60 + om.parse::<i64>().ok()?;
            if sign == '+' { minutes } else { -minutes }
        }
        _ => 0,
    };
    let secs = days_from_civil(y, m, d) * 86_400 + hh * 3600 + mm * 60 + ss - offset * 60;
    Some(secs * 1000 + milli)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        for ms in [
            0i64,
            1,
            951_782_400_123,
            1_795_000_000_000,
            4_102_444_800_999,
        ] {
            assert_eq!(millis_from_iso(&iso_from_millis(ms)), Some(ms));
        }
        assert_eq!(iso_from_millis(0), "1970-01-01T00:00:00.000Z");
    }

    #[test]
    fn server_format_and_zones() {
        let a = millis_from_iso("2026-11-20T12:00:00.1234567Z").unwrap();
        assert_eq!(a % 1000, 123);
        let b = millis_from_iso("2026-11-20T13:00:00+01:00").unwrap();
        assert_eq!(b, millis_from_iso("2026-11-20T12:00:00Z").unwrap());
        assert!(millis_from_iso("kaputt").is_none());
    }

    #[test]
    fn leap_day() {
        assert_eq!(
            iso_from_millis(1_709_164_800_000),
            "2024-02-29T00:00:00.000Z"
        );
    }
}
