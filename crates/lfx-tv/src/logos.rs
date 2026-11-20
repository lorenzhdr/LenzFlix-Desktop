//! Senderfarben und Kurznamen (Port von `ui/logos.js`): Farbe je Platz eindeutig, genutzt von
//! Senderliste und Vorschaubild.

use lfx_tv_schedule::Channel;
use lfx_tv_schedule::jsval::js_trim;

/// 18 verschiedene Senderfarben (Platz 1 bis 18).
pub const LOGO_COLORS: [&str; 18] = [
    "#e53935", "#fb8c00", "#fdd835", "#7cb342", "#00897b", "#00acc1", "#1e88e5", "#3949ab",
    "#8e24aa", "#d81b60", "#6d4c41", "#546e7a", "#f4511e", "#c0ca33", "#43a047", "#039be5",
    "#5e35b1", "#ec407a",
];

const MAX_NAME: usize = 9;

/// Farbe des Platzes `no` (ungültige Nummern werden auf 1 gesetzt, Umlauf nach 18).
pub fn channel_color(no: i64) -> &'static str {
    let n = no.max(1);
    LOGO_COLORS[((n - 1) as usize) % LOGO_COLORS.len()]
}

/// Dunkle (`#1a1a1a`) oder weiße Schrift: die Farbe mit dem höheren WCAG-Kontrast.
pub fn channel_ink(no: i64) -> &'static str {
    let c = &channel_color(no)[1..];
    let ch = |i: usize| f64::from(u8::from_str_radix(&c[i..i + 2], 16).unwrap_or(0));
    let lin = |v: f64| {
        let v = v / 255.0;
        if v <= 0.03928 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    let l = 0.2126 * lin(ch(0)) + 0.7152 * lin(ch(2)) + 0.0722 * lin(ch(4)) + 0.05;
    // 0,0603 = Luminanz von #1a1a1a + 0,05
    if l / 0.0603 > 1.05 / l {
        "#1a1a1a"
    } else {
        "#ffffff"
    }
}

fn fixed_name(key: &str) -> Option<&'static str> {
    Some(match key {
        "kino" => "KINO",
        "klassik" => "KLASSIK",
        "serie" => "SERIE",
        "marathon" => "MARATHON",
        "premiere" => "PREMIERE",
        "thema" => "THEMA",
        _ => return None,
    })
}

/// Kurzname: `KINO`, `KLASSIK`, Genre gekürzt auf 9 Zeichen (Zeichen, nicht Bytes).
pub fn short_name(channel: &Channel) -> String {
    if let Some(f) = fixed_name(&channel.key) {
        return f.to_string();
    }
    let base: String = match channel.genre_label.as_deref().filter(|s| !s.is_empty()) {
        Some(g) => g.to_string(),
        None => {
            let stripped = strip_live(&channel.name);
            if stripped.is_empty() {
                format!("P{}", channel.no)
            } else {
                stripped.to_string()
            }
        }
    };
    let up = js_trim(&base).to_uppercase();
    if up.chars().count() > MAX_NAME {
        js_trim(&up.chars().take(MAX_NAME).collect::<String>()).to_string()
    } else {
        up
    }
}

fn strip_live(s: &str) -> &str {
    if s.len() >= 4 && s.is_char_boundary(4) && s[..4].eq_ignore_ascii_case("LIVE") {
        let rest = &s[4..];
        let t = rest.trim_start_matches(lfx_tv_schedule::jsval::is_js_ws);
        if t.len() < rest.len() {
            return t;
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn ch(no: i64, key: &str, name: &str, genre: Option<&str>) -> Channel {
        Channel {
            no,
            key: key.into(),
            name: name.into(),
            kind: "genre",
            genre_label: genre.map(str::to_string),
        }
    }

    #[test]
    fn achtzehn_eindeutige_farben() {
        let set: HashSet<_> = (1..=18).map(channel_color).collect();
        assert_eq!(set.len(), 18);
        assert_eq!(channel_color(19), channel_color(1));
        assert_eq!(channel_color(0), channel_color(1));
    }

    #[test]
    fn schriftfarbe_nach_kontrast() {
        assert_eq!(channel_ink(3), "#1a1a1a"); // gelb
        assert_eq!(channel_ink(8), "#ffffff"); // dunkelblau
    }

    #[test]
    fn kurznamen() {
        assert_eq!(short_name(&ch(1, "kino", "LIVE Kino", None)), "KINO");
        assert_eq!(
            short_name(&ch(5, "g5", "LIVE Action", Some("Action"))),
            "ACTION"
        );
        assert_eq!(
            short_name(&ch(6, "g6", "x", Some("Dokumentationen"))),
            "DOKUMENTA"
        );
        assert_eq!(short_name(&ch(7, "g7", "LIVE Krimi", None)), "KRIMI");
        assert_eq!(short_name(&ch(9, "g9", "", None)), "P9");
        // Umlaute zählen als ein Zeichen
        assert_eq!(
            short_name(&ch(8, "g8", "", Some("Übernatürliches"))),
            "ÜBERNATÜR"
        );
    }
}
