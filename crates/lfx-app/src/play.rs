//! Wiedergabe: Titel vorbereiten, Player und Videoausgabe starten, OSD, Fortschrittsmeldung, Verlassen.

use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use lfx_api::{BaseItem, ItemKind};
use lfx_nav::NavAction;
use lfx_player::{
    ApiSink, PlayRequest, Player, PlayerConfig, PrepareOptions, ProgressReporter, ReporterConfig,
    prepare,
};

use crate::app::{App, post};
use crate::route::Route;
use crate::video::{SharedStats, VideoPipe};

/// Wie lange das OSD nach der letzten Eingabe sichtbar bleibt.
const OSD_HIDE_AFTER: Duration = Duration::from_secs(3);

/// Zustand der Wiedergabe.
#[derive(Default)]
pub struct PlayState {
    pub player: Option<Player>,
    pub pipe: Option<VideoPipe>,
    pub reporter: Option<ProgressReporter>,
    pub title: String,
    pub last_activity: Option<Instant>,
    /// Zählt Wiedergabewünsche; späte Antworten alter Wünsche werden verworfen.
    pub generation: u64,
    /// Es wurde ein Titel an den Player übergeben.
    pub opened: bool,
}

/// `m:ss` bzw. `h:mm:ss`.
pub fn format_time(secs: f64) -> String {
    let s = secs.max(0.0).round() as u64;
    let (h, m, sec) = (s / 3600, (s / 60) % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{sec:02}")
    } else {
        format!("{m}:{sec:02}")
    }
}

fn play_title(item: &BaseItem) -> String {
    match (item.kind, item.series_name.as_deref()) {
        (ItemKind::Episode, Some(series)) if !series.is_empty() => {
            format!("{series}: {}", item.name)
        }
        _ => item.name.clone(),
    }
}

impl App {
    /// Startet die Wiedergabe eines Titels.
    pub fn play_item(self: &Rc<Self>, item: &BaseItem) {
        let Some(active) = self.session.active() else {
            return;
        };
        let epoch = {
            let mut st = self.st.borrow_mut();
            st.play.generation += 1;
            st.play.title = play_title(item);
            st.play.last_activity = Some(Instant::now());
            st.play.opened = false;
            st.play.generation
        };
        self.go(Route::Player {
            id: item.id.clone(),
        });
        let ui = self.win();
        ui.set_player_title(play_title(item).as_str().into());
        ui.set_player_note("".into());
        ui.set_pos_text("0:00".into());
        ui.set_dur_text("0:00".into());
        ui.set_progress(0.0);
        ui.set_paused(false);
        ui.set_osd(true);
        ui.set_buffering(true);
        self.start_osd_timer();
        let resume = item
            .user_data
            .as_ref()
            .filter(|u| !u.played && u.playback_position_ticks > 0)
            .map_or(0.0, |u| u.playback_position_ticks as f64 / 10_000_000.0);
        let client = active.client.clone();
        let id = item.id.clone();
        // W2-detail: Vorauswahl aus Detail/Menü (Ton, Untertitel, Version, von Anfang an)
        let choice = self.take_play_choice().unwrap_or_default();
        self.rt.spawn(async move {
            let opts = PrepareOptions {
                start_seconds: if choice.from_start { 0.0 } else { resume },
                media_source_id: choice.source,
                audio_stream_index: choice.audio,
                subtitle_stream_index: choice.sub,
                ..PrepareOptions::default()
            };
            match prepare(&client, &id, &opts).await {
                Ok(req) => post(move |app| app.begin_playback(epoch, req)),
                Err(e) => {
                    let text = e.to_string();
                    post(move |app| app.playback_failed(epoch, &text));
                }
            }
        });
    }

    fn playback_failed(self: &Rc<Self>, epoch: u64, text: &str) {
        if self.st.borrow().play.generation != epoch {
            return;
        }
        tracing::warn!("Wiedergabe nicht möglich: {text}");
        let ui = self.win();
        ui.set_buffering(false);
        ui.set_player_note(format!("Wiedergabe nicht möglich.\n{text}").into());
        ui.set_osd(true);
    }

    fn begin_playback(self: &Rc<Self>, epoch: u64, req: PlayRequest) {
        if self.st.borrow().play.generation != epoch
            || !matches!(self.st.borrow().router.current(), Route::Player { .. })
        {
            return;
        }
        let ui = self.win();
        let Some(layer) = self.shell.video_layer() else {
            ui.set_buffering(false);
            ui.set_player_note("Wiedergabe auf macOS folgt".into());
            return;
        };
        let Some(active) = self.session.active() else {
            return;
        };
        // Player und Ausgabe einmal anlegen und wiederverwenden.
        if self.st.borrow().play.player.is_none() {
            match Player::new(PlayerConfig::default()) {
                Ok(p) => self.st.borrow_mut().play.player = Some(p),
                Err(e) => {
                    ui.set_buffering(false);
                    ui.set_player_note(
                        format!("Der Player konnte nicht gestartet werden.\n{}", e.text).into(),
                    );
                    return;
                }
            }
        }
        let player = self
            .st
            .borrow()
            .play
            .player
            .clone()
            .expect("Player vorhanden");
        if self.st.borrow().play.pipe.is_none() {
            match VideoPipe::start(&player, layer.clone()) {
                Ok(p) => self.st.borrow_mut().play.pipe = Some(p),
                Err(text) => {
                    ui.set_buffering(false);
                    ui.set_player_note(text.into());
                    return;
                }
            }
        }
        layer.set_rect(None);
        layer.set_visible(true);
        let interval = self.opts.report_interval.unwrap_or(Duration::from_secs(10));
        let reporter = {
            let _guard = self.rt.enter();
            ProgressReporter::spawn(
                &player,
                req.clone(),
                Arc::new(ApiSink::new(active.client.clone())),
                ReporterConfig {
                    report_progress: true,
                    interval,
                },
            )
        };
        {
            let mut st = self.st.borrow_mut();
            if let Some(old) = st.play.reporter.replace(reporter) {
                old.abort();
            }
            st.play.opened = true;
        }
        player.open(req);
        tracing::info!("Wiedergabe gestartet");
    }

    // ---------------------------------------------------------------- OSD

    fn start_osd_timer(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.timers.osd.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(250),
            move || {
                if let Some(app) = weak.upgrade() {
                    app.osd_tick();
                }
            },
        );
    }

    fn osd_tick(self: &Rc<Self>) {
        if !matches!(self.st.borrow().router.current(), Route::Player { .. }) {
            self.timers.osd.stop();
            return;
        }
        let (player, last, opened) = {
            let st = self.st.borrow();
            (
                st.play.player.clone(),
                st.play.last_activity,
                st.play.opened,
            )
        };
        let Some(player) = player.filter(|_| opened) else {
            return;
        };
        let s = player.state();
        let ui = self.win();
        ui.set_pos_text(format_time(s.position).into());
        let dur = s.duration.unwrap_or(0.0);
        ui.set_dur_text(format_time(dur).into());
        ui.set_progress(if dur > 0.0 {
            (s.position / dur).clamp(0.0, 1.0) as f32
        } else {
            0.0
        });
        ui.set_paused(s.paused);
        ui.set_buffering(s.buffering || (!s.loaded && s.error.is_none()));
        if let Some(e) = &s.error {
            ui.set_player_note(format!("Wiedergabe nicht möglich.\n{}", e.text).into());
        }
        let recent = last.is_some_and(|t| t.elapsed() < OSD_HIDE_AFTER);
        ui.set_osd(s.paused || recent || s.error.is_some());
        if s.eof && s.loaded {
            tracing::info!("Ende erreicht");
            self.leave_player();
        }
    }

    /// Bewegung oder Eingabe: OSD zeigen.
    pub fn player_activity(self: &Rc<Self>) {
        self.st.borrow_mut().play.last_activity = Some(Instant::now());
        if matches!(self.st.borrow().router.current(), Route::Player { .. }) {
            self.win().set_osd(true);
        }
    }

    /// Aktionen im Player-Kontext (Tastatur, Gamepad).
    pub fn player_action(self: &Rc<Self>, action: NavAction) {
        let Some(player) = self.st.borrow().play.player.clone() else {
            if action == NavAction::Back {
                self.leave_player();
            }
            return;
        };
        let s = player.state();
        match action {
            NavAction::Back | NavAction::Stop => self.leave_player(),
            NavAction::PlayPause | NavAction::Activate => player.toggle_pause(),
            NavAction::Play => player.play(),
            NavAction::Pause => player.pause(),
            NavAction::SeekForward => player.seek_by(10.0),
            NavAction::SeekBackward => player.seek_by(-10.0),
            NavAction::VolumeUp => player.set_volume((s.volume + 5.0).min(130.0)),
            NavAction::VolumeDown => player.set_volume((s.volume - 5.0).max(0.0)),
            NavAction::Mute => player.set_muted(!s.muted),
            _ => {}
        }
    }

    pub fn toggle_pause(self: &Rc<Self>) {
        self.player_activity();
        if let Some(p) = self.st.borrow().play.player.clone() {
            p.toggle_pause();
        }
    }

    pub fn seek_by(self: &Rc<Self>, secs: f32) {
        self.player_activity();
        if let Some(p) = self.st.borrow().play.player.clone() {
            p.seek_by(f64::from(secs));
        }
    }

    pub fn seek_to_fraction(self: &Rc<Self>, frac: f32) {
        self.player_activity();
        if let Some(p) = self.st.borrow().play.player.clone()
            && let Some(d) = p.state().duration
        {
            p.seek_to(d * f64::from(frac.clamp(0.0, 1.0)));
        }
    }

    /// Verlässt den Player: stoppen (der Melder sendet „Stopped“), Video ausblenden, zurück.
    pub fn leave_player(self: &Rc<Self>) {
        if !matches!(self.st.borrow().router.current(), Route::Player { .. }) {
            return;
        }
        self.stop_playback();
        self.go_back();
    }

    /// Stoppt die laufende Wiedergabe und blendet das Video aus.
    pub fn stop_playback(self: &Rc<Self>) {
        self.timers.osd.stop();
        let (player, reporter) = {
            let mut st = self.st.borrow_mut();
            st.play.generation += 1;
            st.play.opened = false;
            (st.play.player.clone(), st.play.reporter.take())
        };
        if let Some(p) = player {
            p.stop();
        }
        if let Some(r) = reporter {
            // „Stopped“ geht im Hintergrund hinaus; die Oberfläche wartet nicht darauf.
            self.rt.spawn(async move { r.join().await });
        }
        if let Some(layer) = self.shell.video_layer() {
            layer.set_visible(false);
        }
        let ui = self.win();
        ui.set_buffering(false);
        ui.set_player_note("".into());
    }

    /// Zähler der Videoausgabe (für Tests).
    pub fn video_stats(&self) -> Option<SharedStats> {
        self.st.borrow().play.pipe.as_ref().map(|p| p.stats.clone())
    }
}
