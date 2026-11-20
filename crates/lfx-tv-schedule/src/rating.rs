//! Altersfreigabe: 0, 6, 12, 16, 18 oder unbekannt (`None`, gilt als erlaubt).

use crate::jsval::{JsVal, js_string, js_trim};

/// Freigabe in Jahren (0, 6, 12, 16, 18); `None` = unbekannt.
pub type Age = Option<u8>;

/// `ratingAge(str)`.
pub fn rating_age(v: &JsVal) -> Age {
    if v.is_nullish() {
        return None;
    }
    let s = js_trim(&js_string(v)).to_uppercase();
    match s.as_str() {
        "G" | "U" | "TV-Y" | "TV-G" => return Some(0),
        "PG" | "TV-PG" | "TV-Y7" | "TV-Y7-FV" => return Some(6),
        "R" | "TV-MA" => return Some(16),
        "NC-17" | "X" | "XXX" | "AO" | "R18" => return Some(18),
        "NR" | "UNRATED" | "" => return None,
        _ => {}
    }
    // Regex /(\d{1,2})\+?A?$/ ohne u-Flag: \d = nur ASCII-Ziffern
    let mut t = s.as_str();
    if let Some(r) = t.strip_suffix('A') {
        t = r;
    }
    if let Some(r) = t.strip_suffix('+') {
        t = r;
    }
    let digits: Vec<u8> = t
        .bytes()
        .rev()
        .take_while(u8::is_ascii_digit)
        .take(2)
        .collect();
    if digits.is_empty() {
        return None;
    }
    let n = if digits.len() == 2 {
        (digits[1] - b'0') * 10 + (digits[0] - b'0')
    } else {
        digits[0] - b'0'
    };
    Some(match n {
        18.. => 18,
        15.. => 16,
        12.. => 12,
        6.. => 6,
        _ => 0,
    })
}

/// Strengere von zwei Freigaben (`None` zählt als schwächer).
pub fn max_age(a: Age, b: Age) -> Age {
    match (a, b) {
        (None, b) => b,
        (a, None) => a,
        (Some(x), Some(y)) => Some(x.max(y)),
    }
}
