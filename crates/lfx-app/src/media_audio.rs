//! W4-media: Audio-Player der Medienseiten. Ein eigener `lfx-player`-Player ohne Bild (`vo=null`) lebt im
//! App-Zustand und überdauert Seitenwechsel; die Mini-Leiste unten zeigt ihn über allen Seiten.
//!
//! Video (Player-Seite) und TV-Modus stoppen die Musik (`media_on_route`, `Act::PlayItem`). Trailer der
//! Startseite sind stumm startbar und laufen unabhängig; beim Start eines Musiktitels endet der Trailer.

use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use lfx_api::{AudioStreamOptions, BaseItem, ItemKind, PlayMethod};
use lfx_core::images::ImageRef;
use lfx_images::Priority;
use lfx_nav::NavAction;
use lfx_platform::{MediaCommand, NowPlaying, PlaybackStatus};
use lfx_player::{ApiSink, PlayRequest, Player, PlayerConfig, ProgressReporter, ReporterConfig};
use lfx_ui::{MediaTrack, ToastKind};
use slint::{Image, ModelRc, Timer, TimerMode, VecModel};

use crate::app::{App, current, post};
use crate::media::{Act, Z_BAR, Z_BODY, Z_FULL, Z_QUEUE, cover_pick, duration_text};
use crate::media_model::{AudioQueue, Repeat};

/// Zustand des Audio-Players.
pub struct AudioCtl {
    pub queue: AudioQueue,
    pub player: Option<Player>,
    pub reporter: Option<ProgressReporter>,
    pub active: bool,
    /// Der aktuelle Titel wurde geöffnet und sein Ende noch nicht behandelt.
    pub opened: bool,
    /// Titel war mindestens einmal geladen (schützt vor einem Rest-Ende des Vorgängers).
    pub seen_loaded: bool,
    pub serial: u64,
    pub full: bool,
    pub saved_focus: (i32, usize),
    pub timer: Timer,
    pub status: Option<PlaybackStatus>,
    pub meta_sent: bool,
    pub timeline: Option<Instant>,
    pub paused: bool,
    pub position: f64,
    pub duration: f64,
}

impl Default for AudioCtl {
    fn default() -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1, |d| d.as_nanos() as u64);
        Self {
            queue: AudioQueue::new(seed),
            player: None,
            reporter: None,
            active: false,
            opened: false,
            seen_loaded: false,
            serial: 0,
            full: false,
            saved_focus: (Z_BODY, 0),
            timer: Timer::default(),
            status: None,
            meta_sent: false,
            timeline: None,
            paused: false,
            position: 0.0,
            duration: 0.0,
        }
    }
}

fn artist_line(i: &BaseItem) -> String {
    if !i.artists.is_empty() {
        i.artists.join(", ")
    } else {
        i.album_artist.clone().unwrap_or_default()
    }
}

fn clock(secs: f64) -> String {
    crate::play::format_time(secs)
}

impl App {
    pub fn audio_active(&self) -> bool {
        self.st.borrow().media.audio.active
    }

    pub fn audio_current_id(&self) -> Option<String> {
        let st = self.st.borrow();
        let a = &st.media.audio;
        if a.active {
            a.queue.current().map(|i| i.id.clone())
        } else {
            None
        }
    }

    fn ensure_audio_player(self: &Rc<Self>) -> Option<Player> {
        if let Some(p) = self.st.borrow().media.audio.player.clone() {
            return Some(p);
        }
        let mut cfg = PlayerConfig::headless();
        // Tests bleiben stumm: LFX_AUDIO_NULL=1 schaltet die Tonausgabe ab, sonst läuft der Ton normal.
        cfg.audio_null = std::env::var_os("LFX_AUDIO_NULL").is_some();
        match Player::new(cfg) {
            Ok(p) => {
                self.st.borrow_mut().media.audio.player = Some(p.clone());
                Some(p)
            }
            Err(e) => {
                tracing::warn!("Audio-Player nicht startbar: {}", e.text);
                self.toast(
                    "Der Audio-Player konnte nicht gestartet werden.",
                    ToastKind::Error,
                );
                None
            }
        }
    }

    // ------------------------------------------------------------ Steuerung

    pub fn audio_play(self: &Rc<Self>, items: Vec<BaseItem>, start: usize, shuffle: Option<bool>) {
        let items: Vec<BaseItem> = items
            .into_iter()
            .filter(|i| i.kind == ItemKind::Audio)
            .collect();
        if items.is_empty() {
            return;
        }
        self.stop_trailer();
        {
            let mut st = self.st.borrow_mut();
            let a = &mut st.media.audio;
            if shuffle == Some(true) {
                a.queue.shuffle = true;
            }
            a.queue.set(items, start);
        }
        self.audio_open_current();
    }

    pub fn audio_play_next(self: &Rc<Self>, items: Vec<BaseItem>) {
        if !self.audio_active() {
            self.audio_play(items, 0, None);
            return;
        }
        self.st.borrow_mut().media.audio.queue.play_next(items);
        self.audio_sync_ui();
        self.toast("Wird als Nächstes gespielt.", ToastKind::Success);
    }

    pub fn audio_enqueue(self: &Rc<Self>, items: Vec<BaseItem>) {
        if !self.audio_active() {
            self.audio_play(items, 0, None);
            return;
        }
        self.st.borrow_mut().media.audio.queue.enqueue(items);
        self.audio_sync_ui();
        self.toast("An die Warteschlange angehängt.", ToastKind::Success);
    }

    pub fn audio_mix(self: &Rc<Self>, id: &str) {
        let Some(client) = self.media_client() else {
            return;
        };
        let id = id.to_owned();
        self.rt.spawn(async move {
            let items = client
                .instant_mix(&id, 50)
                .await
                .map(|r| r.items)
                .unwrap_or_default();
            post(move |app| {
                if items.is_empty() {
                    app.toast("Kein Mix verfügbar.", ToastKind::Info);
                } else {
                    app.audio_play(items, 0, None);
                }
            });
        });
    }

    fn audio_open_current(self: &Rc<Self>) {
        let Some(item) = self.st.borrow().media.audio.queue.current().cloned() else {
            self.audio_stop();
            return;
        };
        let Some(client) = self.media_client() else {
            return;
        };
        let Some(player) = self.ensure_audio_player() else {
            return;
        };
        let url = match client.audio_stream_url(&item.id, &AudioStreamOptions::default()) {
            Ok(u) => u,
            Err(e) => {
                tracing::warn!("Audio-Adresse: {e}");
                return;
            }
        };
        let source = item
            .media_sources
            .first()
            .map_or_else(|| item.id.clone(), |s| s.id.clone());
        let mut req = PlayRequest::new(item.id.clone(), source, url);
        req.http_headers = vec![format!("Authorization: {}", client.auth_header_value())];
        req.play_method = PlayMethod::DirectPlay;
        let interval = self.opts.report_interval.unwrap_or(Duration::from_secs(10));
        let reporter = {
            let _g = self.rt.enter();
            ProgressReporter::spawn(
                &player,
                req.clone(),
                Arc::new(ApiSink::new(client.clone())),
                ReporterConfig {
                    report_progress: true,
                    interval,
                },
            )
        };
        let old = {
            let mut st = self.st.borrow_mut();
            let a = &mut st.media.audio;
            a.serial += 1;
            a.active = true;
            a.opened = true;
            a.seen_loaded = false;
            a.meta_sent = false;
            a.position = 0.0;
            a.duration = item.run_time_ticks.unwrap_or(0) as f64 / 10_000_000.0;
            a.reporter.replace(reporter)
        };
        if let Some(r) = old {
            // „Stopped“ des Vorgängers geht im Hintergrund hinaus
            self.rt.spawn(async move { r.join().await });
        }
        player.open(req);
        self.audio_start_timer();
        self.audio_sync_ui();
        self.audio_load_cover(&item);
        self.audio_smtc_meta(&item);
        self.media_refresh_playing();
    }

    fn audio_start_timer(self: &Rc<Self>) {
        let st = self.st.borrow();
        st.media
            .audio
            .timer
            .start(TimerMode::Repeated, Duration::from_millis(250), || {
                if let Some(app) = current() {
                    app.audio_tick();
                }
            });
    }

    fn audio_load_cover(self: &Rc<Self>, item: &BaseItem) {
        let serial = self.st.borrow().media.audio.serial;
        self.with_media(|g| g.set_np_has_cover(false));
        if let Some(pick) = cover_pick(item) {
            self.fetch_image(
                &ImageRef {
                    pick,
                    max_width: 640,
                    quality: 85,
                },
                Priority::Visible,
                move |app, img: Image| {
                    if app.st.borrow().media.audio.serial != serial {
                        return;
                    }
                    app.with_media(|g| {
                        g.set_np_cover(img);
                        g.set_np_has_cover(true);
                    });
                },
            );
        }
    }

    pub fn audio_toggle(self: &Rc<Self>) {
        let (player, opened) = {
            let st = self.st.borrow();
            (st.media.audio.player.clone(), st.media.audio.opened)
        };
        let Some(p) = player else { return };
        if opened {
            p.toggle_pause();
        } else {
            self.audio_open_current();
        }
    }

    pub fn audio_next(self: &Rc<Self>, auto: bool) {
        let n = self.st.borrow_mut().media.audio.queue.next(auto);
        if n.is_some() {
            self.audio_open_current();
        } else {
            self.audio_stop();
        }
    }

    pub fn audio_prev(self: &Rc<Self>) {
        let (pos, player) = {
            let st = self.st.borrow();
            (st.media.audio.position, st.media.audio.player.clone())
        };
        if pos > 3.0 {
            if let Some(p) = player {
                p.seek_to(0.0);
            }
            return;
        }
        let before = self.st.borrow().media.audio.queue.current_index();
        let now = self.st.borrow_mut().media.audio.queue.previous();
        if now == before {
            if let Some(p) = player {
                p.seek_to(0.0);
            }
        } else {
            self.audio_open_current();
        }
    }

    pub fn audio_jump(self: &Rc<Self>, queue_pos: usize) {
        let idx = {
            let st = self.st.borrow();
            st.media
                .audio
                .queue
                .upcoming()
                .get(queue_pos)
                .map(|(i, _)| *i)
        };
        if let Some(i) = idx {
            self.st.borrow_mut().media.audio.queue.jump_to(i);
            self.audio_open_current();
        }
    }

    pub fn audio_remove(self: &Rc<Self>, item_index: usize) {
        let was_current = {
            let mut st = self.st.borrow_mut();
            let a = &mut st.media.audio;
            let cur = a.queue.current_index() == Some(item_index);
            a.queue.remove(item_index);
            cur
        };
        if self.st.borrow().media.audio.queue.is_empty() {
            self.audio_stop();
        } else if was_current {
            self.audio_open_current();
        } else {
            self.audio_sync_ui();
        }
    }

    pub fn audio_seek_fraction(self: &Rc<Self>, v: f32) {
        let (p, d) = {
            let st = self.st.borrow();
            (st.media.audio.player.clone(), st.media.audio.duration)
        };
        if let (Some(p), true) = (p, d > 0.0) {
            p.seek_to(f64::from(v.clamp(0.0, 1.0)) * d);
        }
    }

    /// Beendet die Musik und blendet die Leiste aus (tut nichts, wenn keine Musik läuft).
    pub fn audio_stop(self: &Rc<Self>) {
        if !self.audio_active() {
            return;
        }
        let (player, reporter, was_full) = {
            let mut st = self.st.borrow_mut();
            let a = &mut st.media.audio;
            a.active = false;
            a.opened = false;
            a.serial += 1;
            a.timer.stop();
            let full = a.full;
            a.full = false;
            a.queue.set(Vec::new(), 0);
            (a.player.clone(), a.reporter.take(), full)
        };
        if let Some(p) = player {
            p.stop();
        }
        if let Some(r) = reporter {
            self.rt.spawn(async move { r.join().await });
        }
        if was_full {
            self.restore_focus_after_full();
        }
        self.with_media(|g| {
            g.set_np_active(false);
            g.set_np_full(false);
            g.set_np_playing(false);
        });
        {
            let st = self.st.borrow();
            if let Some(m) = &st.play.media {
                m.clear();
            }
        }
        self.media_refresh_playing();
        let on_media = self.st.borrow().media.zone == Z_BAR;
        if on_media {
            self.st.borrow_mut().media.zone = Z_BODY;
        }
    }

    // ------------------------------------------------------------ Tick und Anzeige

    fn audio_tick(self: &Rc<Self>) {
        let Some(player) = self.st.borrow().media.audio.player.clone() else {
            return;
        };
        let s = player.state();
        let mut advance: Option<bool> = None;
        {
            let mut st = self.st.borrow_mut();
            let a = &mut st.media.audio;
            if !a.active {
                return;
            }
            if a.opened && s.loaded {
                a.seen_loaded = true;
            }
            if a.opened && s.error.is_some() {
                a.opened = false;
                advance = Some(false);
            } else if a.opened && a.seen_loaded && s.eof {
                a.opened = false;
                advance = Some(true);
            }
            a.paused = s.paused;
            if a.opened {
                a.position = s.position;
                if let Some(d) = s.duration.filter(|d| *d > 0.0) {
                    a.duration = d;
                }
            }
        }
        if let Some(natural) = advance {
            if !natural {
                self.toast("Dieser Titel lässt sich nicht abspielen.", ToastKind::Error);
            }
            self.audio_next(true);
            return;
        }
        let (pos, dur, paused, opened, loaded) = {
            let st = self.st.borrow();
            let a = &st.media.audio;
            (a.position, a.duration, a.paused, a.opened, s.loaded)
        };
        let frac = if dur > 0.0 { (pos / dur) as f32 } else { 0.0 };
        self.with_media(|g| {
            g.set_np_pos(frac.clamp(0.0, 1.0));
            g.set_np_time(format!("{} / {}", clock(pos), clock(dur)).into());
            g.set_np_playing(opened && loaded && !paused);
        });
        self.audio_smtc_tick(opened && loaded, paused, pos, dur);
    }

    /// Schreibt Titel, Warteschlange und Schalter in die Oberfläche.
    pub fn audio_sync_ui(self: &Rc<Self>) {
        let (item, shuffle, repeat, rows, active) = {
            let st = self.st.borrow();
            let a = &st.media.audio;
            let rows: Vec<MediaTrack> = a
                .queue
                .upcoming()
                .into_iter()
                .enumerate()
                .filter_map(|(n, (i, cur))| {
                    a.queue.items.get(i).map(|t| MediaTrack {
                        num: (n + 1).to_string().into(),
                        title: t.name.as_str().into(),
                        sub: artist_line(t).into(),
                        right: duration_text(t).into(),
                        playing: cur,
                    })
                })
                .collect();
            (
                a.queue.current().cloned(),
                a.queue.shuffle,
                a.queue.repeat,
                rows,
                a.active,
            )
        };
        self.with_media(|g| {
            g.set_np_active(active);
            if let Some(i) = &item {
                g.set_np_title(i.name.as_str().into());
                g.set_np_artist(artist_line(i).into());
            }
            g.set_np_shuffle(shuffle);
            g.set_np_repeat(repeat.index());
            g.set_np_queue(ModelRc::new(VecModel::from(rows)));
        });
    }

    // ------------------------------------------------------------ Leiste und Vollbild

    pub fn bar_press(self: &Rc<Self>, i: i32) {
        match i {
            0 => self.audio_prev(),
            1 => self.audio_toggle(),
            2 => self.audio_next(false),
            3 => self.audio_toggle_shuffle(),
            4 => self.audio_cycle_repeat(),
            5 => self.audio_set_full(true),
            _ => {}
        }
    }

    pub fn full_press(self: &Rc<Self>, i: i32) {
        match i {
            0 => self.audio_toggle_shuffle(),
            1 => self.audio_prev(),
            2 => self.audio_toggle(),
            3 => self.audio_next(false),
            4 => self.audio_cycle_repeat(),
            5 => {
                if let Some(id) = self.audio_current_id() {
                    self.audio_mix(&id);
                }
            }
            _ => {}
        }
    }

    fn audio_toggle_shuffle(self: &Rc<Self>) {
        {
            let mut st = self.st.borrow_mut();
            let q = &mut st.media.audio.queue;
            let on = !q.shuffle;
            q.set_shuffle(on);
        }
        self.audio_sync_ui();
    }

    fn audio_cycle_repeat(self: &Rc<Self>) {
        {
            let mut st = self.st.borrow_mut();
            let q = &mut st.media.audio.queue;
            let r = q.repeat.cycle();
            q.set_repeat(r);
        }
        self.audio_sync_ui();
    }

    pub fn audio_set_full(self: &Rc<Self>, on: bool) {
        if on && !self.audio_active() {
            return;
        }
        let was = self.st.borrow().media.audio.full;
        if on == was {
            return;
        }
        if on {
            let mut st = self.st.borrow_mut();
            st.media.audio.saved_focus = (st.media.zone, st.media.index);
            st.media.audio.full = true;
            st.media.zone = Z_FULL;
            st.media.index = 2;
        } else {
            self.st.borrow_mut().media.audio.full = false;
            self.restore_focus_after_full();
        }
        self.with_media(|g| g.set_np_full(on));
        self.media_show_focus();
    }

    fn restore_focus_after_full(self: &Rc<Self>) {
        let mut st = self.st.borrow_mut();
        let (z, i) = st.media.audio.saved_focus;
        st.media.zone = z;
        st.media.index = i;
    }

    /// Tasten im Vollbild-Player; `true` = verbraucht.
    pub fn full_action(self: &Rc<Self>, a: NavAction) -> bool {
        let (zone, idx) = {
            let st = self.st.borrow();
            (st.media.zone, st.media.index)
        };
        let qlen = self.st.borrow().media.audio.queue.len();
        let cur_pos = self
            .st
            .borrow()
            .media
            .audio
            .queue
            .upcoming()
            .iter()
            .position(|(_, c)| *c)
            .unwrap_or(0);
        let (mut nz, mut ni) = (zone, idx);
        match (zone, a) {
            (_, NavAction::Back) => {
                self.audio_set_full(false);
                return true;
            }
            (Z_FULL, NavAction::Left) => ni = idx.saturating_sub(1),
            (Z_FULL, NavAction::Right) => ni = (idx + 1).min(6),
            (Z_FULL, NavAction::Down) => {
                if qlen > 0 {
                    nz = Z_QUEUE;
                    ni = cur_pos;
                }
            }
            (Z_QUEUE, NavAction::Up) => {
                if idx == 0 {
                    nz = Z_FULL;
                    ni = 2;
                } else {
                    ni = idx - 1;
                }
            }
            (Z_QUEUE, NavAction::Down) => ni = (idx + 1).min(qlen.saturating_sub(1)),
            (_, NavAction::SeekForward) | (_, NavAction::Right) if zone == Z_QUEUE => {
                self.audio_seek_by(10.0);
            }
            (_, NavAction::SeekBackward) | (_, NavAction::Left) if zone == Z_QUEUE => {
                self.audio_seek_by(-10.0);
            }
            (Z_FULL, NavAction::Activate) => {
                if idx == 6 {
                    self.audio_set_full(false);
                } else {
                    self.full_press(idx as i32);
                }
            }
            (Z_QUEUE, NavAction::Activate) => self.audio_jump(idx),
            (Z_QUEUE, NavAction::Menu | NavAction::Info) => self.queue_menu(idx),
            _ => {}
        }
        {
            let mut st = self.st.borrow_mut();
            st.media.zone = nz;
            st.media.index = ni;
        }
        self.media_show_focus();
        true
    }

    fn audio_seek_by(self: &Rc<Self>, secs: f64) {
        if let Some(p) = self.st.borrow().media.audio.player.clone() {
            p.seek_by(secs);
        }
    }

    pub fn queue_menu(self: &Rc<Self>, queue_pos: usize) {
        let idx = {
            let st = self.st.borrow();
            st.media
                .audio
                .queue
                .upcoming()
                .get(queue_pos)
                .map(|(i, _)| *i)
        };
        if let Some(i) = idx {
            let name = self
                .st
                .borrow()
                .media
                .audio
                .queue
                .items
                .get(i)
                .map(|t| t.name.clone())
                .unwrap_or_default();
            self.media_open_menu(
                &name,
                vec![
                    ("Abspielen".into(), Act::QueueRemove(usize::MAX)),
                    (
                        "Aus der Warteschlange entfernen".into(),
                        Act::QueueRemove(i),
                    ),
                ],
            );
        }
    }

    // ------------------------------------------------------------ Systemanzeige (Medientasten)

    fn audio_smtc_meta(self: &Rc<Self>, item: &BaseItem) {
        self.ensure_media_controls();
        let st = self.st.borrow();
        if let Some(m) = &st.play.media {
            m.set_now_playing(&NowPlaying {
                title: item.name.clone(),
                subtitle: artist_line(item),
                cover_file: None,
            });
            m.set_buttons(true, true, false);
        }
    }

    fn audio_smtc_tick(&self, playing_or_loaded: bool, paused: bool, pos: f64, dur: f64) {
        let mut guard = self.st.borrow_mut();
        let st = &mut *guard;
        let a = &mut st.media.audio;
        let Some(m) = &st.play.media else { return };
        let status = if !playing_or_loaded {
            PlaybackStatus::Changing
        } else if paused {
            PlaybackStatus::Paused
        } else {
            PlaybackStatus::Playing
        };
        if a.status != Some(status) {
            m.set_status(status);
            a.status = Some(status);
        }
        let due = a
            .timeline
            .is_none_or(|t| t.elapsed() >= Duration::from_secs(1));
        if due && dur > 0.0 {
            m.set_timeline(pos, dur);
            a.timeline = Some(Instant::now());
        }
    }

    /// Systembefehl (Medientaste) für die Musik; `true` = verbraucht.
    pub fn audio_media_command(self: &Rc<Self>, cmd: MediaCommand) -> bool {
        if !self.audio_active()
            || matches!(
                self.st.borrow().router.current(),
                crate::route::Route::Player { .. }
            )
        {
            return false;
        }
        let Some(p) = self.st.borrow().media.audio.player.clone() else {
            return false;
        };
        match cmd {
            MediaCommand::Play => p.play(),
            MediaCommand::Pause => p.pause(),
            MediaCommand::Toggle => self.audio_toggle(),
            MediaCommand::Stop => self.audio_stop(),
            MediaCommand::Next => self.audio_next(false),
            MediaCommand::Previous => self.audio_prev(),
            MediaCommand::FastForward => p.seek_by(10.0),
            MediaCommand::Rewind => p.seek_by(-10.0),
            MediaCommand::SeekTo(t) => p.seek_to(t),
        }
        true
    }

    /// Zustand für Tests.
    pub fn audio_state_json(&self) -> serde_json::Value {
        let st = self.st.borrow();
        let a = &st.media.audio;
        serde_json::json!({
            "active": a.active,
            "opened": a.opened,
            "title": a.queue.current().map(|i| i.name.clone()),
            "id": a.queue.current().map(|i| i.id.clone()),
            "queue": a.queue.len(),
            "shuffle": a.queue.shuffle,
            "repeat": a.queue.repeat.index(),
            "paused": a.paused,
            "position": a.position,
            "duration": a.duration,
            "full": a.full,
            "mpv_loaded": a.player.as_ref().map(|p| p.state().loaded),
            "mpv_eof": a.player.as_ref().map(|p| p.state().eof),
        })
    }

    /// Setzt Wiederholen (Automation).
    pub fn audio_set_repeat(self: &Rc<Self>, r: Repeat) {
        self.st.borrow_mut().media.audio.queue.set_repeat(r);
        self.audio_sync_ui();
    }
}
