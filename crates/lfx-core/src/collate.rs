//! Deutsche Sortierung wie `localeCompare(b, 'de')` (ICU, Standardeinstellung) für Genre- und Titelnamen.
//!
//! Handgebauter Schlüssel in drei Stufen, kein ICU: (1) Grundbuchstaben ohne Akzente und Groß-/Kleinschreibung,
//! Reihenfolge Leerraum/Satzzeichen < Ziffern < Buchstaben < Rest; (2) Akzente (ä > a, ß > ss);
//! (3) Kleinbuchstabe vor Großbuchstabe. Grenzen: nur lateinische Schrift ist fein abgestimmt, seltene
//! Satz-/Sonderzeichen sind nach einer festen Liste geordnet; Abweichungen zu ICU sind dort möglich.
//! Die Golden-Vektoren (`tests/golden/dayseed.json`) belegen die Übereinstimmung für übliche Genre-Namen.

use std::cmp::Ordering;
use unicode_normalization::UnicodeNormalization;

const PUNCT_ORDER: &str = r#" _-,;:!?.'"()[]{}@*/\&#%`^+<=>|~$"#;

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    primary: Vec<(u8, u32)>,
    accents: Vec<u32>,
    case: Vec<u8>,
}

fn is_mark(c: char) -> bool {
    ('\u{0300}'..='\u{036F}').contains(&c)
}

fn key(s: &str) -> Key {
    let mut primary = Vec::new();
    let mut accents = Vec::new();
    let mut case = Vec::new();
    for c in s.nfd() {
        if is_mark(c) {
            if let Some(last) = accents.last_mut() {
                *last = *last * 8 + (c as u32 - 0x2FF);
            }
            continue;
        }
        let upper = c.is_uppercase();
        for l in c.to_lowercase() {
            let (base, acc): (&str, u32) = match l {
                'ß' => ("ss", 1),
                'æ' => ("ae", 1),
                'œ' => ("oe", 1),
                'ø' => ("o", 1),
                'đ' => ("d", 1),
                'ł' => ("l", 1),
                _ => ("", 0),
            };
            if base.is_empty() {
                let class = if let Some(p) = PUNCT_ORDER.find(l) {
                    (0u8, p as u32)
                } else if l.is_ascii_digit() {
                    (1, l as u32)
                } else if l.is_alphabetic() && (l as u32) < 0x250 {
                    (2, l as u32)
                } else if l.is_alphabetic() {
                    (3, l as u32)
                } else {
                    (0, 100 + l as u32)
                };
                primary.push(class);
                accents.push(0);
                case.push(u8::from(upper));
            } else {
                for b in base.chars() {
                    primary.push((2, b as u32));
                    accents.push(acc);
                    case.push(u8::from(upper));
                }
            }
        }
    }
    Key {
        primary,
        accents,
        case,
    }
}

/// Vergleich wie `a.localeCompare(b, 'de')`.
pub fn de_compare(a: &str, b: &str) -> Ordering {
    key(a).cmp(&key(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted(v: &[&str]) -> Vec<String> {
        let mut o: Vec<String> = v.iter().map(|s| (*s).to_owned()).collect();
        o.sort_by(|a, b| de_compare(a, b));
        o
    }

    #[test]
    fn umlauts_sort_with_base_letter() {
        assert_eq!(
            sorted(&["Zug", "Ärger", "Abend", "Äpfel", "Apfel"]),
            vec!["Abend", "Apfel", "Äpfel", "Ärger", "Zug"]
        );
    }

    #[test]
    fn case_and_sharp_s() {
        assert_eq!(sorted(&["b", "B", "a", "A"]), vec!["a", "A", "b", "B"]);
        assert_eq!(de_compare("ss", "ß"), Ordering::Less);
        assert_eq!(de_compare("Straße", "Strasse"), Ordering::Greater);
    }

    #[test]
    fn punctuation_before_letters() {
        assert_eq!(
            sorted(&["Action", "Action & Adventure", "Science-Fiction", "Sci-Fi"]),
            vec!["Action", "Action & Adventure", "Sci-Fi", "Science-Fiction"]
        );
    }
}
