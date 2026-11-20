//! Kleine JS-Entsprechungen (Math.round, Date.parse, encodeURIComponent).

use lfx_tv_schedule::time::date_parse_iso;

/// Millisekunden je Tag.
pub const DAY_MS: f64 = 86_400_000.0;

/// `Math.round` (Halbe nach oben, auch bei negativen Zahlen).
pub(crate) fn js_round(x: f64) -> f64 {
    (x + 0.5).floor()
}

/// `Date.parse` für ISO-Zeichenketten (V8-kompatibel); ungültig = `None` (JS: `NaN`).
pub fn parse_ms(s: &str) -> Option<f64> {
    date_parse_iso(s).filter(|t| t.is_finite())
}

/// `Date.parse(x)` für optionale Eingaben.
pub(crate) fn parse_opt(s: Option<&str>) -> Option<f64> {
    s.and_then(parse_ms)
}

/// `encodeURIComponent`.
pub fn encode_uri_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `decodeURIComponent` / `URLSearchParams` (mit `+` als Leerzeichen); kaputte Folgen bleiben stehen.
pub fn decode_query_component(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => match (hex(b[i + 1]), hex(b[i + 2])) {
                (Some(h), Some(l)) => {
                    out.push(h * 16 + l);
                    i += 2;
                }
                _ => out.push(b'%'),
            },
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// Zeichenkette nur, wenn gesetzt und nicht leer (JS-Wahrheitswert eines Strings).
pub(crate) fn non_empty(s: &Option<String>) -> Option<&str> {
    s.as_deref().filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounds_like_js() {
        assert_eq!(js_round(2.5), 3.0);
        assert_eq!(js_round(-2.5), -2.0);
        assert_eq!(js_round(0.4), 0.0);
    }

    #[test]
    fn uri_roundtrip() {
        let s = "Komödie & Co/Ä";
        assert_eq!(decode_query_component(&encode_uri_component(s)), s);
        assert_eq!(encode_uri_component("a b"), "a%20b");
        assert_eq!(decode_query_component("a+b%20c%zz"), "a b c%zz");
    }

    #[test]
    fn parses_jellyfin_dates() {
        assert_eq!(
            parse_ms("2026-03-01T10:00:00.0000000Z"),
            Some(1_772_359_200_000.0)
        );
        assert_eq!(parse_ms("kaputt"), None);
    }
}
