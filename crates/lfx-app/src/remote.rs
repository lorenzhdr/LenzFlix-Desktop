//! W5-remote: Fernsteuerung und SyncPlay in der App (Anbindung von `lfx-remote`).
//!
//! - Die Laufzeit (`RemoteSession`) lebt auf Tokio und ruft [`AppPlayer`] (`PlayerControl`) aus fremden Threads. Zeitkritisches
//!   (Pause, Fortsetzen, Springen, Tempo) geht direkt an den gemeldeten Player, alles mit App-Zustand per [`post`] in den UI-Thread.
//! - Der Zustand für Drift und Ready/Buffering (`PlayerSnapshot`) wird live aus dem Player gelesen, nie aus einer Kopie.
//! - Eigene Bedienung (Pause, Spulen) geht in einer SyncPlay-Gruppe an den Server statt an den Player (`remote_intent_*`);
//!   die Befehle des Servers kommen als angewendete Aktionen zurück und rufen diese Funktionen nie auf (keine Schleife).

use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use lfx_api::{BaseItem, ItemKind};
use lfx_nav::NavAction;
use lfx_player::{Player, TrackChoice, TrackKind};
use lfx_remote::{
    ConnectionState, GeneralCommandName as G, GroupInfo, GroupStateType, PlayCommand, PlayerAction,
    PlayerControl, PlayerSnapshot, RemoteConfig, RemoteEvent, RemoteHandle, RemoteSession,
    ServerMessage, SyncPlayApi, SyncPlayEvent, TypedCommand,
};
use lfx_ui::ToastKind;

use crate::app::{App, post};
use crate::play_osd::TogetherOp;
use crate::route::Route;

const TICKS: f64 = 10_000_000.0;

/// Der Player, auf den Fernbefehle gerade wirken (Video im Player oder Musik).
struct Slot {
    player: Player,
    item_id: Option<String>,
}

/// Zwischen UI-Thread und Laufzeit geteilt.
#[derive(Default)]
pub struct RemoteShared {
    slot: Mutex<Option<Slot>>,
}

impl RemoteShared {
    fn player(&self) -> Option<Player> {
        self.slot.lock().ok()?.as_ref().map(|s| s.player.clone())
    }

    /// Live-Zustand des Ziel-Players.
    pub fn snapshot(&self) -> PlayerSnapshot {
        let guard = self.slot.lock().ok();
        let Some(slot) = guard.as_deref().and_then(Option::as_ref) else {
            return PlayerSnapshot {
                rate: 1.0,
                ..PlayerSnapshot::default()
            };
        };
        let s = slot.player.state();
        PlayerSnapshot {
            item_id: slot.item_id.clone(),
            position_ticks: (s.position * TICKS) as i64,
            playing: s.loaded && !s.paused,
            buffering: !s.loaded || s.buffering,
            rate: if s.speed > 0.0 { s.speed } else { 1.0 },
        }
    }
}

/// `PlayerControl` der App.
pub struct AppPlayer(pub Arc<RemoteShared>);

impl PlayerControl for AppPlayer {
    fn apply(&self, action: PlayerAction) {
        match &action {
            PlayerAction::Pause => {
                if let Some(p) = self.0.player() {
                    p.pause();
                }
            }
            PlayerAction::Unpause => {
                if let Some(p) = self.0.player() {
                    p.play();
                }
            }
            PlayerAction::PlayPause => {
                if let Some(p) = self.0.player() {
                    p.toggle_pause();
                }
            }
            PlayerAction::SeekTicks(t) => {
                if let Some(p) = self.0.player() {
                    p.seek_to(*t as f64 / TICKS);
                }
            }
            PlayerAction::Rewind => {
                if let Some(p) = self.0.player() {
                    p.seek_by(-10.0);
                }
            }
            PlayerAction::FastForward => {
                if let Some(p) = self.0.player() {
                    p.seek_by(30.0);
                }
            }
            PlayerAction::SetRate(r) => {
                if let Some(p) = self.0.player() {
                    p.set_speed(*r);
                }
            }
            _ => {
                let a = action;
                post(move |app| app.remote_apply(a));
            }
        }
    }

    fn snapshot(&self) -> PlayerSnapshot {
        self.0.snapshot()
    }
}

/// Wartende Arbeit, sobald der neue Titel geladen ist.
struct Pending {
    generation: u64,
    seek_secs: Option<f64>,
    pause: bool,
}

/// Zustand der Fernsteuerung (nur UI-Thread).
#[derive(Default)]
pub struct RemoteCtl {
    handle: Option<RemoteHandle>,
    api: Option<SyncPlayApi>,
    shared: Arc<RemoteShared>,
    task: Option<tokio::task::AbortHandle>,
    timer: slint::Timer,
    pub allow: bool,
    conn: Option<ConnectionState>,
    /// Ergebnis der letzten Gruppenliste.
    pub groups: Vec<GroupInfo>,
    /// Weitere Videos, die nach dem laufenden kommen (Fernbefehl „Als Nächstes“ / „Ans Ende“).
    queue: VecDeque<BaseItem>,
    pending: Option<Pending>,
    /// Zuletzt ausgeführte Fernaktionen (nur Namen, für Tests).
    pub log: Vec<String>,
    last_refresh: Option<std::time::Instant>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tgt {
    Video,
    Audio,
    None,
}

fn action_name(a: &PlayerAction) -> &'static str {
    match a {
        PlayerAction::Play { .. } => "play",
        PlayerAction::Stop => "stop",
        PlayerAction::NextTrack => "next",
        PlayerAction::PreviousTrack => "previous",
        PlayerAction::SetVolume(_) => "volume",
        PlayerAction::SetMuted(_) | PlayerAction::ToggleMute => "mute",
        PlayerAction::SetAudioStreamIndex(_) => "audio-track",
        PlayerAction::SetSubtitleStreamIndex(_) => "subtitle-track",
        PlayerAction::ToggleFullscreen => "fullscreen",
        PlayerAction::GoHome => "home",
        PlayerAction::DisplayMessage { .. } => "message",
        PlayerAction::DisplayContent { .. } => "content",
        PlayerAction::General(_) => "general",
        _ => "other",
    }
}

impl App {
    // ---------------------------------------------------------------- Lebenszyklus

    /// Verbindet den Sitzungs-WebSocket (nach Anmeldung).
    pub fn remote_start(self: &Rc<Self>) {
        self.remote_stop();
        let Some(active) = self.session.active() else {
            return;
        };
        let allow = self
            .local_get(crate::account_logic::KEY_REMOTE)
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let shared = self.st.borrow().remote.shared.clone();
        let mut cfg = RemoteConfig::new(active.client.clone(), Arc::new(AppPlayer(shared)));
        cfg.allow_remote_control = allow;
        if !allow {
            cfg.capabilities.supports_media_control = false;
            cfg.capabilities.supported_commands.clear();
        }
        let handle = {
            let _guard = self.rt.enter();
            RemoteSession::start(cfg)
        };
        let mut rx = handle.subscribe();
        let task = self.rt.spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(ev) => post(move |a| a.remote_event(ev)),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(_) => break,
                }
            }
        });
        let api = handle.syncplay();
        {
            let mut st = self.st.borrow_mut();
            let r = &mut st.remote;
            r.allow = allow;
            r.api = Some(api);
            r.handle = Some(handle);
            r.task = Some(task.abort_handle());
            let weak = Rc::downgrade(self);
            r.timer.start(
                slint::TimerMode::Repeated,
                Duration::from_millis(100),
                move || {
                    if let Some(app) = weak.upgrade() {
                        app.remote_tick();
                    }
                },
            );
        }
        tracing::info!(allow, "Fernsteuerung gestartet");
    }

    /// Trennt die Verbindung (Abmelden, Beenden, Einstellung geändert).
    pub fn remote_stop(self: &Rc<Self>) {
        let (handle, task) = {
            let mut st = self.st.borrow_mut();
            let r = &mut st.remote;
            r.timer.stop();
            r.api = None;
            r.conn = None;
            r.groups.clear();
            r.queue.clear();
            r.pending = None;
            if let Ok(mut s) = r.shared.slot.lock() {
                *s = None;
            }
            (r.handle.take(), r.task.take())
        };
        if let Some(t) = task {
            t.abort();
        }
        drop(handle); // bricht die Laufzeit-Aufgabe ab
    }

    /// Einstellung „Fernsteuerung erlauben“ wurde geändert.
    pub fn remote_settings_changed(self: &Rc<Self>) {
        if self.st.borrow().remote.handle.is_some() {
            self.remote_start();
        }
    }

    // ---------------------------------------------------------------- Zeitgeber

    fn remote_target(&self) -> Tgt {
        let st = self.st.borrow();
        if st.play.opened && matches!(st.router.current(), Route::Player { .. }) {
            Tgt::Video
        } else if st.media.audio.active {
            Tgt::Audio
        } else {
            Tgt::None
        }
    }

    fn remote_tick(self: &Rc<Self>) {
        let tgt = self.remote_target();
        let slot = {
            let st = self.st.borrow();
            match tgt {
                Tgt::Video => st
                    .play
                    .player
                    .clone()
                    .map(|p| (p, st.play.item.as_ref().map(|i| i.id.clone()))),
                Tgt::Audio => st
                    .media
                    .audio
                    .player
                    .clone()
                    .filter(|_| st.media.audio.opened)
                    .map(|p| (p, st.media.audio.queue.current().map(|i| i.id.clone()))),
                Tgt::None => None,
            }
        };
        {
            let st = self.st.borrow();
            if let Ok(mut s) = st.remote.shared.slot.lock() {
                *s = slot.map(|(player, item_id)| Slot { player, item_id });
            }
        }
        self.remote_pending_tick();
    }

    /// Nach dem Laden: Startposition anfahren und bei Gruppenstart pausieren.
    fn remote_pending_tick(self: &Rc<Self>) {
        let todo = {
            let st = self.st.borrow();
            let Some(p) = &st.remote.pending else { return };
            if p.generation != st.play.generation {
                None
            } else {
                let loaded = st.play.player.as_ref().is_some_and(|pl| pl.state().loaded);
                if loaded {
                    Some((p.seek_secs, p.pause, st.play.player.clone()))
                } else {
                    return;
                }
            }
        };
        self.st.borrow_mut().remote.pending = None;
        if let Some((seek, pause, Some(player))) = todo {
            if pause {
                player.pause();
            }
            if let Some(s) = seek {
                player.seek_to(s);
            }
        }
    }

    // ---------------------------------------------------------------- Ereignisse

    fn remote_event(self: &Rc<Self>, ev: RemoteEvent) {
        match ev {
            RemoteEvent::Connection(c) => {
                tracing::info!("Fernsteuerung: Verbindungszustand geändert");
                self.st.borrow_mut().remote.conn = Some(c);
            }
            RemoteEvent::Message(m) => match &*m {
                ServerMessage::UserDataChanged(_) | ServerMessage::LibraryChanged(_) => {
                    self.remote_data_changed();
                }
                _ => {}
            },
            RemoteEvent::SyncPlay(e) => match e {
                SyncPlayEvent::Joined(g) => {
                    self.toast(
                        &format!("Gruppe „{}“ beigetreten", g.group_name),
                        ToastKind::Success,
                    );
                }
                SyncPlayEvent::Left => self.toast("Gruppe verlassen", ToastKind::Info),
                SyncPlayEvent::Denied(_) => {
                    self.toast("Die Gruppe ist nicht verfügbar", ToastKind::Error);
                }
                SyncPlayEvent::ParticipantJoined(u) => {
                    self.toast(&format!("{u} ist der Gruppe beigetreten"), ToastKind::Info);
                }
                SyncPlayEvent::ParticipantLeft(u) => {
                    self.toast(&format!("{u} hat die Gruppe verlassen"), ToastKind::Info);
                }
                _ => {}
            },
            RemoteEvent::ParseError(_) => {}
        }
        self.remote_refresh_menu();
    }

    /// Benutzerdaten oder Bibliothek haben sich geändert: Startseite auffrischen (höchstens alle 5 s, nie im Player).
    fn remote_data_changed(self: &Rc<Self>) {
        if !matches!(self.st.borrow().router.current(), Route::Home) {
            return; // beim nächsten Betreten wird ohnehin neu geladen
        }
        {
            let mut st = self.st.borrow_mut();
            if st
                .remote
                .last_refresh
                .is_some_and(|t| t.elapsed() < Duration::from_secs(5))
            {
                return;
            }
            st.remote.last_refresh = Some(std::time::Instant::now());
        }
        self.load_home();
    }

    // ---------------------------------------------------------------- Aktionen

    /// Setzt eine Aktion aus der Ferne um (UI-Thread).
    pub fn remote_apply(self: &Rc<Self>, action: PlayerAction) {
        self.st
            .borrow_mut()
            .remote
            .log
            .push(action_name(&action).to_owned());
        let tgt = self.remote_target();
        match action {
            PlayerAction::Play {
                item_ids,
                start_index,
                start_position_ticks,
                command,
                media_source_id,
                audio_stream_index,
                subtitle_stream_index,
            } => self.remote_play(
                item_ids,
                start_index,
                start_position_ticks,
                command,
                (media_source_id, audio_stream_index, subtitle_stream_index),
            ),
            PlayerAction::Stop => match tgt {
                Tgt::Video => self.leave_player(),
                Tgt::Audio => self.audio_stop(),
                Tgt::None => {}
            },
            PlayerAction::NextTrack => match tgt {
                Tgt::Video => {
                    if !self.remote_queue_next() {
                        self.play_next();
                    }
                }
                Tgt::Audio => self.audio_next(false),
                Tgt::None => {}
            },
            PlayerAction::PreviousTrack => match tgt {
                Tgt::Video => {
                    if let Some(p) = self.st.borrow().play.player.clone() {
                        p.seek_to(0.0);
                    }
                }
                Tgt::Audio => self.audio_prev(),
                Tgt::None => {}
            },
            PlayerAction::SetVolume(v) => {
                for p in self.remote_players() {
                    p.set_volume(f64::from(v.min(100)));
                    if v > 0 {
                        p.set_muted(false);
                    }
                }
            }
            PlayerAction::SetMuted(m) => {
                for p in self.remote_players() {
                    p.set_muted(m);
                }
            }
            PlayerAction::ToggleMute => {
                for p in self.remote_players() {
                    p.set_muted(!p.state().muted);
                }
            }
            PlayerAction::SetAudioStreamIndex(i) => self.remote_set_stream(TrackKind::Audio, i),
            PlayerAction::SetSubtitleStreamIndex(i) => {
                self.remote_set_stream(TrackKind::Subtitle, i);
            }
            PlayerAction::ToggleFullscreen => self.toggle_fullscreen(),
            PlayerAction::GoHome => {
                self.remote_leave_video();
                if self.session.active().is_some() {
                    self.go_reset(Route::Home);
                }
            }
            PlayerAction::DisplayMessage { header, text, .. } => {
                let shown = match header.filter(|h| !h.is_empty()) {
                    Some(h) => format!("{h}: {text}"),
                    None => text,
                };
                self.toast(&shown, ToastKind::Info);
            }
            PlayerAction::DisplayContent { item_id, .. } => {
                if self.session.active().is_some() {
                    self.remote_leave_video();
                    self.go(Route::Detail { id: item_id });
                }
            }
            PlayerAction::General(g) => {
                if let TypedCommand::Other { name, .. } = g.typed() {
                    self.remote_general(&name);
                }
            }
            // direkt behandelt (siehe `AppPlayer::apply`)
            _ => {}
        }
    }

    fn remote_leave_video(self: &Rc<Self>) {
        if self.remote_target() == Tgt::Video {
            self.leave_player();
        }
    }

    /// Alle Player, die Lautstärkebefehle erhalten.
    fn remote_players(&self) -> Vec<Player> {
        let st = self.st.borrow();
        let mut v = Vec::new();
        if let Some(p) = st.play.player.clone() {
            v.push(p);
        }
        if st.media.audio.active
            && let Some(p) = st.media.audio.player.clone()
        {
            v.push(p);
        }
        v
    }

    fn remote_general(self: &Rc<Self>, name: &G) {
        let nav = |a: NavAction| self.apply_action(a);
        match name {
            G::MoveUp => nav(NavAction::Up),
            G::MoveDown => nav(NavAction::Down),
            G::MoveLeft => nav(NavAction::Left),
            G::MoveRight => nav(NavAction::Right),
            G::PageUp => nav(NavAction::PageUp),
            G::PageDown => nav(NavAction::PageDown),
            G::Select => nav(NavAction::Activate),
            G::Back => nav(NavAction::Back),
            G::ToggleContextMenu | G::ToggleOsdMenu => nav(NavAction::Menu),
            G::ToggleOsd => self.player_activity(),
            G::GoToSearch => {
                if self.session.active().is_some() {
                    self.remote_leave_video();
                    self.go(Route::Search);
                }
            }
            G::GoToSettings => {
                if self.session.active().is_some() {
                    self.remote_leave_video();
                    self.go(Route::Settings);
                }
            }
            G::VolumeUp | G::VolumeDown => {
                let d = if *name == G::VolumeUp { 5.0 } else { -5.0 };
                for p in self.remote_players() {
                    p.set_volume((p.state().volume + d).clamp(0.0, 100.0));
                    p.set_muted(false);
                }
            }
            _ => {}
        }
    }

    // ---------------------------------------------------------------- Wiedergabe aus der Ferne

    fn remote_play(
        self: &Rc<Self>,
        ids: Vec<String>,
        start_index: Option<i32>,
        start_ticks: Option<i64>,
        command: PlayCommand,
        choice: (Option<String>, Option<i32>, Option<i32>),
    ) {
        let Some(active) = self.session.active() else {
            return;
        };
        if ids.is_empty() {
            return;
        }
        let client = active.client.clone();
        self.rt.spawn(async move {
            let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
            let found = client
                .items(&lfx_core::rows::logic::base_query().ids(&refs))
                .await;
            let items: Vec<BaseItem> = match found {
                Ok(q) => ids
                    .iter()
                    .filter_map(|id| q.items.iter().find(|i| &i.id == id).cloned())
                    .collect(),
                Err(e) => {
                    tracing::warn!("Fernwiedergabe: Titel nicht ladbar: {e}");
                    return;
                }
            };
            post(move |app| {
                app.remote_play_items(items, start_index, start_ticks, command, choice);
            });
        });
    }

    fn remote_play_items(
        self: &Rc<Self>,
        items: Vec<BaseItem>,
        start_index: Option<i32>,
        start_ticks: Option<i64>,
        command: PlayCommand,
        (source, audio, sub): (Option<String>, Option<i32>, Option<i32>),
    ) {
        if items.is_empty() {
            return;
        }
        let idx = start_index.unwrap_or(0).clamp(0, items.len() as i32 - 1) as usize;
        let is_audio = matches!(items[idx].kind, ItemKind::Audio | ItemKind::AudioBook);
        if is_audio {
            match command {
                PlayCommand::PlayNext => {
                    self.audio_play_next(items);
                    self.toast("Als Nächstes eingereiht", ToastKind::Info);
                }
                PlayCommand::PlayLast => {
                    self.audio_enqueue(items);
                    self.toast("Zur Warteschlange hinzugefügt", ToastKind::Info);
                }
                _ => self.audio_play(items, idx, None),
            }
            return;
        }
        let busy = self.remote_target() == Tgt::Video;
        match (&command, busy) {
            (PlayCommand::PlayNext, true) => {
                let mut st = self.st.borrow_mut();
                for it in items.into_iter().rev() {
                    st.remote.queue.push_front(it);
                }
                drop(st);
                self.toast("Als Nächstes eingereiht", ToastKind::Info);
                return;
            }
            (PlayCommand::PlayLast, true) => {
                self.st.borrow_mut().remote.queue.extend(items);
                self.toast("Zur Warteschlange hinzugefügt", ToastKind::Info);
                return;
            }
            _ => {}
        }
        let item = items[idx].clone();
        {
            let mut st = self.st.borrow_mut();
            st.remote.queue = items.into_iter().skip(idx + 1).collect();
            st.detail.choice = Some(crate::detail::PlayChoice {
                from_start: false,
                source,
                audio,
                sub,
            });
        }
        let in_group = self
            .st
            .borrow()
            .remote
            .handle
            .as_ref()
            .is_some_and(|h| h.group().is_some());
        self.remote_start_video(&item);
        let mut st = self.st.borrow_mut();
        let generation = st.play.generation;
        st.remote.pending = Some(Pending {
            generation,
            seek_secs: start_ticks.filter(|t| *t > 0).map(|t| t as f64 / TICKS),
            pause: in_group,
        });
    }

    fn remote_start_video(self: &Rc<Self>, item: &BaseItem) {
        if self.remote_target() == Tgt::Video {
            self.stop_playback_keep_route();
            self.start_item(item, crate::play::ResumeMode::Restart, true);
        } else {
            self.play_item_with(item, crate::play::ResumeMode::Restart);
        }
    }

    /// Nächstes eingereihtes Video starten (`true` = gestartet). Wird auch am Ende eines Titels gefragt.
    pub fn remote_queue_next(self: &Rc<Self>) -> bool {
        let next = self.st.borrow_mut().remote.queue.pop_front();
        match next {
            Some(item) => {
                self.remote_start_video(&item);
                true
            }
            None => false,
        }
    }

    /// Spurwahl des Servers (Stream-Index der Quelle) auf mpv-Spuren abbilden: n-te Spur der Art.
    fn remote_set_stream(self: &Rc<Self>, kind: TrackKind, index: i32) {
        let Some(player) = self.st.borrow().play.player.clone() else {
            return;
        };
        let setter = move |c: TrackChoice| match kind {
            TrackKind::Audio => player.set_audio_track(c),
            _ => player.set_subtitle_track(c),
        };
        if index < 0 {
            if kind == TrackKind::Subtitle {
                setter(TrackChoice::Off);
            }
            return;
        }
        let (Some(active), Some(item_id), source) = (
            self.session.active(),
            self.st.borrow().play.item.as_ref().map(|i| i.id.clone()),
            self.st
                .borrow()
                .play
                .request
                .as_ref()
                .map(|r| r.media_source_id.clone()),
        ) else {
            return;
        };
        let client = active.client.clone();
        let want = if kind == TrackKind::Audio {
            "Audio"
        } else {
            "Subtitle"
        };
        self.rt.spawn(async move {
            let Ok(full) = client.item(&item_id, &["MediaSources"]).await else {
                return;
            };
            let src = source
                .and_then(|s| full.media_sources.iter().find(|m| m.id == s))
                .or_else(|| full.media_sources.first());
            let Some(src) = src else { return };
            let ordinal = src
                .media_streams
                .iter()
                .filter(|s| s.kind == want)
                .position(|s| s.index == index);
            let Some(n) = ordinal else { return };
            post(move |app| {
                let Some(p) = app.st.borrow().play.player.clone() else {
                    return;
                };
                let tracks = p.state().tracks;
                if let Some(t) = tracks.iter().filter(|t| t.kind == kind).nth(n) {
                    if kind == TrackKind::Audio {
                        p.set_audio_track(TrackChoice::Id(t.id));
                    } else {
                        p.set_subtitle_track(TrackChoice::Id(t.id));
                    }
                }
            });
        });
    }

    // ---------------------------------------------------------------- Gemeinsam schauen: eigene Bedienung

    fn remote_in_group(&self) -> bool {
        self.st
            .borrow()
            .remote
            .handle
            .as_ref()
            .is_some_and(|h| h.group().is_some())
    }

    fn remote_send(self: &Rc<Self>, f: impl FnOnce(SyncPlayApi) -> RemoteFut + 'static) {
        let Some(api) = self.st.borrow().remote.api.clone() else {
            return;
        };
        let fut = f(api);
        self.rt.spawn(async move {
            if let Err(e) = fut.await {
                tracing::warn!("SyncPlay-Anfrage fehlgeschlagen: {e}");
            }
        });
    }

    /// Eigene Pause/Fortsetzen-Bedienung in einer Gruppe. `true` = an den Server gegeben (Player bleibt unberührt).
    pub fn remote_intent_pause(self: &Rc<Self>, want_pause: Option<bool>) -> bool {
        if !self.remote_in_group() || self.remote_target() != Tgt::Video {
            return false;
        }
        let paused = self
            .st
            .borrow()
            .play
            .player
            .as_ref()
            .is_some_and(|p| p.state().paused);
        let pause = want_pause.unwrap_or(!paused);
        if pause {
            self.remote_send(|a| Box::pin(async move { a.pause().await }));
        } else {
            self.remote_send(|a| Box::pin(async move { a.unpause().await }));
        }
        true
    }

    /// Eigenes Spulen in einer Gruppe (`secs` = Sprungziel in Sekunden).
    pub fn remote_intent_seek(self: &Rc<Self>, secs: f64) -> bool {
        if !self.remote_in_group() || self.remote_target() != Tgt::Video {
            return false;
        }
        let ticks = (secs.max(0.0) * TICKS) as i64;
        self.remote_send(move |a| Box::pin(async move { a.seek(ticks).await }));
        true
    }

    /// Eigenes Spulen relativ zur Position.
    pub fn remote_intent_seek_by(self: &Rc<Self>, delta: f64) -> bool {
        let pos = self
            .st
            .borrow()
            .play
            .player
            .as_ref()
            .map_or(0.0, |p| p.state().position);
        self.remote_intent_seek(pos + delta)
    }

    /// Tasten und Medientasten: `true` = übernommen.
    pub fn remote_intent_action(self: &Rc<Self>, action: NavAction) -> bool {
        match action {
            NavAction::PlayPause => self.remote_intent_pause(None),
            NavAction::Play => self.remote_intent_pause(Some(false)),
            NavAction::Pause => self.remote_intent_pause(Some(true)),
            NavAction::SeekForward => self.remote_intent_seek_by(10.0),
            NavAction::SeekBackward => self.remote_intent_seek_by(-10.0),
            _ => false,
        }
    }

    // ---------------------------------------------------------------- Menü „Gemeinsam schauen“

    /// Statuszeile für die Anzeige im Player (leer ohne Gruppe).
    pub fn remote_sync_text(&self) -> String {
        let st = self.st.borrow();
        let Some(h) = st.remote.handle.as_ref() else {
            return String::new();
        };
        let Some(g) = h.group() else {
            return String::new();
        };
        let snap = st.remote.shared.snapshot();
        if snap.item_id.is_some() && snap.buffering {
            "Gemeinsam: puffert".to_owned()
        } else if g.state == Some(GroupStateType::Waiting) {
            "Gemeinsam: wartet auf andere".to_owned()
        } else {
            match h.drift_micros() {
                Some(d) if d.abs() < 150_000 => "Gemeinsam: synchron".to_owned(),
                Some(d) => format!(
                    "Gemeinsam: Abweichung {:.1} s",
                    d.unsigned_abs() as f64 / 1e6
                )
                .replace('.', ","),
                None => "Gemeinsam: verbunden".to_owned(),
            }
        }
    }

    /// Einträge des Menüs.
    pub fn remote_menu_items(&self) -> Vec<crate::play_osd::MenuItem> {
        use crate::play_osd::{MenuAction, MenuItem};
        let status = self.remote_sync_text();
        let st = self.st.borrow();
        let r = &st.remote;
        let mk = |label: String, detail: String, selected: bool, op: Option<TogetherOp>| MenuItem {
            label,
            detail,
            selected,
            action: op.map_or(MenuAction::None, MenuAction::Together),
        };
        let mut v = Vec::new();
        if !matches!(r.conn, Some(ConnectionState::Connected)) {
            v.push(mk(
                "Nicht mit dem Server verbunden".into(),
                String::new(),
                false,
                None,
            ));
            return v;
        }
        match r.handle.as_ref().and_then(RemoteHandle::group) {
            Some(g) => {
                v.push(mk(
                    format!("Gruppe: {}", g.group_name),
                    g.participants.join(", "),
                    true,
                    None,
                ));
                v.push(mk(status, String::new(), false, None));
                v.push(mk(
                    "Gruppe verlassen".into(),
                    String::new(),
                    false,
                    Some(TogetherOp::Leave),
                ));
            }
            None => {
                v.push(mk(
                    "Neue Gruppe starten".into(),
                    "Mit dem aktuellen Titel".into(),
                    false,
                    Some(TogetherOp::New),
                ));
                for (i, g) in r.groups.iter().enumerate() {
                    v.push(mk(
                        g.group_name.clone(),
                        format!("{} Teilnehmer", g.participants.len()),
                        false,
                        Some(TogetherOp::Join(i)),
                    ));
                }
                v.push(mk(
                    "Liste aktualisieren".into(),
                    String::new(),
                    false,
                    Some(TogetherOp::Refresh),
                ));
            }
        }
        v
    }

    /// Gruppenliste laden (beim Öffnen des Menüs).
    pub fn remote_fetch_groups(self: &Rc<Self>) {
        let Some(api) = self.st.borrow().remote.api.clone() else {
            return;
        };
        self.rt.spawn(async move {
            if let Ok(groups) = api.list_groups().await {
                post(move |app| {
                    app.st.borrow_mut().remote.groups = groups;
                    app.remote_refresh_menu();
                });
            }
        });
    }

    /// Ist das Gruppen-Menü offen, werden die Einträge neu aufgebaut.
    fn remote_refresh_menu(self: &Rc<Self>) {
        let open = self
            .st
            .borrow()
            .play
            .menu
            .as_ref()
            .is_some_and(|m| m.kind == crate::play_osd::MenuKind::Together);
        if open {
            let items = self.remote_menu_items();
            if let Some(m) = self.st.borrow_mut().play.menu.as_mut() {
                m.items = items;
                m.focus = m.focus.min(m.items.len().saturating_sub(1));
            }
            self.refresh_view();
        }
    }

    /// Eintrag des Menüs gewählt. `true` = Menü bleibt offen.
    pub fn remote_together_op(self: &Rc<Self>, op: TogetherOp) -> bool {
        match op {
            TogetherOp::Refresh => {
                self.remote_fetch_groups();
                true
            }
            TogetherOp::Leave => {
                self.remote_send(|a| Box::pin(async move { a.leave().await }));
                false
            }
            TogetherOp::Join(i) => {
                let id = self
                    .st
                    .borrow()
                    .remote
                    .groups
                    .get(i)
                    .map(|g| g.group_id.clone());
                if let Some(id) = id {
                    self.remote_send(move |a| Box::pin(async move { a.join(&id).await }));
                }
                false
            }
            TogetherOp::New => {
                let name = format!(
                    "Gruppe von {}",
                    self.session
                        .active()
                        .map_or_else(String::new, |a| a.user_name)
                );
                let (item, pos) = {
                    let st = self.st.borrow();
                    (
                        st.play.item.as_ref().map(|i| i.id.clone()),
                        st.play.player.as_ref().map_or(0.0, |p| p.state().position),
                    )
                };
                self.remote_send(move |a| {
                    Box::pin(async move {
                        a.new_group(&name).await?;
                        if let Some(id) = item {
                            a.set_new_queue(&[id], 0, (pos * TICKS) as i64).await?;
                        }
                        Ok(())
                    })
                });
                false
            }
        }
    }

    /// Zustand für Tests.
    pub fn remote_state_json(&self) -> serde_json::Value {
        let st = self.st.borrow();
        let r = &st.remote;
        let group = r.handle.as_ref().and_then(RemoteHandle::group);
        serde_json::json!({
            "started": r.handle.is_some(),
            "allow": r.allow,
            "connected": matches!(r.conn, Some(ConnectionState::Connected)),
            "in_group": group.is_some(),
            "group": group.as_ref().map(|g| g.group_name.clone()),
            "participants": group.as_ref().map(|g| g.participants.clone()),
            "groups": r.groups.iter().map(|g| g.group_name.clone()).collect::<Vec<_>>(),
            "queue": r.queue.len(),
            "log": r.log,
            "drift_ms": r.handle.as_ref().and_then(RemoteHandle::drift_micros).map(|d| d / 1000),
            "toast": self.win().get_toast().to_string(),
        })
    }
}

type RemoteFut =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), lfx_api::ApiError>> + Send>>;

#[cfg(feature = "dev-automation")]
impl App {
    /// Entwickler-Befehle `remote_*`: `remote_refresh`, `remote_join` (`index`), `remote_new`, `remote_leave`, `remote_menu`.
    pub fn remote_automation(
        self: &Rc<Self>,
        cmd: &str,
        msg: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        match cmd {
            "remote_refresh" => self.remote_fetch_groups(),
            "remote_join" => {
                let i = msg["index"].as_u64().unwrap_or(0) as usize;
                self.remote_together_op(TogetherOp::Join(i));
            }
            "remote_new" => {
                self.remote_together_op(TogetherOp::New);
            }
            "remote_leave" => {
                self.remote_together_op(TogetherOp::Leave);
            }
            "remote_allow" => {
                let v = msg["value"].as_bool().unwrap_or(true);
                self.save_setting(crate::account_logic::KEY_REMOTE, serde_json::Value::Bool(v));
                self.remote_settings_changed();
            }
            "remote_state" => {}
            _ => return Err("unbekannter remote-Befehl".into()),
        }
        Ok(serde_json::json!({}))
    }
}
