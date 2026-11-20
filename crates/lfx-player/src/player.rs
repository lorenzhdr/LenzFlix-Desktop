//! Der Player: ein eigener Thread besitzt die mpv-Ereignisschleife; Kommandos kommen per Kanal,
//! der Zustand wird per `watch` und die Ereignisse per `broadcast` veröffentlicht.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use lfx_mpv_sys::ffi::{MPV_FORMAT_DOUBLE, MPV_FORMAT_FLAG, MPV_FORMAT_STRING};
use lfx_mpv_sys::{Api, EndReason, Event, Mpv, PropValue};
use tokio::sync::{broadcast, watch};

use crate::request::{PlayRequest, StreamKind};
use crate::state::{
    ChapterInfo, EndKind, PlayerError, PlayerEvent, PlayerState, Track, TrackKind, VideoParams,
};

/// Wohin mpv das Bild schickt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VideoMode {
    /// `vo=libmpv`: die App rendert über ein [`crate::VideoOutput`] (D3D11 oder OpenGL).
    #[default]
    Render,
    /// `vo=null`: kein Bild (Tests, reine Tonwiedergabe).
    Headless,
}

/// Einstellungen beim Erzeugen des Players.
#[derive(Debug, Clone)]
pub struct PlayerConfig {
    pub video: VideoMode,
    /// Tonausgabe abschalten (`ao=null`, für Tests).
    pub audio_null: bool,
    /// Hardware-Decoder (`hwdec`); Standard `auto-safe` (unter Windows `d3d11va`).
    pub hwdec: String,
    /// Weitere mpv-Optionen (überschreiben die Standardwerte).
    pub extra_options: Vec<(String, String)>,
}

impl Default for PlayerConfig {
    fn default() -> Self {
        Self {
            video: VideoMode::Render,
            audio_null: false,
            hwdec: "auto-safe".to_owned(),
            extra_options: Vec::new(),
        }
    }
}

impl PlayerConfig {
    /// Konfiguration für Tests ohne Bild und Ton.
    pub fn headless() -> Self {
        Self {
            video: VideoMode::Headless,
            audio_null: true,
            hwdec: "no".to_owned(),
            extra_options: Vec::new(),
        }
    }
}

/// Auswahl einer Ton- oder Untertitelspur.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackChoice {
    /// mpv wählt selbst.
    Auto,
    /// Aus (nur sinnvoll für Untertitel).
    Off,
    /// mpv-Spur-Id aus [`PlayerState::tracks`].
    Id(i64),
}

enum Cmd {
    Open(Box<PlayRequest>),
    Pause(bool),
    Toggle,
    Seek { secs: f64, absolute: bool },
    Track(TrackKind, TrackChoice),
    Volume(f64),
    Mute(bool),
    Speed(f64),
    Stop,
    Shutdown,
}

struct Inner {
    mpv: Arc<Mpv>,
    tx: Sender<Cmd>,
    state: watch::Sender<PlayerState>,
    events: broadcast::Sender<PlayerEvent>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Shutdown);
        self.mpv.wakeup();
        let handle = self.thread.lock().ok().and_then(|mut t| t.take());
        if let Some(h) = handle
            && h.thread().id() != std::thread::current().id()
        {
            let _ = h.join();
        }
    }
}

/// Handle auf den Player (billig klonbar; der letzte Klon beendet mpv).
#[derive(Clone)]
pub struct Player {
    inner: Arc<Inner>,
}

impl Player {
    /// Lädt libmpv (Suchpfade siehe `lfx_mpv_sys::api`) und startet den Player.
    pub fn new(config: PlayerConfig) -> Result<Self, PlayerError> {
        let api = Api::load().map_err(|e| PlayerError {
            code: 0,
            text: format!("libmpv nicht ladbar: {e}"),
        })?;
        Self::with_api(Arc::new(api), config)
    }

    /// Startet den Player mit einer bereits geladenen libmpv.
    pub fn with_api(api: Arc<Api>, config: PlayerConfig) -> Result<Self, PlayerError> {
        let mpv = Mpv::new(api).map_err(to_err)?;
        configure(&mpv, &config);
        mpv.initialize().map_err(to_err)?;
        let mpv = Arc::new(mpv);
        let (tx, rx) = channel();
        let (state_tx, _) = watch::channel(PlayerState::default());
        let (ev_tx, _) = broadcast::channel(256);
        let worker = Worker::new(mpv.clone(), state_tx.clone(), ev_tx.clone());
        let handle = std::thread::Builder::new()
            .name("lfx-player-mpv".into())
            .spawn(move || worker.run(rx))
            .map_err(|e| PlayerError {
                code: 0,
                text: format!("Thread nicht startbar: {e}"),
            })?;
        Ok(Self {
            inner: Arc::new(Inner {
                mpv,
                tx,
                state: state_tx,
                events: ev_tx,
                thread: Mutex::new(Some(handle)),
            }),
        })
    }

    /// mpv-Handle für Render-Kontexte ([`crate::D3d11Output`], [`crate::GlOutput`]).
    pub fn mpv(&self) -> Arc<Mpv> {
        self.inner.mpv.clone()
    }

    /// Aktueller Zustand.
    pub fn state(&self) -> PlayerState {
        self.inner.state.borrow().clone()
    }

    /// Zustandskanal (`changed().await`).
    pub fn subscribe_state(&self) -> watch::Receiver<PlayerState> {
        self.inner.state.subscribe()
    }

    /// Ereigniskanal (Opened, FileLoaded, PauseChanged, ...).
    pub fn subscribe_events(&self) -> broadcast::Receiver<PlayerEvent> {
        self.inner.events.subscribe()
    }

    fn send(&self, cmd: Cmd) {
        if self.inner.tx.send(cmd).is_ok() {
            self.inner.mpv.wakeup();
        }
    }

    /// Öffnet einen Titel (ersetzt die laufende Wiedergabe). Fehler erscheinen als
    /// [`PlayerState::error`] und als [`PlayerEvent::Ended`].
    pub fn open(&self, request: PlayRequest) {
        self.send(Cmd::Open(Box::new(request)));
    }

    pub fn play(&self) {
        self.send(Cmd::Pause(false));
    }

    pub fn pause(&self) {
        self.send(Cmd::Pause(true));
    }

    pub fn toggle_pause(&self) {
        self.send(Cmd::Toggle);
    }

    /// Springt auf die Position in Sekunden.
    pub fn seek_to(&self, seconds: f64) {
        self.send(Cmd::Seek {
            secs: seconds,
            absolute: true,
        });
    }

    /// Springt relativ (negativ = zurück).
    pub fn seek_by(&self, seconds: f64) {
        self.send(Cmd::Seek {
            secs: seconds,
            absolute: false,
        });
    }

    pub fn set_audio_track(&self, choice: TrackChoice) {
        self.send(Cmd::Track(TrackKind::Audio, choice));
    }

    /// Untertitel wählen; `TrackChoice::Off` schaltet sie ab.
    pub fn set_subtitle_track(&self, choice: TrackChoice) {
        self.send(Cmd::Track(TrackKind::Subtitle, choice));
    }

    /// Lautstärke (mpv-Skala, 100 = Normal).
    pub fn set_volume(&self, volume: f64) {
        self.send(Cmd::Volume(volume));
    }

    pub fn set_muted(&self, muted: bool) {
        self.send(Cmd::Mute(muted));
    }

    pub fn set_speed(&self, speed: f64) {
        self.send(Cmd::Speed(speed));
    }

    /// Beendet die Wiedergabe (mpv bleibt bereit).
    pub fn stop(&self) {
        self.send(Cmd::Stop);
    }
}

fn to_err(e: lfx_mpv_sys::MpvError) -> PlayerError {
    PlayerError {
        code: e.code,
        text: e.text,
    }
}

/// Standardoptionen setzen. Unbekannte Optionen (andere mpv-Version) sind kein Fehler.
fn configure(mpv: &Mpv, cfg: &PlayerConfig) {
    let mut opts: Vec<(&str, String)> = vec![
        (
            "vo",
            match cfg.video {
                VideoMode::Render => "libmpv",
                VideoMode::Headless => "null",
            }
            .to_owned(),
        ),
        ("hwdec", cfg.hwdec.clone()),
        ("terminal", "no".into()),
        ("idle", "yes".into()),
        ("keep-open", "yes".into()),
        // Kein mpv-eigenes Bedienkonzept, keine Skripte/Konfigdateien.
        ("config", "no".into()),
        ("load-scripts", "no".into()),
        ("ytdl", "no".into()),
        ("osc", "no".into()),
        ("osd-level", "0".into()),
        ("input-default-bindings", "no".into()),
        ("input-vo-keyboard", "no".into()),
        ("resume-playback", "no".into()),
        ("save-position-on-quit", "no".into()),
        ("audio-display", "no".into()),
        ("sub-auto", "no".into()),
        // Aus G3: Spitzenmessung kostet bei 4K-HDR die Bildrate.
        ("hdr-compute-peak", "no".into()),
        // Netzwerk-Cache.
        ("cache", "yes".into()),
        ("demuxer-max-bytes", "150MiB".into()),
        ("demuxer-max-back-bytes", "50MiB".into()),
        ("demuxer-readahead-secs", "60".into()),
        ("network-timeout", "30".into()),
        // Untertitel-Grundstil.
        ("sub-font-size", "46".into()),
        ("sub-color", "#FFFFFF".into()),
        ("sub-border-color", "#000000".into()),
        ("sub-border-size", "2.4".into()),
        ("sub-shadow-offset", "1".into()),
        ("sub-pos", "96".into()),
    ];
    if cfg.audio_null {
        opts.push(("ao", "null".into()));
    }
    for (k, v) in opts.iter().map(|(k, v)| (*k, v.as_str())).chain(
        cfg.extra_options
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str())),
    ) {
        if let Err(e) = mpv.set_option_string(k, v) {
            tracing::debug!("mpv-Option {k} nicht gesetzt: {e}");
        }
    }
}

// Beobachtungs-Ids.
const OBS_PAUSE: u64 = 1;
const OBS_TIME: u64 = 2;
const OBS_DURATION: u64 = 3;
const OBS_CACHE: u64 = 4;
const OBS_EOF: u64 = 5;
const OBS_VOLUME: u64 = 6;
const OBS_MUTE: u64 = 7;
const OBS_SPEED: u64 = 8;
const OBS_AID: u64 = 9;
const OBS_SID: u64 = 10;
const OBS_HWDEC: u64 = 11;

struct Worker {
    mpv: Arc<Mpv>,
    state: watch::Sender<PlayerState>,
    events: broadcast::Sender<PlayerEvent>,
    generation: u64,
    request: Option<Arc<PlayRequest>>,
    /// Läuft eine Wiedergabe (seit `open`, bis zum Ende-Ereignis)?
    active: bool,
    /// Wie viele `EndFile`-Ereignisse gehören zu bereits abgeschlossenen Wiedergaben?
    skip_end: u32,
    loaded: bool,
    seek_pending: bool,
    last_pos_sent: f64,
}

impl Worker {
    fn new(
        mpv: Arc<Mpv>,
        state: watch::Sender<PlayerState>,
        events: broadcast::Sender<PlayerEvent>,
    ) -> Self {
        Self {
            mpv,
            state,
            events,
            generation: 0,
            request: None,
            active: false,
            skip_end: 0,
            loaded: false,
            seek_pending: false,
            last_pos_sent: -1.0,
        }
    }

    fn run(mut self, rx: Receiver<Cmd>) {
        let obs = [
            (OBS_PAUSE, "pause", MPV_FORMAT_FLAG),
            (OBS_TIME, "time-pos", MPV_FORMAT_DOUBLE),
            (OBS_DURATION, "duration", MPV_FORMAT_DOUBLE),
            (OBS_CACHE, "paused-for-cache", MPV_FORMAT_FLAG),
            (OBS_EOF, "eof-reached", MPV_FORMAT_FLAG),
            (OBS_VOLUME, "volume", MPV_FORMAT_DOUBLE),
            (OBS_MUTE, "mute", MPV_FORMAT_FLAG),
            (OBS_SPEED, "speed", MPV_FORMAT_DOUBLE),
            (OBS_AID, "aid", MPV_FORMAT_STRING),
            (OBS_SID, "sid", MPV_FORMAT_STRING),
            (OBS_HWDEC, "hwdec-current", MPV_FORMAT_STRING),
        ];
        for (id, name, fmt) in obs {
            if let Err(e) = self.mpv.observe_property(id, name, fmt) {
                tracing::warn!("observe {name}: {e}");
            }
        }
        'outer: loop {
            while let Ok(cmd) = rx.try_recv() {
                if matches!(cmd, Cmd::Shutdown) {
                    break 'outer;
                }
                self.command(cmd);
            }
            match self.mpv.wait_event(0.25) {
                Event::Shutdown => break,
                ev => self.event(ev),
            }
        }
        if self.active {
            self.active = false;
            self.emit(PlayerEvent::Ended {
                generation: self.generation,
                kind: EndKind::Quit,
            });
        }
    }

    fn emit(&self, ev: PlayerEvent) {
        // Kein Empfänger ist in Ordnung.
        let _ = self.events.send(ev);
    }

    fn modify(&self, f: impl FnOnce(&mut PlayerState)) {
        self.state.send_if_modified(|s| {
            let before = s.clone();
            f(s);
            *s != before
        });
    }

    fn fail(&self, text: String, code: i32) {
        tracing::warn!("Player: {text} (mpv {code})");
        let err = PlayerError { code, text };
        self.modify(|s| s.error = Some(err));
    }

    fn command(&mut self, cmd: Cmd) {
        let r = match cmd {
            Cmd::Open(req) => {
                self.open(*req);
                return;
            }
            Cmd::Pause(p) => self.mpv.set_property_flag("pause", p),
            Cmd::Toggle => {
                let p = self.mpv.get_property_flag("pause").unwrap_or(false);
                self.mpv.set_property_flag("pause", !p)
            }
            Cmd::Seek { secs, absolute } => {
                let mode = if absolute {
                    "absolute+exact"
                } else {
                    "relative+exact"
                };
                self.mpv.command(&["seek", &format!("{secs:.3}"), mode])
            }
            Cmd::Track(kind, choice) => {
                let prop = if kind == TrackKind::Audio {
                    "aid"
                } else {
                    "sid"
                };
                let value = match choice {
                    TrackChoice::Auto => "auto".to_owned(),
                    TrackChoice::Off => "no".to_owned(),
                    TrackChoice::Id(n) => n.to_string(),
                };
                self.mpv.set_property_string(prop, &value)
            }
            Cmd::Volume(v) => self.mpv.set_property_f64("volume", v.clamp(0.0, 200.0)),
            Cmd::Mute(m) => self.mpv.set_property_flag("mute", m),
            Cmd::Speed(s) => self.mpv.set_property_f64("speed", s.clamp(0.1, 8.0)),
            Cmd::Stop => self.mpv.command(&["stop"]),
            Cmd::Shutdown => return,
        };
        if let Err(e) = r {
            self.fail(e.text, e.code);
        }
    }

    fn open(&mut self, req: PlayRequest) {
        if self.active {
            // Die laufende Wiedergabe endet; mpv schickt dafür noch ein EndFile, das ignoriert wird.
            self.emit(PlayerEvent::Ended {
                generation: self.generation,
                kind: EndKind::Stop,
            });
            self.skip_end += 1;
        }
        self.generation += 1;
        self.loaded = false;
        self.seek_pending = false;
        self.last_pos_sent = -1.0;
        let generation = self.generation;
        let start = req.start_seconds.max(0.0);
        self.modify(|s| {
            let (volume, muted, speed) = (s.volume, s.muted, s.speed);
            *s = PlayerState {
                generation,
                position: start,
                volume,
                muted,
                speed,
                ..PlayerState::default()
            };
        });
        let req = Arc::new(req);
        self.request = Some(req.clone());
        self.emit(PlayerEvent::Opened {
            generation,
            item_id: req.item_id.clone(),
            media_source_id: req.media_source_id.clone(),
        });

        // Header können Kommas enthalten (Authorization): einzeln per `change-list ... append`
        // anhängen (ein Element, keine Listenzerlegung).
        if let Err(e) = self
            .mpv
            .command(&["change-list", "http-header-fields", "clr", ""])
        {
            tracing::warn!("http-header-fields leeren: {e}");
        }
        for h in &req.http_headers {
            if let Err(e) = self
                .mpv
                .command(&["change-list", "http-header-fields", "append", h])
            {
                tracing::warn!("http-header-fields anhängen: {e}");
            }
        }
        let _ = self.mpv.set_property_flag("pause", false);

        let mut opts: Vec<String> = Vec::new();
        if start > 0.0 {
            opts.push(format!("start={start:.3}"));
        }
        if let Some(aid) = req
            .audio_stream_index
            .and_then(|i| req.mpv_track_id(StreamKind::Audio, i))
        {
            opts.push(format!("aid={aid}"));
        }
        match req.subtitle_stream_index {
            Some(i) if i < 0 => opts.push("sid=no".into()),
            Some(i) => {
                let embedded = req
                    .streams
                    .iter()
                    .any(|s| s.index == i && s.kind == StreamKind::Subtitle && !s.external);
                if embedded && let Some(sid) = req.mpv_track_id(StreamKind::Subtitle, i) {
                    opts.push(format!("sid={sid}"));
                }
            }
            None => {}
        }
        let r = if opts.is_empty() {
            self.mpv.command(&["loadfile", &req.stream_url, "replace"])
        } else {
            self.mpv.command(&[
                "loadfile",
                &req.stream_url,
                "replace",
                "-1",
                &opts.join(","),
            ])
        };
        match r {
            Ok(()) => self.active = true,
            Err(e) => {
                self.active = false;
                let err = PlayerError {
                    code: e.code,
                    text: e.text,
                };
                self.fail(err.text.clone(), err.code);
                self.emit(PlayerEvent::Ended {
                    generation,
                    kind: EndKind::Error(err),
                });
            }
        }
    }

    fn event(&mut self, ev: Event) {
        match ev {
            Event::FileLoaded if self.active => self.on_loaded(),
            Event::EndFile { reason } => self.on_end(reason),
            Event::VideoReconfig => self.refresh_video(),
            Event::Seek if self.loaded => self.seek_pending = true,
            Event::PlaybackRestart if self.seek_pending => {
                self.seek_pending = false;
                if let Ok(p) = self.mpv.get_property_f64("time-pos") {
                    self.last_pos_sent = p;
                    self.modify(|s| s.position = p);
                }
                self.emit(PlayerEvent::Seeked {
                    generation: self.generation,
                });
            }
            Event::PropertyChange {
                userdata, value, ..
            } => self.on_property(userdata, value),
            _ => {}
        }
    }

    fn on_end(&mut self, reason: EndReason) {
        if self.skip_end > 0 {
            self.skip_end -= 1;
            return;
        }
        if !self.active {
            return;
        }
        self.active = false;
        self.loaded = false;
        let kind = match reason {
            EndReason::Eof => EndKind::Eof,
            EndReason::Error(code) => {
                let err = PlayerError {
                    code,
                    text: self.mpv.api().error_string(code),
                };
                self.fail(err.text.clone(), err.code);
                EndKind::Error(err)
            }
            EndReason::Quit => EndKind::Quit,
            EndReason::Stop | EndReason::Redirect | EndReason::Other(_) => EndKind::Stop,
        };
        self.modify(|s| s.loaded = false);
        self.emit(PlayerEvent::Ended {
            generation: self.generation,
            kind,
        });
    }

    fn on_loaded(&mut self) {
        if let Some(req) = self.request.clone() {
            for sub in &req.external_subtitles {
                let title = sub.title.as_deref().unwrap_or("");
                let lang = sub.lang.as_deref().unwrap_or("");
                if let Err(e) = self
                    .mpv
                    .command(&["sub-add", &sub.url, "auto", title, lang])
                {
                    tracing::warn!("sub-add: {e}");
                }
            }
            // Explizite Untertitelwahl nach dem Anhängen durchsetzen.
            match req.subtitle_stream_index {
                Some(i) if i < 0 => {
                    let _ = self.mpv.set_property_string("sid", "no");
                }
                Some(i) => {
                    if let Some(sid) = req.mpv_track_id(StreamKind::Subtitle, i) {
                        let _ = self.mpv.set_property_string("sid", &sid.to_string());
                    }
                }
                None => {}
            }
        }
        self.loaded = true;
        let tracks = self.read_tracks();
        let chapters = self.read_chapters();
        self.modify(|s| {
            s.loaded = true;
            s.tracks = tracks;
            s.chapters = chapters;
        });
        self.refresh_video();
        self.emit(PlayerEvent::FileLoaded {
            generation: self.generation,
        });
    }

    fn on_property(&mut self, id: u64, value: PropValue) {
        let generation = self.generation;
        match (id, value) {
            (OBS_PAUSE, PropValue::Flag(p)) => {
                let changed = self.state.borrow().paused != p;
                self.modify(|s| s.paused = p);
                if changed && self.loaded {
                    self.emit(PlayerEvent::PauseChanged {
                        generation,
                        paused: p,
                    });
                }
            }
            (OBS_TIME, PropValue::Double(d)) => {
                if self.active && !self.state.borrow().eof && (d - self.last_pos_sent).abs() >= 0.1
                {
                    self.last_pos_sent = d;
                    self.modify(|s| s.position = d);
                }
            }
            (OBS_DURATION, PropValue::Double(d)) => self.modify(|s| s.duration = Some(d)),
            (OBS_DURATION, PropValue::None) => self.modify(|s| s.duration = None),
            (OBS_CACHE, PropValue::Flag(b)) => self.modify(|s| s.buffering = b),
            (OBS_EOF, PropValue::Flag(b)) => {
                let was = self.state.borrow().eof;
                if b && !was && self.active && self.loaded {
                    // Endposition festhalten, dann melden.
                    let dur = self
                        .mpv
                        .get_property_f64("duration")
                        .ok()
                        .or(self.state.borrow().duration);
                    self.modify(|s| {
                        s.eof = true;
                        if let Some(d) = dur {
                            s.position = d;
                        }
                    });
                    self.emit(PlayerEvent::Eof { generation });
                } else if !b {
                    self.modify(|s| s.eof = false);
                }
            }
            (OBS_VOLUME, PropValue::Double(v)) => self.modify(|s| s.volume = v),
            (OBS_MUTE, PropValue::Flag(m)) => self.modify(|s| s.muted = m),
            (OBS_SPEED, PropValue::Double(v)) => self.modify(|s| s.speed = v),
            (OBS_AID | OBS_SID, _) => {
                if self.loaded {
                    let tracks = self.read_tracks();
                    let changed = self.state.borrow().tracks != tracks;
                    self.modify(|s| s.tracks = tracks);
                    if changed {
                        self.emit(PlayerEvent::TrackChanged { generation });
                    }
                }
            }
            (OBS_HWDEC, PropValue::String(s)) => {
                let v = (!s.is_empty() && s != "no").then_some(s);
                self.modify(|st| st.hwdec = v);
            }
            (OBS_HWDEC, _) => self.modify(|s| s.hwdec = None),
            _ => {}
        }
    }

    fn opt_string(&self, name: &str) -> Option<String> {
        self.mpv
            .get_property_string(name)
            .ok()
            .filter(|s| !s.is_empty())
    }

    fn flag(&self, name: &str) -> bool {
        self.mpv.get_property_flag(name).unwrap_or(false)
    }

    fn read_tracks(&self) -> Vec<Track> {
        let n = self.mpv.get_property_i64("track-list/count").unwrap_or(0);
        (0..n)
            .filter_map(|i| {
                let p = |f: &str| format!("track-list/{i}/{f}");
                let kind = match self.opt_string(&p("type"))?.as_str() {
                    "video" => TrackKind::Video,
                    "audio" => TrackKind::Audio,
                    "sub" => TrackKind::Subtitle,
                    _ => return None,
                };
                Some(Track {
                    id: self.mpv.get_property_i64(&p("id")).ok()?,
                    kind,
                    lang: self.opt_string(&p("lang")),
                    title: self.opt_string(&p("title")),
                    codec: self.opt_string(&p("codec")),
                    default: self.flag(&p("default")),
                    forced: self.flag(&p("forced")),
                    external: self.flag(&p("external")),
                    selected: self.flag(&p("selected")),
                })
            })
            .collect()
    }

    fn read_chapters(&self) -> Vec<ChapterInfo> {
        let n = self.mpv.get_property_i64("chapter-list/count").unwrap_or(0);
        (0..n)
            .filter_map(|i| {
                Some(ChapterInfo {
                    title: self.opt_string(&format!("chapter-list/{i}/title")),
                    start: self
                        .mpv
                        .get_property_f64(&format!("chapter-list/{i}/time"))
                        .ok()?,
                })
            })
            .collect()
    }

    fn refresh_video(&self) {
        let f = |n: &str| self.mpv.get_property_f64(&format!("video-params/{n}")).ok();
        let params = VideoParams {
            width: self.mpv.get_property_i64("video-params/w").unwrap_or(0),
            height: self.mpv.get_property_i64("video-params/h").unwrap_or(0),
            primaries: self.opt_string("video-params/primaries"),
            gamma: self.opt_string("video-params/gamma"),
            sig_peak: f("sig-peak"),
            max_cll: f("max-cll"),
            max_fall: f("max-fall"),
        };
        self.modify(|s| s.video = params);
    }
}
