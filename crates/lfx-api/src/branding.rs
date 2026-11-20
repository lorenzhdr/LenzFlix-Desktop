//! Auswertung des Branding-CSS (`GET /Branding/Css`): liest die Admin-Einstellungen des Web-Themes aus
//! den Custom Properties `--lfx-charts-url`, `--lfx-stats-url` und `--lfx-tv-channels`.
//!
//! Regeln wie im Web-Theme: Anführungszeichen und Leerraum werden entfernt, die Charts-URL muss `https`
//! sein (Ausnahme nur zum Testen: `http` auf Loopback), die Statistik-URL `https` oder ein Pfad mit genau
//! einem führenden Schrägstrich, Sender nur auf den Plätzen 5 bis 16.

use std::collections::BTreeMap;

/// Aus dem Branding-CSS gelesene Einstellungen.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Branding {
    /// Basis-URL des Charts-Workers (ohne abschließenden Schrägstrich).
    pub charts_url: Option<String>,
    /// URL der Statistik-Seite.
    pub stats_url: Option<String>,
    /// Senderbelegung Platz → Genre.
    pub tv_channels: BTreeMap<u8, String>,
}

/// Entfernt `/* … */`-Kommentare.
fn strip_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        match rest[start + 2..].find("*/") {
            Some(end) => rest = &rest[start + 2 + end + 2..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
}

/// Rohwert der zuletzt gesetzten Custom Property `name` (ohne `!important`), `None` wenn nicht vorhanden.
fn custom_property(css: &str, name: &str) -> Option<String> {
    let mut found = None;
    let mut from = 0;
    while let Some(pos) = css[from..].find(name) {
        let at = from + pos;
        from = at + name.len();
        let before_ok = css[..at].chars().next_back().is_none_or(|c| !is_ident(c));
        let after = css[from..].trim_start();
        if !before_ok || !after.starts_with(':') {
            continue;
        }
        let value_src = &after[1..];
        let mut value = String::new();
        let mut quote: Option<char> = None;
        for c in value_src.chars() {
            match quote {
                Some(q) => {
                    value.push(c);
                    if c == q {
                        quote = None;
                    }
                }
                None => match c {
                    ';' | '}' => break,
                    '"' | '\'' => {
                        quote = Some(c);
                        value.push(c);
                    }
                    _ => value.push(c),
                },
            }
        }
        let lower = value.to_ascii_lowercase();
        let value = match lower.rfind("!important") {
            Some(i) => value[..i].to_owned(),
            None => value,
        };
        found = Some(value);
    }
    found
}

fn strip_quotes(raw: &str) -> &str {
    raw.trim_matches(|c: char| c == '"' || c == '\'' || c.is_whitespace())
}

fn host_is_loopback(url: &str) -> bool {
    let rest = url.strip_prefix("http://").unwrap_or("");
    let host = rest.split(['/', ':']).next().unwrap_or("");
    host == "127.0.0.1" || host == "localhost" || host == "[::1]"
}

/// Charts-URL bereinigen: nur `https` (Loopback-`http` zum Testen), ohne Leerraum/Anführungszeichen.
pub fn clean_charts_url(raw: &str) -> Option<String> {
    let s = strip_quotes(raw).trim_end_matches('/');
    if s.is_empty() || s.contains(|c: char| c.is_whitespace() || c == '"' || c == '\'') {
        return None;
    }
    let lower = s.to_ascii_lowercase();
    let ok = (lower.starts_with("https://") && s.len() > "https://".len())
        || (lower.starts_with("http://") && host_is_loopback(&lower));
    ok.then(|| s.to_owned())
}

/// Statistik-URL bereinigen: `https://…` oder Pfad mit genau einem führenden `/`.
pub fn clean_stats_url(raw: &str) -> Option<String> {
    let s = strip_quotes(raw);
    if s.is_empty() || s.contains(|c: char| c.is_whitespace() || "\"'\\<>".contains(c)) {
        return None;
    }
    let https = s.to_ascii_lowercase().starts_with("https://") && s.len() > "https://".len();
    let path = s.starts_with('/') && !s.starts_with("//") && !s.starts_with("/\\");
    (https || path).then(|| s.to_owned())
}

/// `"5=Action;6=Komödie"` → Platz → Genre (nur 5 bis 16, ungültige Einträge entfallen).
pub fn parse_channels(raw: &str) -> BTreeMap<u8, String> {
    let mut out = BTreeMap::new();
    for part in strip_quotes(raw).split(';') {
        let Some((no, label)) = part.split_once('=') else {
            continue;
        };
        let Ok(no) = no.trim().parse::<u8>() else {
            continue;
        };
        let label = strip_quotes(label);
        if (5..=16).contains(&no) && !label.is_empty() {
            out.insert(no, label.to_owned());
        }
    }
    out
}

/// Wertet ein Branding-CSS aus. `@import`-Regeln werden nie nachgeladen.
pub fn parse_branding(css: &str) -> Branding {
    let css = strip_comments(css);
    let channels = custom_property(&css, "--lfx-tv-channels")
        .map(|v| parse_channels(&v))
        .filter(|m| !m.is_empty())
        .or_else(|| {
            custom_property(&css, "--lfx-live-channels")
                .map(|v| parse_channels(&v))
                .filter(|m| !m.is_empty())
        })
        .unwrap_or_default();
    Branding {
        charts_url: custom_property(&css, "--lfx-charts-url").and_then(|v| clean_charts_url(&v)),
        stats_url: custom_property(&css, "--lfx-stats-url").and_then(|v| clean_stats_url(&v)),
        tv_channels: channels,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_three_properties() {
        let css = r#"
            @import url("https://theme.example.invalid/lenzflix.min.css");
            /* Kommentar --lfx-charts-url: "https://boese.invalid"; */
            :root {
              --lfx-charts-url: "https://charts.example.invalid/";
              --lfx-stats-url:  '/lenzflix-stats' ;
              --lfx-tv-channels: "5=Action;6=Komödie;4=Zu klein;17=Zu groß;7=;x=Y;16='Drama'";
            }"#;
        let b = parse_branding(css);
        assert_eq!(
            b.charts_url.as_deref(),
            Some("https://charts.example.invalid")
        );
        assert_eq!(b.stats_url.as_deref(), Some("/lenzflix-stats"));
        assert_eq!(
            b.tv_channels.into_iter().collect::<Vec<_>>(),
            vec![
                (5, "Action".to_owned()),
                (6, "Komödie".to_owned()),
                (16, "Drama".to_owned())
            ]
        );
    }

    #[test]
    fn last_definition_wins_and_important_is_dropped() {
        let css = ":root{--lfx-charts-url:\"https://a.invalid\"} html{--lfx-charts-url:\"https://b.invalid\" !important}";
        assert_eq!(
            parse_branding(css).charts_url.as_deref(),
            Some("https://b.invalid")
        );
    }

    #[test]
    fn rejects_unsafe_urls() {
        assert_eq!(clean_charts_url("http://charts.example.invalid"), None);
        assert_eq!(clean_charts_url("https://a b.invalid"), None);
        assert_eq!(clean_charts_url("ftp://x"), None);
        assert_eq!(
            clean_charts_url("http://127.0.0.1:8096/cw").as_deref(),
            Some("http://127.0.0.1:8096/cw")
        );
        assert_eq!(clean_stats_url("//evil.invalid"), None);
        assert_eq!(clean_stats_url("http://x.invalid"), None);
    }

    #[test]
    fn ignores_look_alike_names_and_falls_back_to_old_channel_name() {
        let css =
            ":root{--lfx-charts-url-alt:\"https://x.invalid\";--lfx-live-channels:\"8=Krimi\"}";
        let b = parse_branding(css);
        assert!(b.charts_url.is_none());
        assert_eq!(b.tv_channels.get(&8).map(String::as_str), Some("Krimi"));
    }

    #[test]
    fn empty_css_gives_defaults() {
        assert_eq!(parse_branding(""), Branding::default());
    }
}
