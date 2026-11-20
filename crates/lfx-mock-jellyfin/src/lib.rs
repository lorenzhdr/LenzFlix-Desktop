//! Synthetischer Jellyfin-Mock für Tests von LENZFLIX Desktop.
//!
//! - Prüft den `Authorization`-Header (Format und Token) wie ein echter Server.
//! - Simuliert Anmeldung per Name/Passwort und Quick Connect.
//! - Zeichnet PlaybackInfo, Playing, Progress und Stopped auf ([`MockHandle::recorded`]).
//! - Verzögerung und Fehler einstellbar ([`MockHandle::set_delay`], [`MockHandle::fail_next`]).
//! - Daten sind komplett erfunden (siehe [`catalog`]).

pub mod catalog;
mod extra;
pub mod media;
pub mod w35_offline;

pub use extra::{
    VIEW_BOOKS, VIEW_LIVETV, VIEW_MUSIC, VIEW_PHOTOS, album_id, artist_id, book_id, channel_id,
    make_png, recording_id, track_id,
};

use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use catalog::{Item, SERVER_ID, TICKS_PER_SECOND, USER_ID, UserState};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

/// Benutzername des Testkontos.
pub const DEMO_USER: &str = "demo";
/// Passwort des Testkontos.
pub const DEMO_PASSWORD: &str = "demo-passwort";
/// Benutzername des zweiten Testkontos (für Benutzerwechsel).
pub const GUEST_USER: &str = "gast";
/// Passwort des zweiten Testkontos.
pub const GUEST_PASSWORD: &str = "gast-passwort";
/// Benutzer-Id des zweiten Testkontos.
pub const GUEST_ID: &str = "00000000000000000000000000a00002";

/// Standard-Branding-CSS des Mocks: zeigt mit `{BASE}/cw` auf den eingebauten Charts-Worker-Nachbau.
pub const DEFAULT_BRANDING: &str = "/* Mock-Branding */
:root {
  --lfx-charts-url: \"{BASE}/cw\";
  --lfx-stats-url: \"/stats\";
  --lfx-tv-channels: \"5=Action;6=Komödie;99=Ungültig\";
}
";

/// Anfangskonfiguration.
#[derive(Debug, Clone)]
pub struct MockConfig {
    /// Quick Connect eingeschaltet?
    pub quick_connect_enabled: bool,
    /// Wenn gesetzt: Quick Connect gilt nach so vielen Abfragen automatisch als bestätigt.
    pub quick_connect_auto_after_polls: Option<u32>,
    /// Verzögerung je Anfrage.
    pub delay: Duration,
}

impl Default for MockConfig {
    fn default() -> Self {
        Self {
            quick_connect_enabled: true,
            quick_connect_auto_after_polls: None,
            delay: Duration::ZERO,
        }
    }
}

/// Verhalten der PlaybackInfo-Antwort (für Tests der Wiedergabe-Vorbereitung).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlaybackMode {
    /// Direktwiedergabe, eingebettete Untertitel (Standard).
    #[default]
    Direct,
    /// Wie `Direct`, zusätzlich ein externer Untertitel (Index 5, `DeliveryUrl`).
    ExternalSubtitle,
    /// Der Server kann nicht direkt liefern: ohne erlaubte Umwandlung `NoCompatibleStream`,
    /// sonst eine HLS-`TranscodingUrl` (mit `api_key` und `StartTimeTicks`, wie ein echter Server).
    ForceTranscode,
}

/// Aufgezeichnete Wiedergabe-Anfrage.
#[derive(Debug, Clone, PartialEq)]
pub enum Recorded {
    /// PlaybackInfo (Element-Id und Body).
    PlaybackInfo { item_id: String, body: Value },
    /// `/Sessions/Playing`.
    Playing(Value),
    /// `/Sessions/Playing/Progress`.
    Progress(Value),
    /// `/Sessions/Playing/Stopped`.
    Stopped(Value),
}

/// Eintrag im Anfrageprotokoll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestLog {
    /// HTTP-Methode.
    pub method: String,
    /// Pfad ohne Query.
    pub path: String,
}

#[derive(Default)]
struct Dyn {
    delay: Duration,
    fail_remaining: u32,
    fail_status: u16,
    qc_enabled: bool,
    qc_auto_after: Option<u32>,
    tokens: Vec<String>,
    /// Zu welchem Konto ein Token gehört (`true` = zweites Konto).
    token_guest: HashMap<String, bool>,
    qc: HashMap<String, Qc>,
    next_token: u32,
    next_qc: u32,
    users: UserState,
    recorded: Vec<Recorded>,
    requests: Vec<RequestLog>,
    last_auth: Option<String>,
    /// Branding-CSS-Vorlage (`{BASE}` wird durch die Adresse aus dem Host-Header ersetzt); `None` = leer.
    branding: Option<String>,
    /// Charts-Worker-Nachbau unter `/cw/` eingeschaltet?
    charts_enabled: bool,
    /// Verhalten von PlaybackInfo.
    playback_mode: PlaybackMode,
    /// Zustand der erweiterten Endpunkte (`extra.rs`).
    extra: extra::ExtraState,
    /// Zustand der Offline-Endpunkte (`w35_offline.rs`).
    w35: w35_offline::W35State,
}

struct Qc {
    code: String,
    authorized: bool,
    polls: u32,
}

struct Shared {
    catalog: Vec<Item>,
    dynamic: Mutex<Dyn>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Dyn> {
        self.dynamic.lock().unwrap_or_else(|e| e.into_inner())
    }
}

type S = State<Arc<Shared>>;
type Params = Query<HashMap<String, String>>;

/// Steuerung des laufenden Mocks. Beim Verwerfen wird der Server beendet.
pub struct MockHandle {
    shared: Arc<Shared>,
    shutdown: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
}

impl MockHandle {
    /// Alle aufgezeichneten Wiedergabe-Anfragen in Reihenfolge.
    pub fn recorded(&self) -> Vec<Recorded> {
        self.shared.lock().recorded.clone()
    }
    /// Anfrageprotokoll.
    pub fn requests(&self) -> Vec<RequestLog> {
        self.shared.lock().requests.clone()
    }
    /// Zuletzt empfangener `Authorization`-Header (roh).
    pub fn last_auth_header(&self) -> Option<String> {
        self.shared.lock().last_auth.clone()
    }
    /// Bisher ausgegebene Tokens (für Tests, dass sie nirgends auftauchen).
    pub fn issued_tokens(&self) -> Vec<String> {
        self.shared.lock().tokens.clone()
    }
    /// Verzögerung je Anfrage ändern.
    pub fn set_delay(&self, delay: Duration) {
        self.shared.lock().delay = delay;
    }
    /// Die nächsten `count` Anfragen (beliebiger Pfad) mit `status` beantworten.
    pub fn fail_next(&self, count: u32, status: u16) {
        let mut d = self.shared.lock();
        d.fail_remaining = count;
        d.fail_status = status;
    }
    /// Quick Connect ein-/ausschalten.
    pub fn set_quick_connect_enabled(&self, on: bool) {
        self.shared.lock().qc_enabled = on;
    }
    /// Bestätigt alle offenen Quick-Connect-Anfragen (wie ein Nutzer in einer angemeldeten Sitzung).
    pub fn authorize_quick_connect(&self) -> usize {
        let mut d = self.shared.lock();
        d.qc.values_mut()
            .filter(|q| !q.authorized)
            .map(|q| q.authorized = true)
            .count()
    }
    /// Macht alle ausgegebenen Tokens ungültig (Server antwortet danach mit 401, wie nach Ablauf).
    pub fn revoke_tokens(&self) {
        let mut d = self.shared.lock();
        d.tokens.clear();
        d.token_guest.clear();
    }
    /// Branding-CSS ersetzen (`{BASE}` = Adresse des Mocks); `None` = leere Antwort.
    pub fn set_branding_css(&self, css: Option<&str>) {
        self.shared.lock().branding = css.map(str::to_owned);
    }
    /// Charts-Worker-Nachbau (`/cw/charts`, `/cw/me/stats`) ein-/ausschalten (aus = 404).
    pub fn set_charts_enabled(&self, on: bool) {
        self.shared.lock().charts_enabled = on;
    }
    /// Verhalten der PlaybackInfo-Antwort umstellen.
    pub fn set_playback_mode(&self, mode: PlaybackMode) {
        self.shared.lock().playback_mode = mode;
    }
    /// Aufzeichnungen und Anfrageprotokoll leeren.
    pub fn clear_recorded(&self) {
        let mut d = self.shared.lock();
        d.recorded.clear();
        d.requests.clear();
    }
    /// Server beenden und warten.
    pub async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

impl Drop for MockHandle {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

/// Id des n-ten Beispielfilms.
pub fn sample_movie_id(n: usize) -> String {
    catalog::movie_id(n)
}
/// Id der n-ten Beispielserie.
pub fn sample_series_id(n: usize) -> String {
    catalog::series_id(n)
}

/// Startet den Mock auf 127.0.0.1 (freier Port) mit Standardkonfiguration.
pub async fn start() -> (SocketAddr, MockHandle) {
    start_with(MockConfig::default()).await
}

/// Startet den Mock mit eigener Konfiguration.
pub async fn start_with(config: MockConfig) -> (SocketAddr, MockHandle) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Port binden");
    let addr = listener.local_addr().expect("Adresse");
    let dynamic = Dyn {
        delay: config.delay,
        qc_enabled: config.quick_connect_enabled,
        qc_auto_after: config.quick_connect_auto_after_polls,
        branding: Some(DEFAULT_BRANDING.to_owned()),
        charts_enabled: true,
        ..Dyn::default()
    };
    let shared = Arc::new(Shared {
        catalog: catalog::build(),
        dynamic: Mutex::new(dynamic),
    });
    let app = router(shared.clone());
    let (tx, rx) = oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = rx.await;
            })
            .await;
    });
    (
        addr,
        MockHandle {
            shared,
            shutdown: Some(tx),
            task: Some(task),
        },
    )
}

fn router(shared: Arc<Shared>) -> Router {
    Router::new()
        .route("/System/Info/Public", get(public_info))
        .route("/System/Ping", get(ping).post(ping))
        .route("/Users/Public", get(users_public))
        .route("/Users/AuthenticateByName", post(auth_by_name))
        .route(
            "/Users/AuthenticateWithQuickConnect",
            post(auth_quick_connect),
        )
        .route("/QuickConnect/Enabled", get(qc_enabled))
        .route("/QuickConnect/Initiate", post(qc_initiate))
        .route("/QuickConnect/Connect", get(qc_connect))
        .route("/QuickConnect/Authorize", post(qc_authorize))
        .route("/Branding/Css", get(branding_css))
        .route("/Branding/Css.css", get(branding_css))
        .route("/cw/charts", get(cw_charts))
        .route("/cw/me/stats", get(cw_me_stats))
        .route("/Sessions/Logout", post(logout))
        .route("/Users/Me", get(me))
        .route("/UserViews", get(views))
        .route("/Items", get(items))
        .route("/Items/Latest", get(latest))
        .route("/Items/{id}", get(item))
        .route("/Items/{id}/Similar", get(similar))
        .route("/Items/{id}/PlaybackInfo", post(playback_info))
        .route("/Items/{id}/Images/{kind}", get(image))
        .route("/Videos/{id}/stream", get(media::stream))
        .route(
            "/Videos/{id}/{source}/Subtitles/{index}/0/{file}",
            get(subtitle_file),
        )
        .route("/UserItems/Resume", get(resume))
        .route("/Shows/NextUp", get(next_up))
        .route("/Shows/{id}/Seasons", get(seasons))
        .route("/Shows/{id}/Episodes", get(episodes))
        .route("/Genres", get(genres))
        .route(
            "/UserFavoriteItems/{id}",
            post(favorite_on).delete(favorite_off),
        )
        .route("/UserPlayedItems/{id}", post(played_on).delete(played_off))
        .route(
            "/UserItems/{id}/Rating",
            post(rating_set).delete(rating_clear),
        )
        .route("/MediaSegments/{id}", get(segments))
        .route("/Sessions/Playing", post(playing))
        .route("/Sessions/Playing/Progress", post(progress))
        .route("/Sessions/Playing/Stopped", post(stopped))
        .merge(extra::routes())
        .merge(w35_offline::routes())
        .layer(middleware::from_fn_with_state(shared.clone(), gate))
        .with_state(shared)
}

// ---------------------------------------------------------------- Middleware

fn parse_auth(value: &str) -> Option<HashMap<String, String>> {
    let rest = value.strip_prefix("MediaBrowser ")?;
    let mut map = HashMap::new();
    for part in rest.split(", ") {
        let (k, v) = part.split_once('=')?;
        let v = v.strip_prefix('"')?.strip_suffix('"')?;
        map.insert(k.to_owned(), v.to_owned());
    }
    Some(map)
}

fn is_public(method: &str, path: &str) -> bool {
    matches!(
        (method, path),
        ("GET", "/System/Info/Public")
            | ("GET" | "POST", "/System/Ping")
            | ("GET", "/Users/Public")
            | ("POST", "/Users/AuthenticateByName")
            | ("POST", "/Users/AuthenticateWithQuickConnect")
            | ("GET", "/QuickConnect/Enabled")
            | ("POST", "/QuickConnect/Initiate")
            | ("GET", "/QuickConnect/Connect")
    )
}

async fn gate(State(shared): S, req: Request, next: Next) -> Response {
    let method = req.method().as_str().to_owned();
    let path = req.uri().path().to_owned();
    let auth = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let delay = {
        let mut d = shared.lock();
        d.requests.push(RequestLog {
            method: method.clone(),
            path: path.clone(),
        });
        d.last_auth = auth.clone();
        d.delay
    };
    if !delay.is_zero() {
        tokio::time::sleep(delay).await;
    }
    // Branding ist wie beim echten Server anonym (ohne Auth-Header); der Charts-Worker-Nachbau
    // prüft selbst `X-Emby-Token`.
    if path.starts_with("/cw/") || path.starts_with("/Branding/") {
        return next.run(req).await;
    }
    {
        let mut d = shared.lock();
        if d.fail_remaining > 0 {
            d.fail_remaining -= 1;
            return StatusCode::from_u16(d.fail_status)
                .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
                .into_response();
        }
    }
    let Some(fields) = auth.as_deref().and_then(parse_auth) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    if ["Client", "Device", "DeviceId", "Version"]
        .iter()
        .any(|k| fields.get(*k).is_none_or(|v| v.is_empty()))
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if !is_public(&method, &path) {
        let ok = fields
            .get("Token")
            .is_some_and(|t| shared.lock().tokens.iter().any(|x| x == t));
        if !ok {
            return StatusCode::UNAUTHORIZED.into_response();
        }
    }
    next.run(req).await
}

// ---------------------------------------------------------------- Hilfen

fn jv(v: Value) -> Response {
    Json(v).into_response()
}

fn lower(params: HashMap<String, String>) -> HashMap<String, String> {
    params
        .into_iter()
        .map(|(k, v)| (k.to_ascii_lowercase(), v))
        .collect()
}

fn user_json_for(guest: bool) -> Value {
    let (id, name) = if guest {
        (GUEST_ID, GUEST_USER)
    } else {
        (USER_ID, DEMO_USER)
    };
    json!({
        "Id": id, "Name": name, "ServerId": SERVER_ID, "HasPassword": true,
        "Policy": { "IsAdministrator": false, "EnableMediaPlayback": true, "EnableVideoPlaybackTranscoding": false, "EnableContentDownloading": !guest },
        "Configuration": { "PlayDefaultAudioTrack": true, "SubtitleMode": "Default" }
    })
}

fn auth_result(shared: &Shared, guest: bool) -> Value {
    let mut d = shared.lock();
    d.next_token += 1;
    let token = format!("mock-token-{:04}-{}", d.next_token, "x7Kq9Zp2Lm4Rt8Vw");
    d.tokens.push(token.clone());
    d.token_guest.insert(token.clone(), guest);
    let user = user_json_for(guest);
    json!({ "User": user, "AccessToken": token, "ServerId": SERVER_ID,
            "SessionInfo": { "UserId": user["Id"], "UserName": user["Name"] } })
}

fn find<'a>(shared: &'a Shared, id: &str) -> Option<&'a Item> {
    shared.catalog.iter().find(|i| i.id == id)
}

fn render(shared: &Shared, it: &Item, fields: &str) -> Value {
    let mut v = catalog::render(&shared.catalog, it, &shared.lock().users, fields);
    w35_offline::decorate(shared, &mut v);
    v
}

fn list(
    shared: &Shared,
    items: &[&Item],
    params: &HashMap<String, String>,
    total: usize,
    start: usize,
) -> Response {
    let fields = params.get("fields").map(String::as_str).unwrap_or("");
    let rendered: Vec<Value> = items.iter().map(|i| render(shared, i, fields)).collect();
    jv(json!({ "Items": rendered, "TotalRecordCount": total, "StartIndex": start }))
}

fn csv(params: &HashMap<String, String>, key: &str) -> Vec<String> {
    params
        .get(key)
        .map(|v| {
            v.split(',')
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn limit_of(params: &HashMap<String, String>) -> usize {
    params
        .get("limit")
        .and_then(|v| v.parse().ok())
        .unwrap_or(usize::MAX)
}

fn page<'a>(
    items: Vec<&'a Item>,
    params: &HashMap<String, String>,
) -> (Vec<&'a Item>, usize, usize) {
    let total = items.len();
    let start = params
        .get("startindex")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0usize);
    let paged = items
        .into_iter()
        .skip(start)
        .take(limit_of(params))
        .collect();
    (paged, total, start)
}

// ---------------------------------------------------------------- Öffentlich / Anmeldung

async fn public_info() -> Response {
    jv(
        json!({ "ServerName": "Mock-Videothek", "Version": "10.11.0", "ProductName": "Mock Media Server", "Id": SERVER_ID,
               "LocalAddress": "http://127.0.0.1", "StartupWizardCompleted": true }),
    )
}

async fn auth_by_name(State(shared): S, Json(body): Json<Value>) -> Response {
    let user = body.get("Username").and_then(Value::as_str).unwrap_or("");
    let pw = body.get("Pw").and_then(Value::as_str).unwrap_or("");
    if user == DEMO_USER && pw == DEMO_PASSWORD {
        jv(auth_result(&shared, false))
    } else if user == GUEST_USER && pw == GUEST_PASSWORD {
        jv(auth_result(&shared, true))
    } else {
        StatusCode::UNAUTHORIZED.into_response()
    }
}

async fn qc_enabled(State(shared): S) -> Response {
    let on = shared.lock().qc_enabled;
    jv(json!(on))
}

async fn qc_initiate(State(shared): S) -> Response {
    let mut d = shared.lock();
    if !d.qc_enabled {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    d.next_qc += 1;
    let secret = format!("qc-secret-{:04}-Hd3Fa9Nb6Yc1", d.next_qc);
    let code = format!("{:06}", 123_400 + d.next_qc);
    d.qc.insert(
        secret.clone(),
        Qc {
            code: code.clone(),
            authorized: false,
            polls: 0,
        },
    );
    jv(
        json!({ "Authenticated": false, "Secret": secret, "Code": code, "DeviceId": "mock", "DeviceName": "mock" }),
    )
}

async fn qc_connect(State(shared): S, Query(params): Params) -> Response {
    let params = lower(params);
    let secret = params.get("secret").cloned().unwrap_or_default();
    let mut d = shared.lock();
    let auto = d.qc_auto_after;
    match d.qc.get_mut(&secret) {
        None => StatusCode::NOT_FOUND.into_response(),
        Some(q) => {
            q.polls += 1;
            if auto.is_some_and(|n| q.polls > n) {
                q.authorized = true;
            }
            jv(json!({ "Authenticated": q.authorized, "Secret": secret, "Code": q.code }))
        }
    }
}

async fn qc_authorize(State(shared): S, Query(params): Params) -> Response {
    let code = lower(params).get("code").cloned().unwrap_or_default();
    let mut d = shared.lock();
    match d.qc.values_mut().find(|q| q.code == code) {
        Some(q) => {
            q.authorized = true;
            jv(json!(true))
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn auth_quick_connect(State(shared): S, Json(body): Json<Value>) -> Response {
    let secret = body.get("Secret").and_then(Value::as_str).unwrap_or("");
    let ok = shared.lock().qc.get(secret).is_some_and(|q| q.authorized);
    if ok {
        jv(auth_result(&shared, false))
    } else {
        StatusCode::UNAUTHORIZED.into_response()
    }
}

async fn logout(State(shared): S, headers: HeaderMap) -> Response {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(parse_auth)
        .and_then(|f| f.get("Token").cloned());
    if let Some(t) = token {
        shared.lock().tokens.retain(|x| *x != t);
    }
    StatusCode::NO_CONTENT.into_response()
}

async fn branding_css(State(shared): S, headers: HeaderMap) -> Response {
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("127.0.0.1");
    let css = shared
        .lock()
        .branding
        .clone()
        .unwrap_or_default()
        .replace("{BASE}", &format!("http://{host}"));
    ([(header::CONTENT_TYPE, "text/css; charset=utf-8")], css).into_response()
}

/// Prüft `X-Emby-Token` des Charts-Worker-Nachbaus gegen die ausgegebenen Tokens.
fn cw_authorized(shared: &Shared, headers: &HeaderMap) -> bool {
    let d = shared.lock();
    d.charts_enabled
        && headers
            .get("x-emby-token")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|t| d.tokens.iter().any(|x| x == t))
}

async fn cw_charts(State(shared): S, headers: HeaderMap) -> Response {
    if !shared.lock().charts_enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    if !cw_authorized(&shared, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let movies: Vec<Value> = (0..3)
        .map(|n| json!({ "id": catalog::movie_id(n), "type": "Movie", "viewers": 3 - n, "plays": 4 - n, "seconds": 3600 * (3 - n) }))
        .collect();
    jv(
        json!({ "version": 1, "generatedAt": "2026-01-01T00:00:00Z", "windowDays": 30,
               "metric": "viewers", "movies": movies, "series": [], "all": movies }),
    )
}

async fn cw_me_stats(State(shared): S, headers: HeaderMap, Query(params): Params) -> Response {
    if !shared.lock().charts_enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    if !cw_authorized(&shared, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let range = params.get("range").map_or("30d", String::as_str);
    match extra::stats_json(range) {
        Some(v) => jv(v),
        None => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "bad-range" })),
        )
            .into_response(),
    }
}

async fn me(State(shared): S, headers: HeaderMap) -> Response {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(parse_auth)
        .and_then(|f| f.get("Token").cloned())
        .unwrap_or_default();
    let guest = shared
        .lock()
        .token_guest
        .get(&token)
        .copied()
        .unwrap_or(false);
    let mut user = user_json_for(guest);
    extra::overlay_me(&shared, &mut user, guest);
    jv(user)
}

async fn ping() -> Response {
    jv(json!("Jellyfin Server"))
}

/// Öffentliche Benutzerliste („Wer schaut?“). Das Demo-Konto hat ein Profilbild-Tag, das zweite nicht.
async fn users_public() -> Response {
    jv(json!([
        { "Id": USER_ID, "Name": DEMO_USER, "ServerId": SERVER_ID, "HasPassword": true,
          "PrimaryImageTag": "mock-avatar-tag" },
        { "Id": GUEST_ID, "Name": GUEST_USER, "ServerId": SERVER_ID, "HasPassword": true }
    ]))
}

// ---------------------------------------------------------------- Bibliothek

async fn views(State(shared): S) -> Response {
    let v: Vec<&Item> = shared
        .catalog
        .iter()
        .filter(|i| i.kind == "CollectionFolder")
        .collect();
    let n = v.len();
    extra::extend_views(&shared, list(&shared, &v, &HashMap::new(), n, 0))
}

fn name_key(i: &Item) -> String {
    i.name.to_lowercase()
}

async fn items(State(shared): S, Query(params): Params) -> Response {
    let params = lower(params);
    if let Some(response) = extra::try_items(&shared, &params) {
        return response;
    }
    let include = csv(&params, "includeitemtypes");
    let exclude = csv(&params, "excludeitemtypes");
    let ids = csv(&params, "ids");
    let filters = csv(&params, "filters");
    let years: Vec<i32> = csv(&params, "years")
        .iter()
        .filter_map(|y| y.parse().ok())
        .collect();
    let genres: Vec<String> = params
        .get("genres")
        .map(|g| g.split('|').map(str::to_lowercase).collect())
        .unwrap_or_default();
    let term = params.get("searchterm").map(|t| t.to_lowercase());
    let min_rating: f64 = params
        .get("mincommunityrating")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.0);
    let recursive = params
        .get("recursive")
        .is_some_and(|v| v.eq_ignore_ascii_case("true"));
    let parent = params.get("parentid").cloned();
    let fav_only = params
        .get("isfavorite")
        .is_some_and(|v| v.eq_ignore_ascii_case("true"));
    let users = shared.lock();
    let mut out: Vec<&Item> = shared
        .catalog
        .iter()
        .filter(|i| {
            if !recursive && parent.is_none() {
                return matches!(i.kind, "Movie" | "Series");
            }
            match &parent {
                Some(p) if !recursive => i.parent_id == *p,
                Some(p) => {
                    i.parent_id == *p
                        || i.series_id.as_deref() == Some(p)
                        || (p == catalog::VIEW_SERIES && i.series_id.is_some())
                        || (p == catalog::VIEW_MOVIES && i.kind == "Movie")
                }
                None => i.kind != "CollectionFolder",
            }
        })
        .filter(|i| include.is_empty() || include.iter().any(|k| k.eq_ignore_ascii_case(i.kind)))
        .filter(|i| !exclude.iter().any(|k| k.eq_ignore_ascii_case(i.kind)))
        .filter(|i| ids.is_empty() || ids.contains(&i.id))
        .filter(|i| years.is_empty() || years.contains(&i.year))
        .filter(|i| {
            genres.is_empty()
                || genres
                    .iter()
                    .any(|g| i.genres.iter().any(|x| x.to_lowercase() == *g))
        })
        .filter(|i| {
            term.as_ref()
                .is_none_or(|t| i.name.to_lowercase().contains(t))
        })
        .filter(|i| i.rating >= min_rating)
        .filter(|i| !fav_only || users.users.favorites.contains(&i.id))
        .filter(|i| {
            filters.iter().all(|f| match f.as_str() {
                "IsPlayed" => users.users.played.contains(&i.id),
                "IsUnplayed" => !users.users.played.contains(&i.id),
                "IsFavorite" => users.users.favorites.contains(&i.id),
                "IsResumable" => users.users.positions.get(&i.id).is_some_and(|p| *p > 0),
                _ => true,
            })
        })
        .collect();
    let last_played = |i: &Item| users.users.positions.get(&i.id).copied().unwrap_or(0);
    let sort = csv(&params, "sortby")
        .into_iter()
        .next()
        .unwrap_or_default();
    match sort.as_str() {
        "SortName" => out.sort_by_key(|i| name_key(i)),
        "DateCreated" => out.sort_by(|a, b| a.created.cmp(&b.created)),
        "ProductionYear" | "PremiereDate" => out.sort_by_key(|i| i.year),
        "CommunityRating" => out.sort_by(|a, b| a.rating.total_cmp(&b.rating)),
        "Runtime" => out.sort_by_key(|i| i.runtime_min),
        "DatePlayed" | "PlayCount" => out.sort_by_key(|i| last_played(i)),
        _ => {}
    }
    drop(users);
    if params
        .get("sortorder")
        .is_some_and(|o| o.eq_ignore_ascii_case("Descending"))
    {
        out.reverse();
    }
    let (paged, total, start) = page(out, &params);
    list(&shared, &paged, &params, total, start)
}

async fn latest(State(shared): S, Query(params): Params) -> Response {
    let params = lower(params);
    let include = csv(&params, "includeitemtypes");
    let parent = params.get("parentid").cloned();
    let fields = params.get("fields").cloned().unwrap_or_default();
    let mut v: Vec<&Item> = shared
        .catalog
        .iter()
        .filter(|i| matches!(i.kind, "Movie" | "Series"))
        .filter(|i| include.is_empty() || include.iter().any(|k| k.eq_ignore_ascii_case(i.kind)))
        .filter(|i| parent.as_ref().is_none_or(|p| i.parent_id == *p))
        .collect();
    v.sort_by(|a, b| b.created.cmp(&a.created));
    let arr: Vec<Value> = v
        .into_iter()
        .take(limit_of(&params))
        .map(|i| render(&shared, i, &fields))
        .collect();
    jv(Value::Array(arr))
}

async fn item(State(shared): S, Path(id): Path<String>, Query(params): Params) -> Response {
    let params = lower(params);
    match find(&shared, &id) {
        Some(i) => jv(render(
            &shared,
            i,
            params.get("fields").map(String::as_str).unwrap_or(""),
        )),
        None => {
            extra::try_item(&shared, &id).unwrap_or_else(|| StatusCode::NOT_FOUND.into_response())
        }
    }
}

async fn similar(State(shared): S, Path(id): Path<String>, Query(params): Params) -> Response {
    let params = lower(params);
    let Some(it) = find(&shared, &id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let v: Vec<&Item> = shared
        .catalog
        .iter()
        .filter(|i| i.kind == it.kind && i.id != it.id)
        .take(limit_of(&params))
        .collect();
    let n = v.len();
    list(&shared, &v, &params, n, 0)
}

async fn resume(State(shared): S, Query(params): Params) -> Response {
    let params = lower(params);
    let v: Vec<&Item> = {
        let d = shared.lock();
        shared
            .catalog
            .iter()
            .filter(|i| matches!(i.kind, "Movie" | "Episode"))
            .filter(|i| {
                d.users.positions.get(&i.id).is_some_and(|p| *p > 0)
                    && !d.users.played.contains(&i.id)
            })
            .take(limit_of(&params))
            .collect()
    };
    let n = v.len();
    list(&shared, &v, &params, n, 0)
}

async fn next_up(State(shared): S, Query(params): Params) -> Response {
    let params = lower(params);
    let only = params.get("seriesid").cloned();
    let v: Vec<&Item> = {
        let d = shared.lock();
        let mut out = Vec::new();
        for s in shared
            .catalog
            .iter()
            .filter(|i| i.kind == "Series" && only.as_ref().is_none_or(|o| *o == i.id))
        {
            let eps: Vec<&Item> = shared
                .catalog
                .iter()
                .filter(|e| e.kind == "Episode" && e.series_id.as_deref() == Some(&s.id))
                .collect();
            // Nur Serien, bei denen schon etwas gesehen wurde; dann die erste ungesehene Folge.
            if eps.iter().any(|e| d.users.played.contains(&e.id))
                && let Some(next) = eps.iter().find(|e| !d.users.played.contains(&e.id))
            {
                out.push(*next);
            }
        }
        out.into_iter().take(limit_of(&params)).collect()
    };
    let n = v.len();
    list(&shared, &v, &params, n, 0)
}

async fn seasons(State(shared): S, Path(id): Path<String>, Query(params): Params) -> Response {
    let params = lower(params);
    let v: Vec<&Item> = shared
        .catalog
        .iter()
        .filter(|i| i.kind == "Season" && i.series_id.as_deref() == Some(&id))
        .collect();
    let n = v.len();
    list(&shared, &v, &params, n, 0)
}

async fn episodes(State(shared): S, Path(id): Path<String>, Query(params): Params) -> Response {
    let params = lower(params);
    let season = params.get("seasonid").cloned();
    let v: Vec<&Item> = shared
        .catalog
        .iter()
        .filter(|i| i.kind == "Episode" && i.series_id.as_deref() == Some(&id))
        .filter(|i| {
            season
                .as_ref()
                .is_none_or(|s| i.season_id.as_deref() == Some(s))
        })
        .collect();
    let n = v.len();
    list(&shared, &v, &params, n, 0)
}

async fn genres(State(shared): S) -> Response {
    let mut names: Vec<&str> = shared
        .catalog
        .iter()
        .flat_map(|i| i.genres.iter().copied())
        .collect();
    names.sort_unstable();
    names.dedup();
    let items: Vec<Value> = names
        .iter()
        .enumerate()
        .map(|(n, g)| json!({ "Id": format!("{:032x}", 0x7000_0000u32 + n as u32), "Name": g, "Type": "Genre" }))
        .collect();
    let total = items.len();
    jv(json!({ "Items": items, "TotalRecordCount": total, "StartIndex": 0 }))
}

// ---------------------------------------------------------------- Benutzerdaten

fn user_data(shared: &Shared, id: &str) -> Response {
    match find(shared, id) {
        Some(i) => jv(render(shared, i, "")["UserData"].clone()),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

fn mutate(shared: &Shared, id: &str, f: impl FnOnce(&mut UserState, &str)) -> Response {
    if find(shared, id).is_none() {
        return StatusCode::NOT_FOUND.into_response();
    }
    f(&mut shared.lock().users, id);
    user_data(shared, id)
}

async fn favorite_on(State(s): S, Path(id): Path<String>) -> Response {
    mutate(&s, &id, |u, id| {
        u.favorites.insert(id.to_owned());
    })
}
async fn favorite_off(State(s): S, Path(id): Path<String>) -> Response {
    mutate(&s, &id, |u, id| {
        u.favorites.remove(id);
    })
}
async fn played_on(State(s): S, Path(id): Path<String>) -> Response {
    mutate(&s, &id, |u, id| {
        u.played.insert(id.to_owned());
        u.positions.remove(id);
    })
}
async fn played_off(State(s): S, Path(id): Path<String>) -> Response {
    mutate(&s, &id, |u, id| {
        u.played.remove(id);
    })
}
async fn rating_set(State(s): S, Path(id): Path<String>, Query(params): Params) -> Response {
    let likes = lower(params)
        .get("likes")
        .is_some_and(|v| v.eq_ignore_ascii_case("true"));
    mutate(&s, &id, |u, id| {
        u.likes.insert(id.to_owned(), likes);
    })
}
async fn rating_clear(State(s): S, Path(id): Path<String>) -> Response {
    mutate(&s, &id, |u, id| {
        u.likes.remove(id);
    })
}

async fn segments(State(shared): S, Path(id): Path<String>) -> Response {
    let Some(it) = find(&shared, &id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !matches!(it.kind, "Movie" | "Episode") {
        return jv(json!({ "Items": [], "TotalRecordCount": 0, "StartIndex": 0 }));
    }
    let runtime = it.runtime_min * 60 * TICKS_PER_SECOND;
    jv(json!({ "Items": [
        { "Id": "seg-intro", "ItemId": id, "Type": "Intro", "StartTicks": 30 * TICKS_PER_SECOND, "EndTicks": 90 * TICKS_PER_SECOND },
        { "Id": "seg-outro", "ItemId": id, "Type": "Outro", "StartTicks": runtime - 120 * TICKS_PER_SECOND, "EndTicks": runtime },
    ], "TotalRecordCount": 2, "StartIndex": 0 }))
}

// ---------------------------------------------------------------- Wiedergabe

async fn playback_info(
    State(shared): S,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    if let Some(response) = extra::try_playback_info(&shared, &id) {
        return response;
    }
    let Some(it) = find(&shared, &id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !matches!(it.kind, "Movie" | "Episode") {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let mode = shared.lock().playback_mode;
    let transcoding_allowed = body
        .get("EnableTranscoding")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    shared.lock().recorded.push(Recorded::PlaybackInfo {
        item_id: id.clone(),
        body,
    });
    let session = format!("play-{}", &id[24..]);
    let mut source = catalog::media_source(it);
    let source_id = source["Id"].as_str().unwrap_or("").to_owned();
    match mode {
        PlaybackMode::Direct => {}
        PlaybackMode::ExternalSubtitle => {
            if let Some(streams) = source["MediaStreams"].as_array_mut() {
                streams.push(json!({
                    "Index": 5, "Type": "Subtitle", "Codec": "srt", "Language": "fra",
                    "DisplayTitle": "Französisch (SRT, extern)", "IsTextSubtitleStream": true,
                    "IsExternal": true,
                    "DeliveryUrl": format!("/Videos/{id}/{source_id}/Subtitles/5/0/Stream.srt"),
                }));
            }
        }
        PlaybackMode::ForceTranscode => {
            source["SupportsDirectPlay"] = json!(false);
            source["SupportsDirectStream"] = json!(false);
            if !transcoding_allowed {
                // Die Quelle ist bekannt, aber nichts davon ist für dieses Gerät lieferbar.
                source["SupportsTranscoding"] = json!(false);
                return jv(json!({ "MediaSources": [source], "ErrorCode": "NoCompatibleStream" }));
            }
            source["SupportsDirectPlay"] = json!(false);
            source["SupportsDirectStream"] = json!(false);
            source["SupportsTranscoding"] = json!(true);
            source["TranscodingSubProtocol"] = json!("hls");
            source["TranscodingUrl"] = json!(format!(
                "/videos/{id}/master.m3u8?MediaSourceId={source_id}&PlaySessionId={session}&api_key=mock-key&StartTimeTicks=0&VideoCodec=h264&AudioCodec=aac"
            ));
        }
    }
    jv(json!({ "MediaSources": [source], "PlaySessionId": session }))
}

/// Kleiner externer Untertitel (SRT) für den Modus [`PlaybackMode::ExternalSubtitle`].
async fn subtitle_file(
    State(shared): S,
    Path((id, _source, _index, _file)): Path<(String, String, String, String)>,
) -> Response {
    if find(&shared, &id).is_none() {
        return StatusCode::NOT_FOUND.into_response();
    }
    (
        [(header::CONTENT_TYPE, "application/x-subrip")],
        "1
00:00:01,000 --> 00:00:02,000
Beispieltext

",
    )
        .into_response()
}

/// Prüft, dass die Pflichtfelder einer Fortschrittsmeldung vorhanden sind.
fn valid_report(body: &Value, need_session: bool) -> bool {
    let has = |k: &str| {
        body.get(k)
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty())
    };
    has("ItemId")
        && (!need_session || has("PlaySessionId"))
        && body.get("PositionTicks").is_some_and(Value::is_i64)
}

async fn playing(State(shared): S, Json(body): Json<Value>) -> Response {
    if !valid_report(&body, true) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    shared.lock().recorded.push(Recorded::Playing(body));
    StatusCode::NO_CONTENT.into_response()
}

async fn progress(State(shared): S, Json(body): Json<Value>) -> Response {
    if !valid_report(&body, true) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let (id, pos) = (
        body["ItemId"].as_str().unwrap_or("").to_owned(),
        body["PositionTicks"].as_i64().unwrap_or(0),
    );
    let mut d = shared.lock();
    d.users.positions.insert(id, pos);
    d.recorded.push(Recorded::Progress(body));
    StatusCode::NO_CONTENT.into_response()
}

async fn stopped(State(shared): S, Json(body): Json<Value>) -> Response {
    if !valid_report(&body, true) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let id = body["ItemId"].as_str().unwrap_or("").to_owned();
    let pos = body["PositionTicks"].as_i64().unwrap_or(0);
    let runtime = find(&shared, &id).map_or(0, |i| i.runtime_min * 60 * TICKS_PER_SECOND);
    let mut d = shared.lock();
    if runtime > 0 && pos * 100 >= runtime * 90 {
        d.users.played.insert(id.clone());
        d.users.positions.remove(&id);
    } else {
        d.users.positions.insert(id, pos);
    }
    d.recorded.push(Recorded::Stopped(body));
    StatusCode::NO_CONTENT.into_response()
}

async fn image(State(shared): S, Path((id, _kind)): Path<(String, String)>) -> Response {
    if find(&shared, &id).is_none() && !extra::has_image(&shared, &id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    (
        [(header::CONTENT_TYPE, "image/png")],
        extra::placeholder_png().to_vec(),
    )
        .into_response()
}

async fn stream(State(shared): S, Path(id): Path<String>) -> Response {
    if find(&shared, &id).is_none() {
        return StatusCode::NOT_FOUND.into_response();
    }
    (
        [(header::CONTENT_TYPE, "video/x-matroska")],
        b"FAKE-VIDEO-BYTES".as_slice(),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_ids_are_32_hex() {
        for i in catalog::build() {
            assert_eq!(i.id.len(), 32, "{}", i.name);
            assert!(i.id.bytes().all(|b| b.is_ascii_hexdigit()));
        }
        assert_eq!(catalog::USER_ID.len(), 32);
        assert_eq!(catalog::SERVER_ID.len(), 32);
    }

    #[test]
    fn parses_auth_header() {
        let f = parse_auth(r#"MediaBrowser Client="LENZFLIX", Device="PC", DeviceId="abc", Version="0.1.0", Token="t""#).unwrap();
        assert_eq!(f["Client"], "LENZFLIX");
        assert_eq!(f["Token"], "t");
        assert!(parse_auth("Bearer x").is_none());
    }
}
