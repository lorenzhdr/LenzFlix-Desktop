//! Zeitfunktionen. Planzone MEZ/MESZ rein arithmetisch (kein Intl, keine Lokalzeit).
//! Alles in absoluten ms (UTC). `D` = Sendetag-Nummer = Tage seit 1970-01-01 des Kalendertags.
//! Ein Sendetag läuft von 05:00 bis 05:00 Planzeit.

use crate::jsval::{JsVal, js_number, js_trim, round_js};

pub const MIN: i64 = 60_000;
pub const HOUR: i64 = 3_600_000;
pub const GRID: i64 = 300_000;
pub const DAY_MS: i64 = 86_400_000;

/// Tage seit 1970-01-01 aus Kalenderdatum (Monat 1..=12, proleptisch gregorianisch).
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Kalenderdatum (Jahr, Monat 1..=12, Tag) aus Tagen seit 1970-01-01.
pub fn civil_from_days(z: i64) -> (i64, i64, i64) {
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

/// Tag des Monats (`getUTCDate`) des letzten Sonntags im 0-basierten Monat `m0`.
fn last_sun(y: i64, m0: i64) -> i64 {
    let last = days_from_civil(y, m0 + 2, 1) - 1;
    let (_, _, d) = civil_from_days(last);
    let wd = (last + 4).rem_euclid(7);
    d - wd
}

/// Offset der Planzone gegenüber UTC in ms (1 h MEZ, 2 h MESZ; letzter So im März/Oktober, 01:00 UTC).
pub fn offset_ms(u: i64) -> i64 {
    let y = civil_from_days(u.div_euclid(DAY_MS)).0;
    let s = days_from_civil(y, 3, last_sun(y, 2)) * DAY_MS + HOUR;
    let e = days_from_civil(y, 10, last_sun(y, 9)) * DAY_MS + HOUR;
    (if u >= s && u < e { 2 } else { 1 }) * HOUR
}

/// Wanduhr (Kalendertag `d`, Minute des Tages `min`, darf > 1440 sein) zu absoluten ms.
pub fn wall_to_utc(d: i64, min: i64) -> i64 {
    let l = d * DAY_MS + min * MIN;
    l - offset_ms(l - offset_ms(l))
}

/// Grenzen eines Sendetags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bounds {
    pub start: i64,
    pub end: i64,
    pub a2015: i64,
    pub a2200: i64,
    pub a2300: i64,
}

/// Start 05:00, Ende = 05:00 des Folgetags, Anker 20:15 / 22:00 / 23:00.
pub fn day_bounds(d: i64) -> Bounds {
    Bounds {
        start: wall_to_utc(d, 300),
        end: wall_to_utc(d + 1, 300),
        a2015: wall_to_utc(d, 1215),
        a2200: wall_to_utc(d, 1320),
        a2300: wall_to_utc(d, 1380),
    }
}

/// Sendetag, zu dem der Zeitpunkt `t` gehört (04:59 gehört zum Vortag).
pub fn broadcast_day_of(t: i64) -> i64 {
    let w = t + offset_ms(t);
    let d = w.div_euclid(DAY_MS);
    if w - d * DAY_MS < 5 * HOUR { d - 1 } else { d }
}

/// Sendetag-Nummer aus Kalenderdatum, Monat wie bei `Date.UTC` 0-basiert (Überläufe normalisiert,
/// Jahr 0..99 = 1900+y, Argumente werden wie `ToIntegerOrInfinity` abgeschnitten).
pub fn day_of_date(y: f64, m: f64, d: f64) -> i64 {
    let mut y = y.trunc() as i64;
    let m = m.trunc() as i64;
    let d = d.trunc() as i64;
    if (0..=99).contains(&y) {
        y += 1900;
    }
    let y = y + m.div_euclid(12);
    let m = m.rem_euclid(12);
    days_from_civil(y, m + 1, 1) + d - 1
}

/// Jahr (`getUTCFullYear`) eines Zeitstempels in ms.
pub fn year_of_ms(ms: f64) -> i64 {
    civil_from_days((ms / DAY_MS as f64).floor() as i64).0
}

fn two(n: i64) -> String {
    if n < 10 {
        format!("0{n}")
    } else {
        n.to_string()
    }
}

/// `'HH:MM'` in der Planzone.
pub fn format_clock(t: i64) -> String {
    let w = t + offset_ms(t);
    let m = (w - w.div_euclid(DAY_MS) * DAY_MS).div_euclid(MIN);
    format!("{}:{}", two(m.div_euclid(60)), two(m % 60))
}

const WD_SHORT: [&str; 7] = ["So", "Mo", "Di", "Mi", "Do", "Fr", "Sa"];
const WD_LONG: [&str; 7] = [
    "Sonntag",
    "Montag",
    "Dienstag",
    "Mittwoch",
    "Donnerstag",
    "Freitag",
    "Samstag",
];

/// Beschriftung eines Sendetags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DayLabel {
    pub weekday: &'static str,
    pub long: &'static str,
    /// `'DD.MM.'`
    pub date: String,
}

/// `{ weekday: 'Mo', long: 'Montag', date: '11.11.' }` zum Sendetag `d`.
pub fn format_day(d: i64) -> DayLabel {
    let wd = (d + 4).rem_euclid(7) as usize;
    let (_, m, day) = civil_from_days(d);
    DayLabel {
        weekday: WD_SHORT[wd],
        long: WD_LONG[wd],
        date: format!("{}.{}.", two(day), two(m)),
    }
}

/// Wochenindex (Woche beginnt Montag).
pub fn week_of(d: i64) -> i64 {
    (d + 3).div_euclid(7)
}

/// Erster Sendetag (Montag) der Woche `w`.
pub fn week_start(w: i64) -> i64 {
    7 * w - 3
}

/// Erster Sendetag, an dem ein Titel mit `added_at = t` bereits im Pool ist.
pub fn admit_day(t: i64) -> i64 {
    broadcast_day_of(t) + 1
}

fn all_digits(s: &[u8]) -> bool {
    s.iter().all(u8::is_ascii_digit)
}

fn num(s: &[u8]) -> i64 {
    s.iter().fold(0i64, |a, &b| a * 10 + i64::from(b - b'0'))
}

fn digits_at(b: &[u8], i: usize, n: usize) -> bool {
    b.len() >= i + n && all_digits(&b[i..i + n])
}

/// `Date.parse` für die ISO-8601-Formen von ECMAScript (V8-kompatibel: Nachkommastellen über
/// drei Stellen werden abgeschnitten). Legacy-Formen liefern `None`.
pub fn date_parse_iso(x: &str) -> Option<f64> {
    let b = x.as_bytes();
    let mut i;
    let year: i64 = if b.first().is_some_and(|c| *c == b'+' || *c == b'-') {
        let neg = b[0] == b'-';
        if !digits_at(b, 1, 6) {
            return None;
        }
        let y = num(&b[1..7]);
        if neg && y == 0 {
            return None;
        }
        i = 7;
        if neg { -y } else { y }
    } else {
        if !digits_at(b, 0, 4) {
            return None;
        }
        i = 4;
        num(&b[0..4])
    };
    let mut month = 1;
    let mut day = 1;
    let (mut h, mut mi, mut s, mut ms) = (0i64, 0i64, 0i64, 0i64);
    let mut offset: i64 = 0;
    if i < b.len() && b[i] == b'-' {
        if !digits_at(b, i + 1, 2) {
            return None;
        }
        month = num(&b[i + 1..i + 3]);
        i += 3;
        if i < b.len() && b[i] == b'-' {
            if !digits_at(b, i + 1, 2) {
                return None;
            }
            day = num(&b[i + 1..i + 3]);
            i += 3;
        }
    }
    if !(1..=12).contains(&month) {
        return None;
    }
    let dim = {
        let a = days_from_civil(year, month, 1);
        let n = if month == 12 {
            days_from_civil(year + 1, 1, 1)
        } else {
            days_from_civil(year, month + 1, 1)
        };
        n - a
    };
    if day < 1 || day > dim {
        return None;
    }
    if i < b.len() && b[i] == b'T' {
        i += 1;
        if !digits_at(b, i, 2) || b.get(i + 2) != Some(&b':') || !digits_at(b, i + 3, 2) {
            return None;
        }
        h = num(&b[i..i + 2]);
        mi = num(&b[i + 3..i + 5]);
        i += 5;
        if i < b.len() && b[i] == b':' {
            if !digits_at(b, i + 1, 2) {
                return None;
            }
            s = num(&b[i + 1..i + 3]);
            i += 3;
            if i < b.len() && b[i] == b'.' {
                let st = i + 1;
                let mut e = st;
                while e < b.len() && b[e].is_ascii_digit() {
                    e += 1;
                }
                if e == st {
                    return None;
                }
                let frac = &b[st..e];
                let mut v = 0i64;
                for k in 0..3 {
                    v = v * 10 + frac.get(k).map_or(0, |c| i64::from(c - b'0'));
                }
                ms = v;
                i = e;
            }
        }
        if h > 24 || mi > 59 || s > 59 || (h == 24 && (mi != 0 || s != 0 || ms != 0)) {
            return None;
        }
        if i < b.len() {
            if b[i] == b'Z' {
                i += 1;
            } else if b[i] == b'+' || b[i] == b'-' {
                if !digits_at(b, i + 1, 2) || b.get(i + 3) != Some(&b':') || !digits_at(b, i + 4, 2)
                {
                    return None;
                }
                let oh = num(&b[i + 1..i + 3]);
                let om = num(&b[i + 4..i + 6]);
                if oh > 23 || om > 59 {
                    return None;
                }
                offset = (oh * 60 + om) * MIN * if b[i] == b'-' { -1 } else { 1 };
                i += 6;
            }
        }
    }
    if i != b.len() {
        return None;
    }
    let t =
        days_from_civil(year, month, day) * DAY_MS + h * HOUR + mi * MIN + s * 1000 + ms - offset;
    if t.abs() > 8_640_000_000_000_000 {
        None
    } else {
        Some(t as f64)
    }
}

/// `parseDate(s)`: Zahl (endlich) bleibt, Zeichenkette ISO; ohne Zeitzone gilt UTC; sonst `None`.
pub fn parse_date(v: &JsVal) -> Option<f64> {
    match v {
        JsVal::Num(n) => n.is_finite().then_some(*n),
        JsVal::Str(s) if !s.is_empty() => {
            let t = js_trim(s);
            let b = t.as_bytes();
            let mut x = t.to_string();
            if b.len() == 10
                && all_digits(&b[0..4])
                && b[4] == b'-'
                && all_digits(&b[5..7])
                && b[7] == b'-'
                && all_digits(&b[8..10])
            {
                x.push_str("T00:00:00Z");
            } else if let Some(p) = t.rfind('T') {
                let tail = &t[p + 1..];
                if !tail.is_empty()
                    && tail
                        .bytes()
                        .all(|c| c.is_ascii_digit() || c == b':' || c == b'.')
                {
                    x.push('Z');
                }
            }
            date_parse_iso(&x)
        }
        _ => None,
    }
}

/// Ticks (100 ns) zu ms (`Math.round(Number(t) / 1e4)`, nicht endlich = 0).
pub fn ticks_to_ms(v: &JsVal) -> f64 {
    let n = round_js(js_number(v) / 1e4);
    if n.is_finite() { n } else { 0.0 }
}
