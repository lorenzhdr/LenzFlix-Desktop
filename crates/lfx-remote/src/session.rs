//! Laufzeit: Session-WebSocket mit Wiederverbindung, Fähigkeiten, Nachrichtenverteilung und SyncPlay.
//!
//! [`RemoteSession::start`] startet einen Hintergrund-Task (tokio) und liefert ein [`RemoteHandle`]:
//! - Ereignisse als `tokio::sync::broadcast` ([`RemoteEvent`]), mehrere Abonnenten möglich.
//! - Befehle an den Player über [`PlayerControl`] (Fernsteuerung und SyncPlay-Korrekturen).
//!
//! Lebenszeichen: Der Server sendet `ForceKeepAlive` (Sekunden); der Client sendet dann alle Sekunden/2
//! `KeepAlive` plus einen WebSocket-Ping. Kommt länger als das Dreifache des Fensters kein Frame, gilt die
//! Verbindung als tot und wird neu aufgebaut.

use crate::capabilities::Capabilities;
use crate::enums::{GroupStateType, PlaybackRequestType};
use crate::message::{
    GroupInfo, GroupUpdate, ParseError, PlayQueueUpdate, SendCommand, ServerMessage,
    keep_alive_message, parse_message,
};
use crate::player::{PlayerAction, PlayerControl};
use crate::syncplay::{DriftConfig, Scheduled, SyncCore, SyncReport};
use crate::syncplay_api::SyncPlayApi;
use crate::time::Clock;
use crate::timesync::{TimeSample, TimeSyncPolicy, measure_once};
use crate::ws::{WsAuth, WsError, WsStream, connect};
use futures_util::{SinkExt, StreamExt};
use lfx_api::Client;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{Notify, broadcast, mpsc};
use tokio::task::JoinHandle;
use tokio::time::{Instant, interval_at, sleep};
use tokio_tungstenite::tungstenite::Message;

/// Wartezeiten der Wiederverbindung: `initial`, dann jeweils mal `factor`, höchstens `max`.
/// Bewusst ohne Zufall (deterministisch, Tests). War eine Verbindung mindestens `reset_after` stabil,
/// beginnt die Folge wieder bei `initial`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Backoff {
    /// Erste Wartezeit.
    pub initial: Duration,
    /// Obergrenze.
    pub max: Duration,
    /// Faktor je Fehlschlag.
    pub factor: f64,
    /// Ab dieser Verbindungsdauer gilt sie als stabil.
    pub reset_after: Duration,
}

impl Default for Backoff {
    fn default() -> Self {
        Self {
            initial: Duration::from_secs(1),
            max: Duration::from_secs(60),
            factor: 2.0,
            reset_after: Duration::from_secs(10),
        }
    }
}

impl Backoff {
    /// Wartezeit vor dem `attempt`-ten Wiederholen (0 = erster).
    pub fn delay(&self, attempt: u32) -> Duration {
        let secs = self.initial.as_secs_f64() * self.factor.powi(attempt.min(32) as i32);
        Duration::from_secs_f64(secs.min(self.max.as_secs_f64()))
    }
}

/// Einstellungen der Fernsteuerung.
#[derive(Clone)]
pub struct RemoteConfig {
    /// Angemeldeter Client.
    pub client: Client,
    /// Player der App.
    pub player: Arc<dyn PlayerControl>,
    /// Uhr (Standard: [`crate::SystemClock`]).
    pub clock: Arc<dyn Clock>,
    /// Gemeldete Fähigkeiten.
    pub capabilities: Capabilities,
    /// Art der WebSocket-Anmeldung.
    pub ws_auth: WsAuth,
    /// Wiederverbindung.
    pub backoff: Backoff,
    /// Drift-Schwellen.
    pub drift: DriftConfig,
    /// Zeit-Synchronisation.
    pub time_policy: TimeSyncPolicy,
    /// Takt für Drift-Korrektur und Ready-Meldungen.
    pub tick_interval: Duration,
    /// Lebenszeichen-Fenster bis zur ersten `ForceKeepAlive`-Nachricht.
    pub default_keepalive: Duration,
    /// W5-remote: Play-, Playstate- und Allgemein-Befehle anderer Clients ausführen? SyncPlay läuft unabhängig davon.
    pub allow_remote_control: bool,
}

impl RemoteConfig {
    /// Standardwerte mit Systemuhr.
    pub fn new(client: Client, player: Arc<dyn PlayerControl>) -> Self {
        Self {
            client,
            player,
            clock: Arc::new(crate::time::SystemClock),
            capabilities: Capabilities::default(),
            ws_auth: WsAuth::default(),
            backoff: Backoff::default(),
            drift: DriftConfig::default(),
            time_policy: TimeSyncPolicy::default(),
            tick_interval: Duration::from_millis(500),
            default_keepalive: Duration::from_secs(60),
            allow_remote_control: true, // W5-remote
        }
    }
}

/// Grund einer getrennten Verbindung.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisconnectReason {
    /// Verbindung verloren oder vom Server geschlossen.
    Lost,
    /// Verbindungsaufbau fehlgeschlagen.
    ConnectFailed(String),
    /// Server lehnt die Anmeldung ab; es wird nicht erneut versucht.
    Unauthorized,
}

/// Verbindungszustand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionState {
    /// Noch nie gestartet oder gerade gestoppt.
    Idle,
    /// Verbindungsaufbau läuft.
    Connecting,
    /// Verbunden, Fähigkeiten werden gemeldet.
    Connected,
    /// Getrennt; `retry_in` = Wartezeit bis zum nächsten Versuch (`None` = endgültig).
    Disconnected {
        /// Grund.
        reason: DisconnectReason,
        /// Nächster Versuch.
        retry_in: Option<Duration>,
    },
    /// Per [`RemoteHandle::stop`] beendet.
    Stopped,
}

/// SyncPlay-Ereignisse für die UI.
#[derive(Debug, Clone, PartialEq)]
pub enum SyncPlayEvent {
    /// Gruppe beigetreten.
    Joined(GroupInfo),
    /// Gruppe verlassen oder verloren.
    Left,
    /// Gruppenzustand.
    StateChanged {
        /// Neuer Zustand.
        state: GroupStateType,
        /// Anlass.
        reason: Option<PlaybackRequestType>,
    },
    /// Teilnehmer beigetreten (Anzeigename).
    ParticipantJoined(String),
    /// Teilnehmer ausgetreten.
    ParticipantLeft(String),
    /// Wiedergabeliste geändert.
    Queue(PlayQueueUpdate),
    /// Befehl der Gruppe (bereits an den Player weitergegeben bzw. eingeplant).
    Command(SendCommand),
    /// Neue Zeitmessung.
    TimeSynced {
        /// Server minus lokal, Mikrosekunden.
        offset_micros: i64,
        /// Ping (RTT/2), Mikrosekunden.
        ping_micros: i64,
    },
    /// Server verweigert die Gruppe (kein Zugriff auf Bibliothek, Gruppe weg).
    Denied(String),
}

/// Ereignis-Strom der Fernsteuerung.
#[derive(Debug, Clone)]
pub enum RemoteEvent {
    /// Verbindungszustand geändert.
    Connection(ConnectionState),
    /// Typisierte Server-Nachricht (alle, auch die bereits an den Player gegebenen).
    Message(Arc<ServerMessage>),
    /// SyncPlay.
    SyncPlay(SyncPlayEvent),
    /// Nachricht nicht lesbar (ohne Inhalt).
    ParseError(ParseError),
}

#[derive(Debug)]
struct Shared {
    connection: ConnectionState,
    group: Option<GroupInfo>,
    /// W5-remote: letzte gemessene Abweichung zur Gruppe in Mikrosekunden.
    drift_micros: Option<i64>,
}

/// Griff auf die laufende Fernsteuerung.
pub struct RemoteHandle {
    events: broadcast::Sender<RemoteEvent>,
    shared: Arc<Mutex<Shared>>,
    stop: Arc<Notify>,
    api: SyncPlayApi,
    task: Option<JoinHandle<()>>,
}

impl RemoteHandle {
    /// Neuer Abonnent des Ereignis-Stroms (sieht nur künftige Ereignisse).
    pub fn subscribe(&self) -> broadcast::Receiver<RemoteEvent> {
        self.events.subscribe()
    }

    /// Aktueller Verbindungszustand.
    pub fn connection(&self) -> ConnectionState {
        self.shared.lock().unwrap().connection.clone()
    }

    /// Aktuelle SyncPlay-Gruppe.
    pub fn group(&self) -> Option<GroupInfo> {
        self.shared.lock().unwrap().group.clone()
    }

    /// REST-Zugriff auf SyncPlay (Gruppen auflisten, beitreten, verlassen, Befehle senden).
    /// Zustandsänderungen kommen danach als [`RemoteEvent::SyncPlay`] über den WebSocket.
    /// W5-remote: letzte gemessene Abweichung zur Gruppe (positiv = voraus), `None` ohne Messung oder Gruppe.
    pub fn drift_micros(&self) -> Option<i64> {
        self.shared.lock().unwrap().drift_micros
    }

    pub fn syncplay(&self) -> SyncPlayApi {
        self.api.clone()
    }

    /// Beendet die Verbindung und den Hintergrund-Task.
    pub async fn stop(mut self) {
        self.stop.notify_one();
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

impl Drop for RemoteHandle {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

/// Einstiegspunkt.
pub struct RemoteSession;

impl RemoteSession {
    /// Startet die Fernsteuerung im Hintergrund (benötigt eine laufende tokio-Laufzeit).
    pub fn start(cfg: RemoteConfig) -> RemoteHandle {
        let (events, _) = broadcast::channel(256);
        let shared = Arc::new(Mutex::new(Shared {
            connection: ConnectionState::Idle,
            group: None,
            drift_micros: None,
        }));
        let stop = Arc::new(Notify::new());
        let api = SyncPlayApi::new(cfg.client.clone());
        let runner = Runner {
            core: SyncCore::new(cfg.drift),
            cfg,
            events: events.clone(),
            shared: shared.clone(),
            stop: stop.clone(),
            scheduled: Vec::new(),
        };
        let task = tokio::spawn(runner.run());
        RemoteHandle {
            events,
            shared,
            stop,
            api,
            task: Some(task),
        }
    }
}

enum Internal {
    Sample(TimeSample),
    GroupGone,
}

enum Exit {
    Stop,
    Lost,
}

struct AbortOnDrop(JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

struct Runner {
    cfg: RemoteConfig,
    core: SyncCore,
    events: broadcast::Sender<RemoteEvent>,
    shared: Arc<Mutex<Shared>>,
    stop: Arc<Notify>,
    scheduled: Vec<Scheduled>,
}

impl Runner {
    fn emit(&self, event: RemoteEvent) {
        let _ = self.events.send(event);
    }

    fn set_connection(&self, state: ConnectionState) {
        self.shared.lock().unwrap().connection = state.clone();
        self.emit(RemoteEvent::Connection(state));
    }

    fn set_group(&self, group: Option<GroupInfo>) {
        let mut sh = self.shared.lock().unwrap();
        if group.is_none() {
            sh.drift_micros = None; // W5-remote
        }
        sh.group = group;
    }

    async fn run(mut self) {
        let mut failures = 0u32;
        loop {
            self.set_connection(ConnectionState::Connecting);
            let connected = tokio::select! {
                r = connect(&self.cfg.client, self.cfg.ws_auth) => r,
                () = self.stop.notified() => break,
            };
            let started = Instant::now();
            let reason = match connected {
                Ok(stream) => {
                    self.set_connection(ConnectionState::Connected);
                    match self.session_loop(stream).await {
                        Exit::Stop => break,
                        Exit::Lost => DisconnectReason::Lost,
                    }
                }
                Err(WsError::Rejected(_)) => {
                    self.set_connection(ConnectionState::Disconnected {
                        reason: DisconnectReason::Unauthorized,
                        retry_in: None,
                    });
                    return;
                }
                Err(e) => DisconnectReason::ConnectFailed(e.to_string()),
            };
            if started.elapsed() >= self.cfg.backoff.reset_after {
                failures = 0;
            }
            let delay = self.cfg.backoff.delay(failures);
            failures = failures.saturating_add(1);
            self.set_connection(ConnectionState::Disconnected {
                reason,
                retry_in: Some(delay),
            });
            tokio::select! {
                () = sleep(delay) => {}
                () = self.stop.notified() => break,
            }
        }
        self.set_connection(ConnectionState::Stopped);
    }

    fn spawn_background(&self, tx: mpsc::UnboundedSender<Internal>) -> Vec<AbortOnDrop> {
        let api = SyncPlayApi::new(self.cfg.client.clone());
        let caps = self.cfg.capabilities.clone();
        let caps_task = tokio::spawn({
            let api = api.clone();
            async move {
                if let Err(e) = api.report_capabilities(&caps).await {
                    tracing::warn!("Fähigkeiten melden fehlgeschlagen: {e}");
                }
            }
        });
        let policy = self.cfg.time_policy;
        let client = self.cfg.client.clone();
        let clock = self.cfg.clock.clone();
        let tx_time = tx.clone();
        let time_task = tokio::spawn(async move {
            let mut n = 0u32;
            loop {
                if let Ok(sample) = measure_once(&client, &*clock).await
                    && tx_time.send(Internal::Sample(sample)).is_err()
                {
                    return;
                }
                n = n.saturating_add(1);
                sleep(if n < policy.greedy_count {
                    policy.greedy_interval
                } else {
                    policy.steady_interval
                })
                .await;
            }
        });
        let mut tasks = vec![AbortOnDrop(caps_task), AbortOnDrop(time_task)];
        // Nach Wiederverbindung prüfen, ob die Gruppe noch existiert.
        if let Some(id) = self.core.group().map(|g| g.group_id.clone()) {
            tasks.push(AbortOnDrop(tokio::spawn(async move {
                if matches!(
                    api.group(&id).await,
                    Err(lfx_api::ApiError::NotFound | lfx_api::ApiError::Forbidden)
                ) {
                    let _ = tx.send(Internal::GroupGone);
                }
            })));
        }
        tasks
    }

    fn apply(&self, action: PlayerAction) {
        self.cfg.player.apply(action);
    }

    /// Führt fällige Aktionen aus (in Reihenfolge der Zeit, bei Gleichstand der Einfügung).
    fn run_due(&mut self, now: i64) {
        let mut due: Vec<Scheduled> = Vec::new();
        let mut rest = Vec::new();
        for s in std::mem::take(&mut self.scheduled) {
            if s.at_local_micros <= now {
                due.push(s)
            } else {
                rest.push(s)
            }
        }
        self.scheduled = rest;
        due.sort_by_key(|s| s.at_local_micros);
        for s in due {
            self.apply(s.action);
        }
    }

    fn queue(&mut self, items: Vec<Scheduled>) {
        self.scheduled.extend(items);
        let now = self.cfg.clock.now_micros();
        self.run_due(now);
    }

    fn handle_text(&mut self, text: &str, keepalive: &mut Duration) {
        let inbound = match parse_message(text) {
            Ok(i) => i,
            Err(e) => {
                tracing::warn!("Nachricht verworfen: {e}");
                self.emit(RemoteEvent::ParseError(e));
                return;
            }
        };
        let msg = inbound.message;
        let now = self.cfg.clock.now_micros();
        match &msg {
            // W5-remote: nur mit erlaubter Fernsteuerung
            ServerMessage::Play(r) if self.cfg.allow_remote_control => {
                self.apply(PlayerAction::from_play(r));
            }
            ServerMessage::Playstate(r) if self.cfg.allow_remote_control => {
                if let Some(a) = PlayerAction::from_playstate(r) {
                    self.apply(a);
                }
            }
            ServerMessage::GeneralCommand(g) if self.cfg.allow_remote_control => {
                self.apply(PlayerAction::from_general(g));
            }
            ServerMessage::ForceKeepAlive { seconds } => {
                *keepalive = Duration::from_secs(u64::from((*seconds).max(2)) / 2)
                    .max(Duration::from_secs(1));
            }
            ServerMessage::SyncPlayCommand(cmd) => {
                let snap = self.cfg.player.snapshot();
                let out = self.core.on_command(cmd, now, &snap);
                self.emit(RemoteEvent::SyncPlay(SyncPlayEvent::Command(cmd.clone())));
                self.queue(out);
            }
            ServerMessage::SyncPlayGroupUpdate(update) => self.handle_group_update(update, now),
            _ => {}
        }
        self.emit(RemoteEvent::Message(Arc::new(msg)));
    }

    fn handle_group_update(&mut self, update: &GroupUpdate, now: i64) {
        let out = self.core.on_group_update(update, now);
        self.set_group(self.core.group().cloned());
        let event = match update {
            GroupUpdate::GroupJoined { info, .. } => Some(SyncPlayEvent::Joined(info.clone())),
            GroupUpdate::GroupLeft { .. } => Some(SyncPlayEvent::Left),
            GroupUpdate::NotInGroup { .. } => Some(SyncPlayEvent::Left),
            GroupUpdate::GroupDoesNotExist { .. } => {
                Some(SyncPlayEvent::Denied("GroupDoesNotExist".into()))
            }
            GroupUpdate::LibraryAccessDenied { .. } => {
                Some(SyncPlayEvent::Denied("LibraryAccessDenied".into()))
            }
            GroupUpdate::StateUpdate { state, reason, .. } => Some(SyncPlayEvent::StateChanged {
                state: state.clone(),
                reason: reason.clone(),
            }),
            GroupUpdate::UserJoined { user, .. } => {
                Some(SyncPlayEvent::ParticipantJoined(user.clone()))
            }
            GroupUpdate::UserLeft { user, .. } => {
                Some(SyncPlayEvent::ParticipantLeft(user.clone()))
            }
            GroupUpdate::PlayQueue { update, .. } => Some(SyncPlayEvent::Queue(update.clone())),
            GroupUpdate::Unknown { .. } => None,
        };
        if let Some(e) = event {
            self.emit(RemoteEvent::SyncPlay(e));
        }
        self.queue(out);
    }

    fn tick(&mut self) {
        if !self.core.in_group() {
            return;
        }
        let now = self.cfg.clock.now_micros();
        let snap = self.cfg.player.snapshot();
        for a in self.core.tick(now, &snap) {
            self.apply(a);
        }
        self.shared.lock().unwrap().drift_micros = self.core.last_drift_micros(); // W5-remote
        if let Some(report) = self.core.poll_report(now, &snap) {
            let api = SyncPlayApi::new(self.cfg.client.clone());
            tokio::spawn(async move {
                let result = match &report {
                    SyncReport::Buffering(r) => api.buffering(r).await,
                    SyncReport::Ready(r) => api.ready(r).await,
                };
                if let Err(e) = result {
                    tracing::warn!("SyncPlay-Meldung fehlgeschlagen: {e}");
                }
            });
        }
    }

    async fn session_loop(&mut self, ws: WsStream) -> Exit {
        let (mut sink, mut stream) = ws.split();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let _tasks = self.spawn_background(tx);

        let mut keepalive = self.cfg.default_keepalive;
        let mut keep_timer = interval_at(Instant::now() + keepalive, keepalive);
        let mut tick_timer = interval_at(
            Instant::now() + self.cfg.tick_interval,
            self.cfg.tick_interval,
        );
        let mut last_rx = Instant::now();
        let stop = self.stop.clone();

        loop {
            let now = self.cfg.clock.now_micros();
            let delay = self
                .scheduled
                .iter()
                .map(|s| s.at_local_micros)
                .min()
                .map(|at| Duration::from_micros((at - now).max(0) as u64));
            let due_sleep = async {
                match delay {
                    Some(d) => sleep(d).await,
                    None => std::future::pending::<()>().await,
                }
            };
            let before = keepalive;
            tokio::select! {
                () = stop.notified() => {
                    let _ = sink.send(Message::Close(None)).await;
                    return Exit::Stop;
                }
                frame = stream.next() => match frame {
                    Some(Ok(Message::Text(text))) => {
                        last_rx = Instant::now();
                        self.handle_text(text.as_str(), &mut keepalive);
                    }
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return Exit::Lost,
                    Some(Ok(_)) => last_rx = Instant::now(),
                },
                _ = keep_timer.tick() => {
                    if last_rx.elapsed() > keepalive * 6 {
                        return Exit::Lost;
                    }
                    if sink.send(Message::Text(keep_alive_message().into())).await.is_err()
                        || sink.send(Message::Ping(Vec::new().into())).await.is_err()
                    {
                        return Exit::Lost;
                    }
                }
                _ = tick_timer.tick() => self.tick(),
                Some(internal) = rx.recv() => self.on_internal(internal),
                () = due_sleep => {
                    let now = self.cfg.clock.now_micros();
                    self.run_due(now);
                }
            }
            if keepalive != before {
                keep_timer = interval_at(Instant::now() + keepalive, keepalive);
            }
        }
    }

    fn on_internal(&mut self, internal: Internal) {
        match internal {
            Internal::Sample(s) => {
                self.core.time_mut().add(s);
                let (offset, ping) = (
                    self.core.time().offset_micros(),
                    self.core.time().ping_micros(),
                );
                self.emit(RemoteEvent::SyncPlay(SyncPlayEvent::TimeSynced {
                    offset_micros: offset,
                    ping_micros: ping,
                }));
                if self.core.in_group() {
                    let api = SyncPlayApi::new(self.cfg.client.clone());
                    let ms = ping / 1000;
                    tokio::spawn(async move {
                        let _ = api.ping(ms).await;
                    });
                }
            }
            Internal::GroupGone => {
                let now = self.cfg.clock.now_micros();
                let gone = GroupUpdate::NotInGroup {
                    group_id: String::new(),
                };
                self.handle_group_update(&gone, now);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_and_caps() {
        let b = Backoff::default();
        let d: Vec<u64> = (0..8).map(|i| b.delay(i).as_secs()).collect();
        assert_eq!(d, [1, 2, 4, 8, 16, 32, 60, 60]);
        assert_eq!(b.delay(1000).as_secs(), 60);
    }
}
