//! Wiedergabe: Titel vorbereiten, Player und Videoausgabe starten, Fortschrittsmeldung, Verlassen.
//!
//! Das OSD (Ansicht, Menüs, Tasten) steht in `play_ui.rs`, Zusatzdienste (Medientasten, Wachhalten, Vollbild,
//! Trickplay) in `play_extras.rs`, die reine Zustandslogik in `play_osd.rs`, HDR in `play_hdr.rs`.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use lfx_api::{BaseItem, ItemKind, PlayMethod};
use lfx_core::images::{ImageRef, display_title, pick_landscape};
use lfx_images::Priority;
use lfx_platform::{KeepAwake, MediaControls};
use lfx_player::segments::{Segment, fetch_segments, next_episode};
use lfx_player::{
    ApiSink, PlayRequest, Player, PlayerConfig, PrepareOptions, ProgressReporter, ReporterConfig,
    prepare,
};
use slint::Image;

use crate::app::{App, post};
use crate::play_osd::{
    Control, FailStage, MenuItem, MenuKind, SubMode, SubStyle, TrackPrefs, TrickGeom, UpNext,
    episode_label, error_code, resume_offer,
};
use crate::route::Route;
use crate::video::{SharedStats, VideoPipe};

/// Wie sich der Start zu einem gemerkten Fortschritt verhält.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeMode {
    /// Bei vorhandenem Fortschritt nachfragen („Fortsetzen bei 12:34 / Von vorne“).
    Ask,
    /// Ohne Nachfrage fortsetzen.
    Resume,
    /// Von vorne.
    Restart,
}

/// Geöffnetes Menü.
pub struct MenuState {
    pub kind: MenuKind,
    pub items: Vec<MenuItem>,
    pub focus: usize,
}

/// Fortsetzen-Abfrage.
pub struct ResumeAsk {
    pub position: f64,
    /// 0 = Fortsetzen, 1 = Von vorne.
    pub focus: i32,
}

/// Trickplay: Geometrie und geladene Kachelbilder.
#[derive(Default)]
pub struct TrickState {
    pub geom: Option<TrickGeom>,
    pub width: u32,
    pub source_id: String,
    pub tiles: HashMap<u32, Image>,
    pub requested: HashSet<u32>,
    /// Zuletzt angefragter Bruchteil der Zeitleiste (Hover oder Ziehen).
    pub hover: Option<f32>,
}

/// Zustand der Wiedergabe.
#[derive(Default)]
pub struct PlayState {
    pub player: Option<Player>,
    pub pipe: Option<VideoPipe>,
    pub reporter: Option<ProgressReporter>,
    pub title: String,
    pub subtitle: String,
    pub last_activity: Option<Instant>,
    /// Zählt Wiedergabewünsche; späte Antworten alter Wünsche werden verworfen.
    pub generation: u64,
    /// Es wurde ein Titel an den Player übergeben.
    pub opened: bool,
    pub item: Option<BaseItem>,
    pub request: Option<PlayRequest>,
    pub segments: Vec<Segment>,
    pub next_item: Option<BaseItem>,
    pub autoplay_next: bool,
    pub up_next: UpNext,
    pub up_next_focus: i32,
    pub menu: Option<MenuState>,
    pub focus: Option<Control>,
    pub resume: Option<ResumeAsk>,
    pub cover: Option<Image>,
    /// Kapitel: (Beginn in Sekunden, Name).
    pub chapters: Vec<(f64, String)>,
    pub trick: TrickState,
    /// Beim Ziehen an der Zeitleiste: gezeigter Bruchteil.
    pub scrub: Option<f32>,
    pub sub_style: SubStyle,
    pub prefs: TrackPrefs,
    pub tracks_applied: bool,
    pub style_applied: bool,
    pub volume_applied: bool,
    /// Schlüssel des zuletzt angewendeten HDR-Plans (Videoparameter + Anzeige).
    pub hdr_key: Option<String>,
    pub hdr_status: String,
    /// Stufe aus `play_osd::QUALITIES`.
    pub quality: usize,
    /// Fehlercode und -text.
    pub error: Option<(String, String)>,
    pub fullscreen: bool,
    #[cfg(windows)]
    pub saved_window: Option<crate::play_extras::SavedWindow>,
    pub awake: Option<KeepAwake>,
    pub media: Option<MediaControls>,
    pub media_tried: bool,
    pub media_status: Option<lfx_platform::PlaybackStatus>,
    pub media_timeline: Option<Instant>,
    pub media_meta_sent: bool,
    pub cover_file_written: bool,
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
        (ItemKind::Episode, Some(series)) if !series.is_empty() => series.to_owned(),
        _ => item.name.clone(),
    }
}

fn play_subtitle(item: &BaseItem) -> String {
    if item.kind == ItemKind::Episode {
        episode_label(item.parent_index_number, item.index_number, &item.name)
    } else {
        String::new()
    }
}

/// Fortschritt des Titels in Sekunden (nur wenn nicht als gesehen markiert).
fn saved_position(item: &BaseItem) -> f64 {
    item.user_data
        .as_ref()
        .filter(|u| !u.played && u.playback_position_ticks > 0)
        .map_or(0.0, |u| u.playback_position_ticks as f64 / 10_000_000.0)
}

fn runtime_secs(item: &BaseItem) -> f64 {
    item.run_time_ticks.unwrap_or(0) as f64 / 10_000_000.0
}

/// Zusatzdaten, die parallel zum Start geladen werden.
struct Extras {
    full: Option<BaseItem>,
    segments: Vec<Segment>,
    next: Option<BaseItem>,
    prefs: TrackPrefs,
    autoplay: bool,
}

impl App {
    /// Startet die Wiedergabe eines Titels (fragt bei vorhandenem Fortschritt nach).
    pub fn play_item(self: &Rc<Self>, item: &BaseItem) {
        self.play_item_with(item, ResumeMode::Ask);
    }

    /// Startet die Wiedergabe mit ausdrücklicher Fortsetzen-Wahl (z. B. Detailseite: „Von vorne“).
    pub fn play_item_with(self: &Rc<Self>, item: &BaseItem, mode: ResumeMode) {
        self.start_item(item, mode, false);
    }

    fn start_item(self: &Rc<Self>, item: &BaseItem, mode: ResumeMode, replace: bool) {
        let Some(active) = self.session.active() else {
            return;
        };
        let sub_style = self.load_sub_style();
        let epoch = {
            let mut st = self.st.borrow_mut();
            let keep_player = st.play.player.take();
            let keep_pipe = st.play.pipe.take();
            let keep_media = st.play.media.take();
            let keep_tried = st.play.media_tried;
            let keep_fs = st.play.fullscreen;
            #[cfg(windows)]
            let keep_saved = st.play.saved_window.take();
            let gen_next = st.play.generation + 1;
            st.play = PlayState {
                player: keep_player,
                pipe: keep_pipe,
                media: keep_media,
                media_tried: keep_tried,
                fullscreen: keep_fs,
                generation: gen_next,
                title: play_title(item),
                subtitle: play_subtitle(item),
                last_activity: Some(Instant::now()),
                item: Some(item.clone()),
                sub_style,
                autoplay_next: true,
                ..PlayState::default()
            };
            #[cfg(windows)]
            {
                st.play.saved_window = keep_saved;
            }
            gen_next
        };
        self.audio_stop(); // W4-media: Musik endet, wenn ein Video startet
        if replace {
            self.go_replace(Route::Player {
                id: item.id.clone(),
            });
        } else {
            self.go(Route::Player {
                id: item.id.clone(),
            });
        }
        self.start_osd_timer();
        self.refresh_view();
        // Startbild: Backdrop, sonst Querformat.
        if let Some(pick) = lfx_core::images::own_image(item, lfx_api::ImageType::Backdrop)
            .or_else(|| pick_landscape(item))
        {
            let r = ImageRef {
                pick,
                max_width: 1920,
                quality: 80,
            };
            self.fetch_image(&r, Priority::Visible, move |app, img| {
                if app.st.borrow().play.generation != epoch {
                    return;
                }
                app.st.borrow_mut().play.cover = Some(img);
                app.write_cover_file();
                app.refresh_view();
            });
        }
        // Zusatzdaten parallel laden.
        let client = active.client.clone();
        let it = item.clone();
        self.rt.spawn(async move {
            let full = client
                .item(&it.id, &["Chapters", "Trickplay", "MediaSources"])
                .await
                .ok();
            let segments = fetch_segments(&client, &it.id).await.unwrap_or_default();
            let next = if it.kind == ItemKind::Episode {
                next_episode(&client, &it).await.ok().flatten()
            } else {
                None
            };
            let cfg = client.user_configuration().await.ok();
            let prefs = cfg
                .as_ref()
                .map_or_else(TrackPrefs::default, |c| TrackPrefs {
                    audio_lang: c
                        .audio_language_preference
                        .clone()
                        .filter(|s| !s.is_empty()),
                    sub_lang: c
                        .subtitle_language_preference
                        .clone()
                        .filter(|s| !s.is_empty()),
                    sub_mode: SubMode::parse(c.subtitle_mode.as_deref()),
                });
            let autoplay = cfg.as_ref().is_none_or(|c| c.enable_next_episode_auto_play);
            let extras = Extras {
                full,
                segments,
                next,
                prefs,
                autoplay,
            };
            post(move |app| app.extras_loaded(epoch, extras));
        });
        // Fortsetzen?
        let saved = saved_position(item);
        let offer = resume_offer(saved, runtime_secs(item));
        match (mode, offer) {
            (ResumeMode::Ask, Some(pos)) => {
                self.st.borrow_mut().play.resume = Some(ResumeAsk {
                    position: pos,
                    focus: 0,
                });
                self.refresh_view();
            }
            (ResumeMode::Restart, _) | (ResumeMode::Ask, None) => {
                self.prepare_and_open(epoch, 0.0, None);
            }
            (ResumeMode::Resume, _) => {
                self.prepare_and_open(epoch, saved, None);
            }
        }
    }

    /// Antwort der Fortsetzen-Abfrage.
    pub fn resume_chosen(self: &Rc<Self>, from_start: bool) {
        let (epoch, pos) = {
            let mut st = self.st.borrow_mut();
            let Some(r) = st.play.resume.take() else {
                return;
            };
            (st.play.generation, r.position)
        };
        self.prepare_and_open(epoch, if from_start { 0.0 } else { pos }, None);
        self.refresh_view();
    }

    /// Bereitet vor und öffnet (Qualitätswechsel startet hier mit der aktuellen Position neu).
    pub fn prepare_and_open(self: &Rc<Self>, epoch: u64, start: f64, max_bitrate: Option<u64>) {
        let Some(active) = self.session.active() else {
            return;
        };
        let Some(item) = self.st.borrow().play.item.clone() else {
            return;
        };
        let client = active.client.clone();
        // W2-detail: Vorauswahl aus Detail/Menü (Ton, Untertitel, Version, von Anfang an)
        let choice = self.take_play_choice().unwrap_or_default();
        self.rt.spawn(async move {
            let opts = PrepareOptions {
                start_seconds: if choice.from_start { 0.0 } else { start },
                max_bitrate,
                media_source_id: choice.source,
                audio_stream_index: choice.audio,
                subtitle_stream_index: choice.sub,
                ..PrepareOptions::default()
            };
            match prepare(&client, &item.id, &opts).await {
                Ok(req) => post(move |app| app.begin_playback(epoch, req)),
                Err(e) => {
                    let text = e.to_string();
                    post(move |app| app.playback_failed(epoch, FailStage::Prepare, 0, &text));
                }
            }
        });
    }

    fn extras_loaded(self: &Rc<Self>, epoch: u64, ex: Extras) {
        {
            let mut st = self.st.borrow_mut();
            if st.play.generation != epoch {
                return;
            }
            st.play.segments = ex.segments;
            st.play.next_item = ex.next;
            st.play.prefs = ex.prefs;
            st.play.autoplay_next = ex.autoplay;
            if let Some(full) = ex.full {
                st.play.chapters = full
                    .chapters
                    .iter()
                    .map(|c| {
                        (
                            c.start_position_ticks as f64 / 10_000_000.0,
                            c.name.clone().unwrap_or_default(),
                        )
                    })
                    .collect();
                // Trickplay: Quelle der laufenden Wiedergabe, kleinste Breite ab 240 px.
                let source = st
                    .play
                    .request
                    .as_ref()
                    .map(|r| r.media_source_id.clone())
                    .or_else(|| full.media_sources.first().map(|s| s.id.clone()));
                if let (Some(map), Some(source)) = (full.trickplay.as_ref(), source) {
                    let infos = map.get(&source).or_else(|| map.values().next());
                    if let Some(infos) = infos {
                        let mut list: Vec<(&String, &lfx_api::TrickplayInfo)> =
                            infos.iter().collect();
                        list.sort_by_key(|(_, i)| i.width);
                        let pick = list
                            .iter()
                            .find(|(_, i)| i.width >= 240)
                            .or_else(|| list.last());
                        if let Some((_, i)) = pick {
                            st.play.trick.geom = Some(TrickGeom {
                                width: i.width,
                                height: i.height,
                                tile_w: i.tile_width,
                                tile_h: i.tile_height,
                                count: i.thumbnail_count,
                                interval_ms: i.interval,
                            });
                            st.play.trick.width = i.width;
                            st.play.trick.source_id = source;
                        }
                    }
                }
            }
        }
        self.refresh_view();
    }

    pub fn playback_failed(
        self: &Rc<Self>,
        epoch: u64,
        stage: FailStage,
        mpv_code: i32,
        text: &str,
    ) {
        if self.st.borrow().play.generation != epoch {
            return;
        }
        tracing::warn!("Wiedergabe nicht möglich: {text}");
        self.st.borrow_mut().play.error = Some((error_code(stage, mpv_code), text.to_owned()));
        self.refresh_view();
    }

    fn begin_playback(self: &Rc<Self>, epoch: u64, req: PlayRequest) {
        if self.st.borrow().play.generation != epoch
            || !matches!(self.st.borrow().router.current(), Route::Player { .. })
        {
            return;
        }
        let Some(layer) = self.shell.video_layer() else {
            self.playback_failed(epoch, FailStage::Output, 0, "Wiedergabe auf macOS folgt");
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
                    self.playback_failed(
                        epoch,
                        FailStage::Engine,
                        e.code,
                        &format!("Der Player konnte nicht gestartet werden.\n{}", e.text),
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
                    self.playback_failed(epoch, FailStage::Output, 0, &text);
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
            st.play.request = Some(req.clone());
        }
        self.ensure_media_controls();
        player.open(req);
        tracing::info!("Wiedergabe gestartet");
    }

    // ---------------------------------------------------------------- Weiter, zurück, Ende

    /// Nächste Folge starten (Karte, Knopf, Ende, Medientaste).
    pub fn play_next(self: &Rc<Self>) {
        let Some(next) = self.st.borrow().play.next_item.clone() else {
            return;
        };
        self.stop_playback_keep_route();
        self.start_item(&next, ResumeMode::Restart, true);
    }

    /// Qualität wechseln: an gleicher Stelle neu öffnen.
    pub fn change_quality(self: &Rc<Self>, level: usize) {
        let Some(&(_, bitrate)) = crate::play_osd::QUALITIES.get(level) else {
            return;
        };
        let (pos, epoch) = {
            let st = self.st.borrow();
            let pos = st.play.player.as_ref().map_or(0.0, |p| p.state().position);
            (pos, st.play.generation)
        };
        {
            let mut st = self.st.borrow_mut();
            st.play.quality = level;
            st.play.menu = None;
            st.play.tracks_applied = false;
            st.play.hdr_key = None;
        }
        // Alte Sitzung beenden, dann neu vorbereiten (erst leeren, dann ActiveEncodings löschen).
        let new_epoch = epoch + 1;
        self.stop_playback_keep_route();
        {
            let mut st = self.st.borrow_mut();
            st.play.generation = new_epoch;
        }
        self.refresh_view();
        self.prepare_and_open(new_epoch, pos, bitrate);
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
        self.stop_playback_keep_route();
        self.exit_fullscreen();
        let ui = self.win();
        ui.set_pv(lfx_ui::PlayerView::default());
        self.clear_media_controls();
    }

    /// Wie `stop_playback`, aber ohne die Oberfläche zu leeren (Folgewiedergabe, Qualitätswechsel).
    fn stop_playback_keep_route(self: &Rc<Self>) {
        let (player, reporter, request) = {
            let mut st = self.st.borrow_mut();
            st.play.generation += 1;
            st.play.opened = false;
            st.play.awake = None;
            (
                st.play.player.clone(),
                st.play.reporter.take(),
                st.play.request.take(),
            )
        };
        if let Some(p) = player {
            p.stop();
        }
        // Umwandlung beenden: erst der Player (oben), dann ActiveEncodings löschen.
        let delete = request
            .as_ref()
            .filter(|r| r.play_method != PlayMethod::DirectPlay)
            .and_then(|r| r.play_session_id.clone());
        let client = self.session.active().map(|a| a.client.clone());
        if let (Some(psid), Some(client)) = (delete, client) {
            self.rt.spawn(async move {
                if let Err(e) = client.delete_active_encodings(&psid).await {
                    tracing::debug!("ActiveEncodings nicht gelöscht: {e}");
                }
            });
        }
        if let Some(r) = reporter {
            // „Stopped“ geht im Hintergrund hinaus; die Oberfläche wartet nicht darauf.
            self.rt.spawn(async move { r.join().await });
        }
        if let Some(layer) = self.shell.video_layer() {
            layer.set_visible(false);
        }
    }

    /// Zähler der Videoausgabe (für Tests).
    pub fn video_stats(&self) -> Option<SharedStats> {
        self.st.borrow().play.pipe.as_ref().map(|p| p.stats.clone())
    }

    /// Anzeigetitel des Titels (für Medientasten).
    pub fn now_playing_title(&self) -> (String, String) {
        let st = self.st.borrow();
        match &st.play.item {
            Some(i) if i.kind == ItemKind::Episode => (display_title(i), st.play.subtitle.clone()),
            Some(i) => (display_title(i), String::new()),
            None => (String::new(), String::new()),
        }
    }
}
