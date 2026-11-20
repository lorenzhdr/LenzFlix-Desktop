//! Protokoll in eine Datei unter dem Datenordner (`logs/lenzflix.log`). Jede Zeile läuft durch
//! [`scrub`]: Adressen (Server-Host, Stream-URLs), Tokens und Schlüssel werden geschwärzt.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Ersetzt Adressen und Geheimnisse in einer Zeile.
///
/// - `http://…` und `https://…` (bis Leerzeichen, Anführungszeichen, `)`, `,` oder `>`) werden zu `<adresse>`.
/// - `Token="…"`, `Token=…`, `ApiKey=…`, `api_key=…`, `X-Emby-Token`, `Authorization: …` und `Bearer …` verlieren den Wert.
/// - `Password`/`Pw` als Schlüssel (`password=…`) ebenso.
pub fn scrub(line: &str) -> String {
    let mut out = replace_urls(line);
    for key in [
        "Token", "ApiKey", "api_key", "apikey", "Password", "password", "Secret", "secret",
        "DeviceId",
    ] {
        out = mask_value(&out, key);
    }
    mask_after(&out, "Bearer ")
}

fn is_url_end(c: char) -> bool {
    c.is_whitespace() || matches!(c, '"' | '\'' | ')' | ',' | '>' | ']' | '}')
}

fn replace_urls(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    loop {
        let http = rest.find("http://");
        let https = rest.find("https://");
        let at = match (http, https) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        let Some(at) = at else {
            out.push_str(rest);
            return out;
        };
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        let end = tail.find(is_url_end).unwrap_or(tail.len());
        out.push_str("<adresse>");
        rest = &tail[end..];
    }
}

/// `Key="wert"`, `Key=wert`, `Key: wert` -> Wert durch `***` ersetzen (ganze Wörter am Schlüsselanfang).
fn mask_value(s: &str, key: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find(key) {
        let before_ok = rest[..i]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric());
        let after = &rest[i + key.len()..];
        let sep = after.chars().next();
        if before_ok && matches!(sep, Some('=') | Some(':')) {
            out.push_str(&rest[..i + key.len() + 1]);
            let val = after[1..].trim_start_matches(' ');
            let skipped = after.len() - 1 - val.len();
            out.push_str(&" ".repeat(skipped));
            let (quoted, body) = match val.strip_prefix('"') {
                Some(b) => (true, b),
                None => (false, val),
            };
            let len = if quoted {
                body.find('"').unwrap_or(body.len())
            } else {
                body.find(|c: char| c.is_whitespace() || c == '&' || c == ',' || c == '"')
                    .unwrap_or(body.len())
            };
            if quoted {
                out.push('"');
            }
            out.push_str("***");
            rest = &body[len..];
        } else {
            out.push_str(&rest[..i + key.len()]);
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

fn mask_after(s: &str, marker: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find(marker) {
        out.push_str(&rest[..i + marker.len()]);
        let tail = &rest[i + marker.len()..];
        let end = tail.find(char::is_whitespace).unwrap_or(tail.len());
        out.push_str("***");
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

/// Schreibt geschwärzte Zeilen in eine Datei.
#[derive(Clone)]
pub struct ScrubWriter {
    file: Arc<Mutex<File>>,
}

impl ScrubWriter {
    /// Öffnet (oder legt an) die Protokolldatei im Anhängemodus.
    pub fn open(path: &Path) -> std::io::Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Self {
            file: Arc::new(Mutex::new(file)),
        })
    }
}

impl Write for ScrubWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let text = String::from_utf8_lossy(buf);
        let clean = scrub(&text);
        let mut f = self.file.lock().unwrap_or_else(|e| e.into_inner());
        f.write_all(clean.as_bytes())?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.file.lock().unwrap_or_else(|e| e.into_inner()).flush()
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for ScrubWriter {
    type Writer = ScrubWriter;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// Richtet `tracing` ein (Datei, keine Farben). Fehler beim Öffnen werden still ignoriert (kein Protokoll).
pub fn init(log_file: &Path) {
    let Ok(w) = ScrubWriter::open(log_file) else {
        return;
    };
    let _ = tracing_subscriber::fmt()
        .with_writer(w)
        .with_ansi(false)
        .with_max_level(tracing::Level::INFO)
        .try_init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_are_hidden() {
        let s = scrub("GET http://192.168.0.5:8096/Videos/abc/stream?static=true fertig");
        assert_eq!(s, "GET <adresse> fertig");
        let s = scrub("Fehler (https://mein.server.example/x), danach");
        assert_eq!(s, "Fehler (<adresse>), danach");
    }

    #[test]
    fn tokens_and_keys_are_hidden() {
        let s = scrub(
            r#"Authorization: MediaBrowser Client="LENZFLIX", Token="geheim123", Version="0.1""#,
        );
        assert!(!s.contains("geheim123"), "{s}");
        assert!(s.contains(r#"Token="***""#), "{s}");
        let s = scrub("pfad?ApiKey=abc123&x=1");
        assert_eq!(s, "pfad?ApiKey=***&x=1");
        let s = scrub("Bearer abc.def.ghi rest");
        assert_eq!(s, "Bearer *** rest");
        let s = scrub("password=hunter2 ok");
        assert_eq!(s, "password=*** ok");
    }

    #[test]
    fn words_containing_a_key_are_kept() {
        assert_eq!(scrub("Tokenizer=3"), "Tokenizer=3");
        assert_eq!(scrub("kein Geheimnis hier"), "kein Geheimnis hier");
    }

    #[test]
    fn mpv_line_with_stream_url() {
        let s = scrub(
            "[ffmpeg] Opening 'https://x.example:8920/Videos/1/stream?static=true&ApiKey=zzz' for reading",
        );
        assert!(!s.contains("x.example") && !s.contains("zzz"), "{s}");
    }

    #[test]
    fn writer_scrubs_into_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logs").join("t.log");
        let mut w = ScrubWriter::open(&path).unwrap();
        w.write_all(b"verbinde http://127.0.0.1:1234/x Token=\"abc\"\n")
            .unwrap();
        w.flush().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            !text.contains("127.0.0.1") && !text.contains("abc"),
            "{text}"
        );
    }
}
