//! Schwärzen von URLs für Logs: Host → `<server>`, Geheimnis-Parameter → `***`.

use reqwest::Url;

/// Query-Schlüssel (kleingeschrieben), deren Werte nie in Logs landen dürfen.
const SENSITIVE_KEYS: &[&str] = &[
    "apikey",
    "api_key",
    "token",
    "accesstoken",
    "access_token",
    "x-emby-token",
    "x-mediabrowser-token",
    "secret",
    "pw",
    "password",
];

/// Liefert eine log-taugliche Form: `<server>` statt Schema/Host/Port, Pfad unverändert,
/// Werte sensibler Query-Parameter durch `***` ersetzt.
pub fn redact_url(url: &str) -> String {
    let Ok(parsed) = Url::parse(url) else {
        return "<ungueltige-url>".to_owned();
    };
    let mut out = String::from("<server>");
    out.push_str(parsed.path());
    let mut first = true;
    for (key, value) in parsed.query_pairs() {
        out.push(if first { '?' } else { '&' });
        first = false;
        out.push_str(&key);
        out.push('=');
        if SENSITIVE_KEYS.contains(&key.to_ascii_lowercase().as_str()) {
            out.push_str("***");
        } else {
            out.push_str(&value);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_host_and_secrets() {
        let s = redact_url(
            "https://media.example.invalid:8920/Videos/1/stream?static=true&ApiKey=abc123&x=1",
        );
        assert_eq!(s, "<server>/Videos/1/stream?static=true&ApiKey=***&x=1");
        assert!(!s.contains("example"));
        assert!(!s.contains("abc123"));
    }

    #[test]
    fn invalid_url_is_masked() {
        assert_eq!(redact_url("kein url"), "<ungueltige-url>");
    }
}
