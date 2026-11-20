//! Mock-Endpunkte für den Offline-Modus (`lfx-offline`): Download mit `Range`, umgewandelter MP4-Strom,
//! Abbruch-/Halte-Hilfen für Tests, `GET /UserItems/{id}/UserData` mit `LastPlayedDate` und ein optionaler
//! externer Untertitel im Element-JSON. Alle Daten sind erfunden. Der Zustand liegt in `Dyn.w35`
//! (je Mock-Instanz, kein globaler Zustand), damit parallele Tests sich nicht stören.

use super::{Dyn, MockHandle, S, Shared, find, jv, render};
use crate::media::parse_range;
use axum::Router;
use axum::extract::{Path, RawQuery, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use futures_util::stream;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io;
use std::sync::Arc;
use tokio::sync::Notify;

/// Größe der Teilstücke, in denen der Mock Inhalte sendet.
const CHUNK: u64 = 16 * 1024;
/// Länge des umgewandelten Stroms (`/Videos/{id}/stream.mp4`).
pub const TRANSCODE_LEN: u64 = 90_000;

/// Aufgezeichneter Abruf eines Download- oder Umwandlungs-Endpunkts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadHit {
    /// Pfad ohne Query.
    pub path: String,
    /// Wert des `Range`-Headers, falls gesendet.
    pub range: Option<String>,
    /// Rohe Query (bei Umwandlung: Zielcodecs und Bitrate; enthält nie ein Token).
    pub query: String,
}

/// Benutzerdaten, wie der Mock sie dem Server-Zustand nach kennt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerUserData {
    /// Position in Ticks.
    pub position_ticks: i64,
    /// Gesehen?
    pub played: bool,
    /// Zuletzt gespielt (ISO-8601), falls gesetzt.
    pub last_played: Option<String>,
}

#[derive(Default)]
pub(crate) struct W35State {
    no_range: bool,
    forbid_download: bool,
    abort_after: Option<u64>,
    hold_after: Option<u64>,
    release: Arc<Notify>,
    hits: Vec<DownloadHit>,
    last_played: HashMap<String, String>,
    external_subs: bool,
}

/// Länge des Download-Inhalts eines Elements (je Id verschieden, 150 bis 190 KB).
pub fn download_len(id: &str) -> u64 {
    150_000 + (u64::from(seed(id)) % 40) * 1_000
}

fn seed(id: &str) -> u32 {
    id.bytes().fold(2_166_136_261u32, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(16_777_619)
    })
}

fn byte_at(id: &str, i: u64) -> u8 {
    (i.wrapping_mul(31) ^ (i >> 8) ^ u64::from(seed(id))) as u8
}

/// Erwarteter Inhalt der Datei eines Elements (positionsabhängig, damit falsches Zusammensetzen auffällt).
pub fn download_bytes(id: &str) -> Vec<u8> {
    (0..download_len(id)).map(|i| byte_at(id, i)).collect()
}

/// Erwarteter Inhalt des umgewandelten Stroms eines Elements.
pub fn transcode_bytes(id: &str) -> Vec<u8> {
    (0..TRANSCODE_LEN).map(|i| byte_at(id, i) ^ 0x5a).collect()
}

impl MockHandle {
    /// Schaltet die `Range`-Unterstützung des Downloads ab (Antwort immer 200 mit vollem Inhalt).
    pub fn set_download_range(&self, on: bool) {
        self.shared.lock().w35.no_range = !on;
    }

    /// Download verbieten (403, wie ohne Recht `EnableContentDownloading`).
    pub fn forbid_download(&self, on: bool) {
        self.shared.lock().w35.forbid_download = on;
    }

    /// Der nächste Abruf bricht nach `bytes` gesendeten Bytes die Verbindung ab (einmalig).
    pub fn abort_next_download_after(&self, bytes: u64) {
        self.shared.lock().w35.abort_after = Some(bytes);
    }

    /// Der nächste Abruf hält nach `bytes` gesendeten Bytes an, bis [`MockHandle::release_download`] (einmalig).
    pub fn hold_next_download_after(&self, bytes: u64) {
        self.shared.lock().w35.hold_after = Some(bytes);
    }

    /// Gibt einen angehaltenen Abruf wieder frei (wirkt auch, wenn er noch nicht wartet).
    pub fn release_download(&self) {
        self.shared.lock().w35.release.notify_one();
    }

    /// Aufgezeichnete Download- und Umwandlungs-Abrufe.
    pub fn download_hits(&self) -> Vec<DownloadHit> {
        self.shared.lock().w35.hits.clone()
    }

    /// Fügt den Medienquellen im Element-JSON einen externen Untertitel (Index 5) hinzu.
    pub fn set_external_subtitles(&self, on: bool) {
        self.shared.lock().w35.external_subs = on;
    }

    /// Setzt Benutzerdaten auf dem „Server“ (Position, gesehen, Zeitpunkt der letzten Wiedergabe).
    pub fn set_server_user_data(
        &self,
        id: &str,
        position_ticks: i64,
        played: bool,
        last_played: Option<&str>,
    ) {
        let mut d = self.shared.lock();
        d.users.positions.insert(id.to_owned(), position_ticks);
        if played {
            d.users.played.insert(id.to_owned());
        } else {
            d.users.played.remove(id);
        }
        match last_played {
            Some(v) => d.w35.last_played.insert(id.to_owned(), v.to_owned()),
            None => d.w35.last_played.remove(id),
        };
    }

    /// Liest die Benutzerdaten des Servers (nach Meldungen des Clients).
    pub fn server_user_data(&self, id: &str) -> ServerUserData {
        let d = self.shared.lock();
        ServerUserData {
            position_ticks: d.users.positions.get(id).copied().unwrap_or(0),
            played: d.users.played.contains(id),
            last_played: d.w35.last_played.get(id).cloned(),
        }
    }
}

pub(crate) fn routes() -> Router<Arc<Shared>> {
    Router::new().route("/UserItems/{id}/UserData", get(user_data_get))
}

/// Merkt `LastPlayedDate` aus dem Body von `POST /UserItems/{id}/UserData` (Aufruf aus `extra.rs`).
pub(crate) fn note_user_data(d: &mut Dyn, id: &str, body: &Value) {
    if let Some(v) = body.get("LastPlayedDate").and_then(Value::as_str) {
        d.w35.last_played.insert(id.to_owned(), v.to_owned());
    }
}

/// Ergänzt das Element-JSON um den externen Untertitel (nur wenn eingeschaltet; Aufruf aus `render`).
pub(crate) fn decorate(shared: &Shared, v: &mut Value) {
    if !shared.lock().w35.external_subs {
        return;
    }
    let id = v["Id"].as_str().unwrap_or("").to_owned();
    let Some(sources) = v.get_mut("MediaSources").and_then(Value::as_array_mut) else {
        return;
    };
    for source in sources {
        let sid = source["Id"].as_str().unwrap_or("").to_owned();
        if let Some(streams) = source["MediaStreams"].as_array_mut() {
            streams.push(json!({
                "Index": 5, "Type": "Subtitle", "Codec": "srt", "Language": "fra",
                "DisplayTitle": "Französisch (SRT, extern)", "IsTextSubtitleStream": true,
                "IsExternal": true, "IsDefault": false,
                "DeliveryUrl": format!("/Videos/{id}/{sid}/Subtitles/5/0/Stream.srt"),
            }));
        }
    }
}

fn record(shared: &Shared, path: String, headers: &HeaderMap, query: Option<String>) {
    let range = headers
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    shared.lock().w35.hits.push(DownloadHit {
        path,
        range,
        query: query.unwrap_or_default(),
    });
}

/// `GET /Items/{id}/Download` für Katalogeinträge mit `Range`; andere Ids bedient `extra.rs`.
pub(crate) async fn download(
    State(shared): S,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let playable = find(&shared, &id).is_some_and(|i| matches!(i.kind, "Movie" | "Episode"));
    if !playable {
        return crate::extra::download(State(shared), Path(id)).await;
    }
    if shared.lock().w35.forbid_download {
        return StatusCode::FORBIDDEN.into_response();
    }
    record(&shared, format!("/Items/{id}/Download"), &headers, None);
    let ranges = !shared.lock().w35.no_range;
    let real = crate::w5_downloads::real_video(); // W5-downloads
    serve(
        &shared,
        &Body {
            id: &id,
            total: real
                .as_ref()
                .map_or_else(|| download_len(&id), |r| r.len() as u64),
            transcode: false,
            ranges,
            ctype: "video/x-matroska",
            real,
        },
        &headers,
    )
}

/// `GET /Videos/{id}/stream.mp4`: umgewandelter Strom ohne Länge und ohne `Range` (wie ein echter Server).
pub(crate) async fn transcoded(
    State(shared): S,
    Path(id): Path<String>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
) -> Response {
    if find(&shared, &id).is_none() {
        return StatusCode::NOT_FOUND.into_response();
    }
    record(&shared, format!("/Videos/{id}/stream.mp4"), &headers, query);
    serve(
        &shared,
        &Body {
            id: &id,
            total: TRANSCODE_LEN,
            transcode: true,
            ranges: false,
            ctype: "video/mp4",
            real: None, // W5-downloads
        },
        &headers,
    )
}

/// Beschreibung des auszuliefernden Inhalts.
struct Body<'a> {
    id: &'a str,
    total: u64,
    transcode: bool,
    ranges: bool,
    ctype: &'a str,
    /// W5-downloads: echte Videodatei statt Mustererzeugung.
    real: Option<Arc<Vec<u8>>>,
}

/// Zustand der Stromerzeugung.
struct Gen {
    id: String,
    pos: u64,
    sent: u64,
    len: u64,
    transcode: bool,
    abort: Option<u64>,
    hold: Option<u64>,
    release: Arc<Notify>,
    real: Option<Arc<Vec<u8>>>, // W5-downloads
}

/// Baut die Antwort: optional mit `Range`, Inhalt in Teilstücken, Abbruch/Halten einmalig aus dem Zustand.
fn serve(shared: &Arc<Shared>, what: &Body<'_>, headers: &HeaderMap) -> Response {
    let (abort, hold, release) = {
        let mut d = shared.lock();
        (
            d.w35.abort_after.take(),
            d.w35.hold_after.take(),
            d.w35.release.clone(),
        )
    };
    let total = what.total;
    let range = what
        .ranges
        .then(|| headers.get(header::RANGE).and_then(|v| v.to_str().ok()))
        .flatten()
        .map(|r| parse_range(r, total));
    let (start, end, status) = match range {
        None => (0, total - 1, StatusCode::OK),
        Some(Some((a, b))) => (a, b, StatusCode::PARTIAL_CONTENT),
        Some(None) => {
            return (
                StatusCode::RANGE_NOT_SATISFIABLE,
                [(header::CONTENT_RANGE, format!("bytes */{total}"))],
            )
                .into_response();
        }
    };
    let len = end - start + 1;
    let gen_state = Gen {
        id: what.id.to_owned(),
        pos: start,
        sent: 0,
        len,
        transcode: what.transcode,
        abort,
        hold,
        release,
        real: what.real.clone(), // W5-downloads
    };
    let body = axum::body::Body::from_stream(stream::unfold(gen_state, next_chunk));
    let mut resp = Response::new(body);
    *resp.status_mut() = status;
    let h = resp.headers_mut();
    h.insert(header::CONTENT_TYPE, what.ctype.parse().expect("Typ"));
    if !what.transcode {
        h.insert(
            header::CONTENT_LENGTH,
            len.to_string().parse().expect("Länge"),
        );
    }
    if what.ranges {
        h.insert(header::ACCEPT_RANGES, "bytes".parse().expect("Header"));
    }
    if status == StatusCode::PARTIAL_CONTENT {
        h.insert(
            header::CONTENT_RANGE,
            format!("bytes {start}-{end}/{total}")
                .parse()
                .expect("Bereich"),
        );
    }
    resp
}

async fn next_chunk(mut g: Gen) -> Option<(Result<bytes::Bytes, io::Error>, Gen)> {
    if g.sent >= g.len {
        return None;
    }
    if g.hold.is_some_and(|h| g.sent >= h) {
        g.release.notified().await;
        g.hold = None;
    }
    if g.abort.is_some_and(|a| g.sent >= a) {
        // Kurz anhalten, damit hyper das bisher Gesendete zustellt, bevor die Verbindung abbricht.
        tokio::time::sleep(std::time::Duration::from_millis(40)).await;
        g.sent = g.len;
        return Some((
            Err(io::Error::new(io::ErrorKind::ConnectionAborted, "Abbruch")),
            g,
        ));
    }
    // Bis zur nächsten Grenze (Abbruch/Halten) oder zum Teilstück-Ende senden.
    let mut n = CHUNK.min(g.len - g.sent);
    for limit in [g.abort, g.hold].into_iter().flatten() {
        if limit > g.sent {
            n = n.min(limit - g.sent);
        }
    }
    let data: Vec<u8> = (g.pos..g.pos + n)
        .map(|i| {
            if let Some(real) = &g.real {
                return real[i as usize]; // W5-downloads
            }
            let b = byte_at(&g.id, i);
            if g.transcode { b ^ 0x5a } else { b }
        })
        .collect();
    g.pos += n;
    g.sent += n;
    Some((Ok(bytes::Bytes::from(data)), g))
}

async fn user_data_get(State(shared): S, Path(id): Path<String>) -> Response {
    let Some(it) = find(&shared, &id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut data = render(&shared, it, "")["UserData"].clone();
    if let Some(v) = shared.lock().w35.last_played.get(&id) {
        data["LastPlayedDate"] = json!(v);
    }
    jv(data)
}
