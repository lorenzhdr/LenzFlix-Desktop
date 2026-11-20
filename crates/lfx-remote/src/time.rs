//! Zeit in Mikrosekunden seit der Unix-Epoche (UTC), RFC-3339-Umwandlung und Uhr-Abstraktion.
//!
//! Alle SyncPlay-Rechnungen laufen über [`Clock`], damit Tests eine simulierte Uhr einsetzen können.

use serde::{Deserialize, Deserializer, Serializer};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Jellyfin-Ticks (100 ns) je Mikrosekunde.
pub const TICKS_PER_MICRO: i64 = 10;
/// Jellyfin-Ticks je Sekunde.
pub const TICKS_PER_SECOND: i64 = 10_000_000;

/// Quelle für „jetzt“ (UTC, Mikrosekunden seit 1970).
pub trait Clock: Send + Sync + 'static {
    /// Aktuelle Zeit.
    fn now_micros(&self) -> i64;
}

/// Systemuhr.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_micros(&self) -> i64 {
        match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(d) => d.as_micros() as i64,
            Err(e) => -(e.duration().as_micros() as i64),
        }
    }
}

/// Von Hand gestellte Uhr (Tests, simulierte Clients). Klone teilen den Stand.
#[derive(Debug, Clone, Default)]
pub struct ManualClock(Arc<AtomicI64>);

impl ManualClock {
    /// Neue Uhr mit Startwert.
    pub fn new(start_micros: i64) -> Self {
        Self(Arc::new(AtomicI64::new(start_micros)))
    }
    /// Setzt die Zeit.
    pub fn set(&self, micros: i64) {
        self.0.store(micros, Ordering::SeqCst);
    }
    /// Stellt die Uhr um `micros` vor.
    pub fn advance(&self, micros: i64) {
        self.0.fetch_add(micros, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now_micros(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

/// Tage seit 1970-01-01 für ein Datum im proleptischen gregorianischen Kalender.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Umkehrung von [`days_from_civil`].
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

/// Liest `YYYY-MM-DDTHH:MM:SS[.ffffff…](Z|±HH:MM)` als Mikrosekunden seit 1970 (UTC).
/// Ohne Zeitzonenangabe gilt UTC (so sendet es Jellyfin gelegentlich). Brüche über 6 Stellen werden abgeschnitten.
pub fn parse_rfc3339(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    let num = |from: usize, len: usize| -> Option<i64> {
        let part = b.get(from..from + len)?;
        if !part.iter().all(u8::is_ascii_digit) {
            return None;
        }
        std::str::from_utf8(part).ok()?.parse().ok()
    };
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || !matches!(b[10], b'T' | b't' | b' ') {
        return None;
    }
    if b[13] != b':' || b[16] != b':' {
        return None;
    }
    let (y, mo, d) = (num(0, 4)?, num(5, 2)?, num(8, 2)?);
    let (h, mi, sec) = (num(11, 2)?, num(14, 2)?, num(17, 2)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    let mut i = 19;
    let mut micros = 0i64;
    if b.get(i) == Some(&b'.') {
        i += 1;
        let mut scale = 100_000i64;
        while let Some(c) = b.get(i).filter(|c| c.is_ascii_digit()) {
            micros += i64::from(c - b'0') * scale;
            scale /= 10;
            i += 1;
        }
    }
    let offset_secs = match b.get(i) {
        None | Some(b'Z' | b'z') => 0,
        Some(sign @ (b'+' | b'-')) => {
            let oh = num(i + 1, 2)?;
            let colon = usize::from(b.get(i + 3) == Some(&b':'));
            let om = num(i + 3 + colon, 2)?;
            let secs = oh * 3600 + om * 60;
            if *sign == b'+' { secs } else { -secs }
        }
        _ => return None,
    };
    let days = days_from_civil(y, mo, d);
    Some(((days * 86_400 + h * 3600 + mi * 60 + sec - offset_secs) * 1_000_000) + micros)
}

/// Schreibt Mikrosekunden als `YYYY-MM-DDTHH:MM:SS.ffffffZ`.
pub fn format_rfc3339(micros: i64) -> String {
    let secs = micros.div_euclid(1_000_000);
    let frac = micros.rem_euclid(1_000_000);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{frac:06}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Serde-Modul für Zeitstempel als Mikrosekunden (`#[serde(with = "crate::time::rfc3339")]`).
pub mod rfc3339 {
    use super::*;

    /// Schreibt RFC 3339.
    pub fn serialize<S: Serializer>(v: &i64, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&format_rfc3339(*v))
    }

    /// Liest RFC 3339.
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
        let text = String::deserialize(d)?;
        parse_rfc3339(&text)
            .ok_or_else(|| serde::de::Error::custom(format!("ungültiger Zeitstempel: {text}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_known_value() {
        // Erfundener Wert; Sekunden seit 1970 unabhängig nachgerechnet.
        let t = parse_rfc3339("2026-11-20T12:34:56.1234567Z").unwrap();
        assert_eq!(t, 1_795_178_096_123_456);
        assert_eq!(format_rfc3339(t), "2026-11-20T12:34:56.123456Z");
        assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_rfc3339("1969-12-31T23:59:59.5Z"), Some(-500_000));
        assert_eq!(format_rfc3339(-500_000), "1969-12-31T23:59:59.500000Z");
    }

    #[test]
    fn offsets_and_defaults() {
        let utc = parse_rfc3339("2026-03-01T10:00:00Z").unwrap();
        assert_eq!(parse_rfc3339("2026-03-01T11:00:00+01:00"), Some(utc));
        assert_eq!(parse_rfc3339("2026-03-01T04:30:00-0530"), Some(utc));
        assert_eq!(parse_rfc3339("2026-03-01T10:00:00"), Some(utc));
        assert_eq!(parse_rfc3339("kaputt"), None);
        assert_eq!(parse_rfc3339("2026-13-01T10:00:00Z"), None);
    }

    #[test]
    fn leap_day() {
        let t = parse_rfc3339("2028-02-29T00:00:00Z").unwrap();
        assert_eq!(format_rfc3339(t), "2028-02-29T00:00:00.000000Z");
    }

    #[test]
    fn manual_clock_shared() {
        let a = ManualClock::new(5);
        let b = a.clone();
        b.advance(10);
        assert_eq!(a.now_micros(), 15);
    }
}
