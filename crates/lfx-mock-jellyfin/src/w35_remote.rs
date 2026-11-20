//! Mock für Fernsteuerung und SyncPlay: Session-WebSocket `/socket`, Fähigkeitsmeldung, `/GetUtcTime`
//! und die SyncPlay-REST-Endpunkte. Alle Daten sind erfunden.
//!
//! - `/socket` verlangt wie jeder andere Pfad den `Authorization`-Header (Middleware). Beim Verbinden sendet der
//!   Mock `ForceKeepAlive` (Standard 60 s) und reicht danach Nachrichten weiter, die Tests per
//!   [`MockHandle::ws_push`] einspeisen. [`MockHandle::ws_drop_all`] schließt alle Verbindungen (Test der
//!   Wiederverbindung).
//! - SyncPlay: genau eine Sitzung. `New`/`Join`/`Leave` und `Pause`/`Unpause`/`Seek`/`Stop` erzeugen die passenden
//!   WebSocket-Nachrichten (Befehle mit `When` = Serverzeit + 200 ms). Alle Aufrufe stehen in
//!   [`MockHandle::syncplay_calls`].

use super::{MockHandle, S, Shared};
use axum::Router;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Json, Path};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::broadcast;

#[derive(Debug, Clone)]
enum WsCmd {
    Text(String),
    Close,
}

#[derive(Debug, Clone)]
struct MockGroup {
    id: String,
    name: String,
    participants: Vec<String>,
}

struct Data {
    total: u32,
    open: u32,
    received: Vec<String>,
    keepalive_secs: u64,
    capabilities: Vec<Value>,
    utc_offset_micros: i64,
    groups: Vec<MockGroup>,
    joined: Option<String>,
    calls: Vec<(String, Value)>,
    position_ticks: i64,
    next_group: u32,
}

struct Inner {
    data: Mutex<Data>,
    tx: broadcast::Sender<WsCmd>,
}

/// Zustand der Fernsteuer-Endpunkte (Klone teilen ihn).
#[derive(Clone)]
pub(crate) struct RemoteState {
    inner: Arc<Inner>,
}

impl Default for RemoteState {
    fn default() -> Self {
        let (tx, _) = broadcast::channel(64);
        Self {
            inner: Arc::new(Inner {
                data: Mutex::new(Data {
                    total: 0,
                    open: 0,
                    received: Vec::new(),
                    keepalive_secs: 60,
                    capabilities: Vec::new(),
                    utc_offset_micros: 0,
                    groups: Vec::new(),
                    joined: None,
                    calls: Vec::new(),
                    position_ticks: 0,
                    next_group: 0,
                }),
                tx,
            }),
        }
    }
}

impl RemoteState {
    fn data(&self) -> std::sync::MutexGuard<'_, Data> {
        self.inner.data.lock().unwrap_or_else(|e| e.into_inner())
    }
    fn push(&self, text: String) {
        let _ = self.inner.tx.send(WsCmd::Text(text));
    }
    fn server_now_micros(&self) -> i64 {
        let real = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_micros() as i64);
        real + self.data().utc_offset_micros
    }
    fn group_update(&self, group_id: &str, kind: &str, data: Value) {
        self.push(
            json!({ "MessageType": "SyncPlayGroupUpdate",
                    "Data": { "GroupId": group_id, "Type": kind, "Data": data } })
            .to_string(),
        );
    }
    fn group_info(&self, g: &MockGroup) -> Value {
        json!({ "GroupId": g.id, "GroupName": g.name, "State": "Idle",
                "Participants": g.participants,
                "LastUpdatedAt": format_micros(self.server_now_micros()) })
    }
    fn command(&self, command: &str, position_ticks: i64) {
        let Some(group) = self.data().joined.clone() else {
            return;
        };
        let now = self.server_now_micros();
        self.push(
            json!({ "MessageType": "SyncPlayCommand", "Data": {
                "GroupId": group, "PlaylistItemId": "00000000000000000000000000c00001",
                "When": format_micros(now + 200_000), "PositionTicks": position_ticks,
                "Command": command, "EmittedAt": format_micros(now) } })
            .to_string(),
        );
    }
}

/// RFC 3339 mit sieben Nachkommastellen (wie der echte Server) aus Mikrosekunden seit 1970.
fn format_micros(micros: i64) -> String {
    let secs = micros.div_euclid(1_000_000);
    let frac = micros.rem_euclid(1_000_000);
    let z = secs.div_euclid(86_400) + 719_468;
    let rem = secs.rem_euclid(86_400);
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{frac:06}0Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

fn state_of(shared: &Shared) -> RemoteState {
    shared.lock().remote.clone()
}

// ---------------------------------------------------------------- Handler

async fn socket(axum::extract::State(shared): S, ws: WebSocketUpgrade) -> Response {
    let state = state_of(&shared);
    // Vor der Upgrade-Antwort abonnieren: Nachrichten, die der Test nach „verbunden“ einspeist, gehen nie verloren.
    let rx = state.inner.tx.subscribe();
    ws.on_upgrade(move |socket| serve_socket(socket, state, rx))
}

async fn serve_socket(
    mut socket: WebSocket,
    state: RemoteState,
    mut rx: broadcast::Receiver<WsCmd>,
) {
    let keepalive = {
        let mut d = state.data();
        d.total += 1;
        d.open += 1;
        d.keepalive_secs
    };
    let hello = json!({ "MessageType": "ForceKeepAlive", "Data": keepalive }).to_string();
    if socket.send(Message::Text(hello.into())).await.is_ok() {
        loop {
            tokio::select! {
                incoming = socket.recv() => match incoming {
                    Some(Ok(Message::Text(t))) => state.data().received.push(t.as_str().to_owned()),
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    Some(Ok(_)) => {}
                },
                cmd = rx.recv() => match cmd {
                    Ok(WsCmd::Text(t)) => {
                        if socket.send(Message::Text(t.into())).await.is_err() { break; }
                    }
                    Ok(WsCmd::Close) => {
                        let _ = socket.send(Message::Close(None)).await;
                        break;
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => break,
                },
            }
        }
    }
    state.data().open -= 1;
}

async fn capabilities(axum::extract::State(shared): S, body: Option<Json<Value>>) -> StatusCode {
    state_of(&shared)
        .data()
        .capabilities
        .push(body.map_or(Value::Null, |b| b.0));
    StatusCode::NO_CONTENT
}

async fn utc_time(axum::extract::State(shared): S) -> Response {
    let state = state_of(&shared);
    let a = format_micros(state.server_now_micros());
    let b = format_micros(state.server_now_micros());
    Json(json!({ "RequestReceptionTime": a, "ResponseTransmissionTime": b })).into_response()
}

async fn sp_list(axum::extract::State(shared): S) -> Response {
    let state = state_of(&shared);
    let groups = state.data().groups.clone();
    Json(
        groups
            .iter()
            .map(|g| state.group_info(g))
            .collect::<Vec<_>>(),
    )
    .into_response()
}

async fn sp_get(axum::extract::State(shared): S, Path(id): Path<String>) -> Response {
    let state = state_of(&shared);
    let found = state.data().groups.iter().find(|g| g.id == id).cloned();
    match found {
        Some(g) => Json(state.group_info(&g)).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn sp_post(
    axum::extract::State(shared): S,
    Path(action): Path<String>,
    body: Option<Json<Value>>,
) -> Response {
    let state = state_of(&shared);
    let body = body.map_or(Value::Null, |b| b.0);
    state.data().calls.push((action.clone(), body.clone()));
    match action.as_str() {
        "New" => {
            let g = {
                let mut d = state.data();
                d.next_group += 1;
                let g = MockGroup {
                    id: format!("00000000000000000000000000b{:05}", d.next_group),
                    name: body["GroupName"].as_str().unwrap_or("Gruppe").to_owned(),
                    participants: vec!["demo".into()],
                };
                d.groups.push(g.clone());
                d.joined = Some(g.id.clone());
                g
            };
            state.group_update(&g.id, "GroupJoined", state.group_info(&g));
        }
        "Join" => {
            let id = body["GroupId"].as_str().unwrap_or_default().to_owned();
            let found = state.data().groups.iter().find(|g| g.id == id).cloned();
            match found {
                Some(mut g) => {
                    if !g.participants.iter().any(|p| p == "demo") {
                        g.participants.push("demo".into());
                    }
                    state.data().joined = Some(id.clone());
                    state.group_update(&id, "GroupJoined", state.group_info(&g));
                }
                None => state.group_update(&id, "GroupDoesNotExist", Value::Null),
            }
        }
        "Leave" => {
            let left = state.data().joined.take();
            if let Some(id) = left {
                state.group_update(&id, "GroupLeft", json!(id));
            }
        }
        "Pause" => {
            let p = state.data().position_ticks;
            state.command("Pause", p);
        }
        "Unpause" => {
            let p = state.data().position_ticks;
            state.command("Unpause", p);
        }
        "Stop" => state.command("Stop", 0),
        "Seek" => {
            let p = body["PositionTicks"].as_i64().unwrap_or(0);
            state.data().position_ticks = p;
            state.command("Seek", p);
        }
        _ => {}
    }
    StatusCode::NO_CONTENT.into_response()
}

pub(crate) fn routes() -> Router<Arc<Shared>> {
    Router::new()
        .route("/socket", get(socket))
        .route("/Sessions/Capabilities/Full", post(capabilities))
        .route("/Sessions/Capabilities", post(capabilities))
        .route("/GetUtcTime", get(utc_time))
        .route("/SyncPlay/List", get(sp_list))
        .route("/SyncPlay/{x}", get(sp_get).post(sp_post))
}

// ---------------------------------------------------------------- Test-Zugriff

impl MockHandle {
    fn remote(&self) -> RemoteState {
        state_of(&self.shared)
    }

    /// Schickt Text an alle offenen WebSocket-Verbindungen.
    pub fn ws_push(&self, text: &str) {
        self.remote().push(text.to_owned());
    }

    /// Schließt alle offenen WebSocket-Verbindungen (der Server bleibt erreichbar).
    pub fn ws_drop_all(&self) {
        let _ = self.remote().inner.tx.send(WsCmd::Close);
    }

    /// Wie viele Verbindungen wurden insgesamt aufgebaut?
    pub fn ws_connections_total(&self) -> u32 {
        self.remote().data().total
    }

    /// Wie viele Verbindungen sind gerade offen?
    pub fn ws_connections_open(&self) -> u32 {
        self.remote().data().open
    }

    /// Alle vom Client über den WebSocket gesendeten Texte.
    pub fn ws_received(&self) -> Vec<String> {
        self.remote().data().received.clone()
    }

    /// `ForceKeepAlive`-Sekunden für künftige Verbindungen.
    pub fn set_keepalive_secs(&self, secs: u64) {
        self.remote().data().keepalive_secs = secs;
    }

    /// Alle gemeldeten Fähigkeiten (Körper der POST-Anfragen).
    pub fn capabilities_posts(&self) -> Vec<Value> {
        self.remote().data().capabilities.clone()
    }

    /// Lässt die Serveruhr von `/GetUtcTime` und den SyncPlay-Zeiten um so viele Millisekunden vorgehen.
    pub fn set_utc_offset_ms(&self, ms: i64) {
        self.remote().data().utc_offset_micros = ms * 1000;
    }

    /// Legt eine Gruppe an, der noch niemand beigetreten ist.
    pub fn add_syncplay_group(&self, name: &str, participants: &[&str]) -> String {
        let state = self.remote();
        let mut d = state.data();
        d.next_group += 1;
        let id = format!("00000000000000000000000000b{:05}", d.next_group);
        d.groups.push(MockGroup {
            id: id.clone(),
            name: name.to_owned(),
            participants: participants.iter().map(|s| (*s).to_owned()).collect(),
        });
        id
    }

    /// Alle SyncPlay-POSTs als (Aktion, Körper).
    pub fn syncplay_calls(&self) -> Vec<(String, Value)> {
        self.remote().data().calls.clone()
    }
}
