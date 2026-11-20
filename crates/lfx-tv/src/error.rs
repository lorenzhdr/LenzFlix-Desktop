//! Fehler des TV-Moduls. Meldungen enthalten nie Schlüssel oder Adressen.

use thiserror::Error;

/// Fehler beim Laden des Katalogs.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum TvError {
    /// Die Quelle (Server) meldete einen Fehler; Text ohne Schlüssel.
    #[error("Katalog konnte nicht geladen werden: {0}")]
    Source(String),
}

/// Fehlertext ohne Schlüssel (`api_key`/`ApiKey`), höchstens 200 Zeichen.
pub fn fatal_text(msg: &str) -> String {
    let lower = msg.to_ascii_lowercase();
    let mut out = String::new();
    let mut i = 0;
    let bytes = msg.as_bytes();
    while i < bytes.len() {
        let rest = &lower[i..];
        let key_len = if rest.starts_with("api_key=") {
            Some(8)
        } else if rest.starts_with("apikey=") {
            Some(7)
        } else {
            None
        };
        if let Some(k) = key_len {
            out.push_str(&msg[i..i + k]);
            out.push('…');
            i += k;
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'&' {
                i += 1;
            }
        } else {
            let ch = msg[i..].chars().next().unwrap_or(' ');
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out.chars().take(200).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schluessel_werden_geschwaerzt() {
        let t = fatal_text("GET /x?api_key=abc123&b=1 und ApiKey=geheim fertig");
        assert!(!t.contains("abc123") && !t.contains("geheim"));
        assert!(t.contains("api_key=…&b=1") && t.contains("ApiKey=… fertig"));
    }

    #[test]
    fn kuerzt_auf_200_zeichen() {
        assert_eq!(fatal_text(&"ä".repeat(500)).chars().count(), 200);
    }
}
