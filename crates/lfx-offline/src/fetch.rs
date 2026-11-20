//! Übertragung einer Datei in eine `.part`-Datei: streamend, fortsetzbar per `Range`, mit Größen- und
//! Prüfsummenermittlung. Die Fortsetzung vertraut nur der Länge der `.part`-Datei, nie einem Zähler.
//!
//! Strenge Regeln: `206` nur mit passendem Anfang im `Content-Range`; `200` auf eine `Range`-Anfrage heißt
//! Neustart bei null; `416` bei voller Länge heißt fertig. Der Token geht nur im `Authorization`-Header mit.

use crate::error::{OfflineError, Result};
use reqwest::StatusCode;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::watch;

/// Steuerung einer laufenden Übertragung.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Control {
    Run,
    Pause,
    Cancel,
}

/// Ergebnis einer Übertragung.
#[derive(Debug)]
pub(crate) enum Outcome {
    /// Fertig: Gesamtlänge und SHA-256 (hex) der `.part`-Datei.
    Done { size: u64, sha256: String },
    /// Auf Wunsch angehalten (Datei bleibt).
    Paused,
    /// Auf Wunsch abgebrochen.
    Canceled,
}

/// Parameter einer Übertragung.
pub(crate) struct Transfer<'a> {
    pub http: &'a reqwest::Client,
    pub url: &'a str,
    pub auth: &'a str,
    pub part: &'a Path,
    /// Darf eine vorhandene `.part`-Datei mit `Range` fortgesetzt werden?
    pub allow_resume: bool,
    /// Zeitlimit ohne neue Daten.
    pub idle: Duration,
}

fn net_err(e: reqwest::Error) -> OfflineError {
    OfflineError::Network(e.without_url().to_string())
}

fn status_err(status: StatusCode) -> OfflineError {
    match status {
        StatusCode::UNAUTHORIZED => OfflineError::Api(lfx_api::ApiError::Unauthorized),
        StatusCode::FORBIDDEN => OfflineError::NotAllowed,
        StatusCode::NOT_FOUND => OfflineError::Api(lfx_api::ApiError::NotFound),
        s => OfflineError::Http(s.as_u16()),
    }
}

/// Liest `bytes a-b/total` (total kann `*` sein).
fn parse_content_range(value: &str) -> Option<(u64, u64, Option<u64>)> {
    let spec = value.trim().strip_prefix("bytes")?.trim();
    let (range, total) = spec.split_once('/')?;
    let (a, b) = range.trim().split_once('-')?;
    let total = match total.trim() {
        "*" => None,
        t => Some(t.parse().ok()?),
    };
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?, total))
}

/// `bytes */N` einer 416-Antwort.
fn parse_unsatisfied(value: &str) -> Option<u64> {
    value
        .trim()
        .strip_prefix("bytes")?
        .trim()
        .strip_prefix("*/")?
        .trim()
        .parse()
        .ok()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// SHA-256 (hex) einer Datei.
pub(crate) async fn sha256_file(path: &Path) -> Result<String> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

async fn part_len(path: &Path) -> u64 {
    tokio::fs::metadata(path)
        .await
        .map(|m| m.len())
        .unwrap_or(0)
}

/// Wartet auf eine Steueränderung (Pause/Abbruch) oder bis `fut` fertig ist.
async fn guarded<T>(
    ctl: &mut watch::Receiver<Control>,
    fut: impl std::future::Future<Output = T>,
) -> std::result::Result<T, Control> {
    let now = *ctl.borrow();
    if now != Control::Run {
        return Err(now);
    }
    tokio::select! {
        biased;
        changed = ctl.changed() => {
            // Sender weg gilt wie Abbruch.
            let state = if changed.is_ok() { *ctl.borrow() } else { Control::Cancel };
            if state == Control::Run { Err(Control::Run) } else { Err(state) }
        }
        value = fut => Ok(value),
    }
}

fn outcome_for(state: Control) -> Outcome {
    if state == Control::Pause {
        Outcome::Paused
    } else {
        Outcome::Canceled
    }
}

/// Führt die Übertragung aus. `on_start(total, start)` läuft nach den Antwortköpfen (Platzprüfung),
/// `on_progress(done, total)` nach jedem Datenstück.
pub(crate) async fn run(
    t: Transfer<'_>,
    ctl: &mut watch::Receiver<Control>,
    on_start: &mut (dyn FnMut(Option<u64>, u64) -> Result<()> + Send),
    on_progress: &mut (dyn FnMut(u64, Option<u64>) + Send),
) -> Result<Outcome> {
    if let Some(dir) = t.part.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    let mut existing = if t.allow_resume {
        part_len(t.part).await
    } else {
        0
    };
    // Höchstens ein zweiter Anlauf ohne Range (416/200-Sonderfälle).
    let mut resp = loop {
        let mut req = t
            .http
            .get(t.url)
            .header(reqwest::header::AUTHORIZATION, t.auth);
        if existing > 0 {
            req = req.header(reqwest::header::RANGE, format!("bytes={existing}-"));
        }
        let resp = match guarded(ctl, req.send()).await {
            Ok(r) => r.map_err(net_err)?,
            Err(Control::Run) => continue,
            Err(state) => return Ok(outcome_for(state)),
        };
        if resp.status() == StatusCode::RANGE_NOT_SATISFIABLE && existing > 0 {
            let total = resp
                .headers()
                .get(reqwest::header::CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                .and_then(parse_unsatisfied);
            if total == Some(existing) {
                // Datei war schon vollständig.
                on_start(total, existing)?;
                let sha = sha256_file(t.part).await?;
                return Ok(Outcome::Done {
                    size: existing,
                    sha256: sha,
                });
            }
            existing = 0;
            tokio::fs::remove_file(t.part).await.ok();
            continue;
        }
        break resp;
    };

    let status = resp.status();
    let (start, total) = if status == StatusCode::PARTIAL_CONTENT {
        let range = resp
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .and_then(parse_content_range);
        match range {
            Some((a, _, total)) if a == existing && existing > 0 => (existing, total),
            _ => return Err(OfflineError::Network("unerwarteter Datenbereich".into())),
        }
    } else if status == StatusCode::OK {
        // Voller Inhalt: bei null beginnen, auch wenn eine Fortsetzung angefragt war.
        (0, resp.content_length())
    } else {
        return Err(status_err(status));
    };
    on_start(total, start)?;

    let mut hasher = Sha256::new();
    let mut file = if start > 0 {
        // Bereits geladenen Teil in die Prüfsumme einrechnen.
        let mut old = tokio::fs::File::open(t.part).await?;
        let mut buf = vec![0u8; 64 * 1024];
        let mut left = start;
        while left > 0 {
            let want = buf.len().min(left as usize);
            let n = old.read(&mut buf[..want]).await?;
            if n == 0 {
                return Err(OfflineError::Network("Teildatei zu kurz".into()));
            }
            hasher.update(&buf[..n]);
            left -= n as u64;
        }
        tokio::fs::OpenOptions::new()
            .append(true)
            .open(t.part)
            .await?
    } else {
        tokio::fs::File::create(t.part).await?
    };

    let mut done = start;
    loop {
        let next = match guarded(ctl, tokio::time::timeout(t.idle, resp.chunk())).await {
            Ok(Ok(r)) => r.map_err(net_err)?,
            Ok(Err(_)) => return Err(OfflineError::Stalled),
            Err(Control::Run) => continue,
            Err(state) => {
                file.flush().await?;
                return Ok(outcome_for(state));
            }
        };
        let Some(chunk) = next else { break };
        file.write_all(&chunk).await?;
        hasher.update(&chunk);
        done += chunk.len() as u64;
        on_progress(done, total);
    }
    file.flush().await?;
    file.sync_all().await.ok();
    drop(file);

    if let Some(total) = total {
        if done > total {
            return Err(OfflineError::SizeMismatch {
                expected: total,
                actual: done,
            });
        }
        if done < total {
            // Strom endete zu früh: später per Range fortsetzen.
            return Err(OfflineError::Network("Übertragung unvollständig".into()));
        }
    }
    Ok(Outcome::Done {
        size: done,
        sha256: hex(&hasher.finalize()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_range_forms() {
        assert_eq!(
            parse_content_range("bytes 10-19/100"),
            Some((10, 19, Some(100)))
        );
        assert_eq!(parse_content_range("bytes 0-9/*"), Some((0, 9, None)));
        assert_eq!(parse_content_range("items 0-1/2"), None);
        assert_eq!(parse_unsatisfied("bytes */100"), Some(100));
        assert_eq!(parse_unsatisfied("bytes 1-2/3"), None);
    }
}
