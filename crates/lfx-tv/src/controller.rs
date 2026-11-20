//! TV-Controller: UI-freie Zustandsmaschine des TV-Modus (Port von `tvmode/controller.js`).
//!
//! Der Controller ist **sans-IO**: Er führt weder Wiedergabe noch Netzwerk noch Timer selbst aus.
//! * Eingaben: Befehle ([`TvController::tune`], [`TvController::zap`] ...), Antworten des Players
//!   (`on_play_result` ...) und [`TvController::poll`] (lässt fällige Zeitgeber wirken).
//! * Ausgaben: [`TvOutput`], abgeholt mit [`TvController::take_output`]. Befehle an den Player
//!   tragen eine Kennung ([`Token`], der Generationszähler): Antworten mit veralteter Kennung
//!   werden verworfen.
//! * Zeit nur über den injizierten [`Clock`]; [`TvController::next_deadline`] sagt, wann `poll`
//!   wieder aufzurufen ist. Fällige Zeitgeber laufen in der Reihenfolge (Zeitpunkt, Anlegereihenfolge)
//!   und sehen dabei ihren Fälligkeitszeitpunkt als „jetzt“ (wie Fake-Timer).
//!
//! Der TV-Modus meldet nie Fortschritt (keine Spuren in „Weiterschauen“); `TvPlayRequest` trägt
//! `report_progress = false`.

use lfx_tv_schedule::time::{broadcast_day_of, format_clock};
use lfx_tv_schedule::{EntryType, PlanEntry};

use crate::clock::Clock;
use crate::digits::{Digits, DigitsEvent};
use crate::filler::{FillerPick, SecretUrl, TrailerInfo, pick_filler, trailer_target};
use crate::model::{next_program, title_of};
use crate::neighbors::{
    Mode, NeighborAction, NeighborCtx, Neighbors, WARM_AFTER_ERROR, WarmPhase, ZapFlags,
};
use crate::plan::PlanSource;
use crate::request::{PlayResult, PlayerPhase, Timing, Token, TvPlayRequest, VolumeMode};
use crate::stream::FailReason;
use crate::tracks::Tracks;
use crate::watch::WatchTracker;
use crate::zapstats::ZapInfo;

/// Entprellung beim Zappen.
pub const DEBOUNCE_MS: i64 = 250;
/// Neuversuch nach Netzwerkfehler.
pub const RETRY_MS: i64 = 10_000;
/// Abstand der Positionsabgleiche.
pub const SYNC_MS: i64 = 30_000;
/// Stillstand, ab dem die Gesehen-Zählung verworfen wird.
pub const STALL_MS: i64 = 60_000;
/// Anzahl der Sender.
pub const CHANNELS: i64 = 18;
const MAX_TIMEOUT: i64 = 2_147_483_647;
const VOL_STEP: f64 = 5.0;
const VOL_SETTLE_MS: i64 = 1000;
const STATS_MAX: usize = 10;
const TICK_MS: i64 = 1000;

/// Phase der Anzeige.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Off,
    Loading,
    Playing,
    Pause,
    Closedown,
    Error,
}

/// Grund neben der Phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    Network,
    Gesture,
    Browser,
    Video,
    Audio,
    Empty,
}

impl Reason {
    /// Name wie im Web.
    pub fn as_str(self) -> &'static str {
        match self {
            Reason::Network => "network",
            Reason::Gesture => "gesture",
            Reason::Browser => "browser",
            Reason::Video => "video",
            Reason::Audio => "audio",
            Reason::Empty => "empty",
        }
    }
}

/// Titel und Untertitel einer Fehlerkarte (`ERROR_TEXTS`).
pub fn error_texts(reason: Reason) -> (&'static str, &'static str) {
    match reason {
        Reason::Video => (
            "Kein Empfang auf diesem Gerät",
            "Dieses Gerät kann den Sender nicht empfangen",
        ),
        Reason::Browser => (
            "Kein Empfang auf diesem Gerät",
            "Bitte einen anderen Browser verwenden",
        ),
        Reason::Audio => (
            "Ton auf diesem Gerät nicht empfangbar",
            "Dem Konto fehlt die Berechtigung zur Ton-Umwandlung",
        ),
        Reason::Gesture => (
            "Zum Starten tippen",
            "Der Browser braucht einmal ein Tippen",
        ),
        Reason::Network | Reason::Empty => ("Empfangsstörung – erneuter Versuch …", ""),
    }
}

/// Zustandskarte im Player.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Card {
    /// Pause mit Folgesendung („Gleich: Titel um HH:MM“).
    Next {
        title: String,
        at: String,
        at_ms: i64,
    },
    /// Pause ohne Folgetitel.
    Pause { text: String },
    /// Sendepause („weiter um HH:MM“).
    Closedown { text: String, until: String },
    /// Auf dem Sender läuft nichts.
    Empty { text: String },
    /// Fehler mit Grund, Code und ggf. Hinweis.
    Error {
        reason: Reason,
        title: String,
        sub: String,
        code: Option<String>,
        hint: Option<String>,
    },
}

/// Eingefrorener Zustand für die Ansicht.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub on: bool,
    /// Sendernummer 1..18.
    pub channel: i64,
    /// Ansicht: aktuelle Sendung; beim Zappen (Entprellung) nur die Vorschau des neuen Senders.
    pub entry: Option<PlanEntry>,
    pub phase: Phase,
    pub reason: Option<Reason>,
    pub card: Option<Card>,
    /// Hinweistext bei Füllprogramm.
    pub banner: Option<String>,
    /// Wiedergabe braucht eine Geste.
    pub needs_tap: bool,
    /// Autoplay nur stumm erlaubt.
    pub muted_by_policy: bool,
    /// 0..100 in 5er-Schritten.
    pub volume: u8,
    /// Nutzer-Stumm oder Autoplay-Sperre.
    pub muted: bool,
}

/// Zu speichernde Einstellungen (nur gesetzte Felder).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PrefsPatch {
    pub last_channel: Option<i64>,
    /// 0..1
    pub volume: Option<f64>,
    pub muted: Option<bool>,
}

/// Kennung eines Tonwechsels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioTicket {
    pub generation: u64,
    pub entry_seq: u64,
}

/// Ausgaben des Controllers: Ereignisse für die Ansicht und Befehle an den Host.
#[derive(Debug, Clone, PartialEq)]
pub enum TvOutput {
    // ---- Ereignisse ----
    /// Zustand hat sich geändert.
    Change(Box<Snapshot>),
    /// Kurzer Hinweistext (Toast).
    Message(String),
    /// Takt, jede Sekunde (aktuelle Zeit).
    Tick(i64),
    /// Sendetag hat gewechselt (05:00).
    Day(i64),
    /// Messung eines Umschaltens.
    Zap(Box<ZapInfo>),
    /// Anzeige der Ziffern-Eingabe (`None` leert sie).
    Digits(Option<String>),
    // ---- Befehle an den Host ----
    /// Wiedergabe für diese Geste freischalten (synchron ausführen).
    Unlock,
    /// Sendung starten.
    Play(TvPlayRequest),
    /// Trailer starten; Antwort mit `on_play_url_result`.
    PlayUrl {
        token: Token,
        url: SecretUrl,
        offset_ms: i64,
    },
    /// Trailer des Titels laden; Antwort mit `on_trailer`.
    LoadTrailer { token: Token, item_id: String },
    /// Wiedergabe beenden (Bild leeren, dann Server-Sitzung beenden).
    StopPlayer,
    /// Live-Position halten (Abweichung > 10 s: springen).
    SyncTo(i64),
    /// Pegel 0..1 setzen.
    SetVolume(f64),
    /// Stumm setzen.
    SetMuted(bool),
    /// PlaybackInfo des Zielsenders vorauflösen (Entprellung).
    PrefetchTarget { item_id: String },
    /// Vorauflösung des Zielsenders abbrechen.
    CancelPrefetch,
    /// PlaybackInfo eines Nachbarn vorauflösen.
    Prefetch { item_id: String },
    /// Sitzung eines Nachbarn vorwärmen.
    Prewarm { item_id: String, offset_ms: i64 },
    /// Notiz für die Messanzeige.
    WarmNote(&'static str),
    /// Vorgewärmte Sitzungen sofort beenden.
    ReleaseWarm,
    /// Ausstrahlung als gesehen markieren.
    SetPlayed { item_id: String },
    /// Einstellungen speichern (Fehler egal).
    SavePrefs(PrefsPatch),
    /// Pläne der Sendetage vorbereiten (Sammlungen laden).
    PrepareDays(Vec<i64>),
    /// Tonspur wechseln; Antwort mit `on_audio_result`.
    SetAudio { ticket: AudioTicket, index: i32 },
    /// Untertitel wählen (`-1` = aus).
    SetSubtitle(i32),
}

/// Zusammenfassung der letzten Umschaltungen.
#[derive(Debug, Clone, PartialEq)]
pub struct ZapRecord {
    pub channel: i64,
    pub ms: i64,
    pub timing: Option<Timing>,
}

/// Messung (`ctl.stats()`).
#[derive(Debug, Clone, PartialEq)]
pub struct Stats {
    pub zaps: Vec<ZapRecord>,
    pub last_ms: Option<i64>,
    pub avg_ms: Option<i64>,
}

/// Wiedergabeziel der aktuellen Ansicht („Aus der Mediathek abspielen“).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestartTarget {
    pub item_id: String,
    /// `movie`, `episode` oder Katalogart.
    pub kind: String,
    pub series_id: Option<String>,
}

/// Startoptionen.
#[derive(Debug, Clone)]
pub struct TvOptions {
    /// Zuletzt gesehener Sender (Standard 1).
    pub last_channel: i64,
    /// Pegel 0..1; Werte über 1 gelten als Prozent.
    pub volume: f64,
    pub muted: bool,
    /// Gesehen-Zählung aktiv (Standard ja).
    pub watch: bool,
}

impl Default for TvOptions {
    fn default() -> Self {
        Self {
            last_channel: 1,
            volume: 1.0,
            muted: false,
            watch: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TimerKind {
    Zap,
    Advance,
    Retry,
    Trailer,
    Stall,
    Tick,
}

#[derive(Debug, Clone, Copy)]
struct Timer {
    kind: TimerKind,
    at: i64,
    seq: u64,
    generation: u64,
}

fn playable(t: EntryType) -> bool {
    matches!(
        t,
        EntryType::Program | EntryType::Fill | EntryType::Nightfill
    )
}

fn reason_of_fail(r: FailReason) -> Reason {
    match r {
        FailReason::Video => Reason::Video,
        FailReason::Audio => Reason::Audio,
        FailReason::Browser => Reason::Browser,
        FailReason::Gesture => Reason::Gesture,
        FailReason::Network | FailReason::Aborted => Reason::Network,
    }
}

fn clamp_i(n: i64, a: i64, b: i64) -> i64 {
    n.max(a).min(b)
}

fn round_js(x: f64) -> f64 {
    lfx_tv_schedule::jsval::round_js(x)
}

/// Der TV-Controller.
pub struct TvController<P: PlanSource> {
    plan: P,
    clock: Box<dyn Clock>,
    out: Vec<TvOutput>,
    cur: i64,
    seq: u64,
    timers: Vec<Timer>,
    nb: Neighbors,
    digits: Digits,
    watch: Option<WatchTracker>,
    flags: ZapFlags,
    disposed: bool,
    on: bool,
    channel: i64,
    entry: Option<PlanEntry>,
    entry_seq: u64,
    view: Option<PlanEntry>,
    generation: u64,
    playing: bool,
    trailer_on: bool,
    phase: Phase,
    reason: Option<Reason>,
    card: Option<Card>,
    banner: Option<String>,
    needs_tap: bool,
    muted_by_policy: bool,
    stall_armed: bool,
    tick_count: u64,
    last_day: Option<i64>,
    pf_active: bool,
    tune_at: Option<i64>,
    zaps: Vec<ZapRecord>,
    vol: i64,
    muted: bool,
    last_vol: i64,
    vol_prev: i64,
    vol_at: i64,
    volume_mode: VolumeMode,
    tracks: Tracks,
}

impl<P: PlanSource> TvController<P> {
    /// Neuer Controller (aus). `plan` gehört ab jetzt dem Controller.
    pub fn new(plan: P, clock: Box<dyn Clock>, opts: TvOptions) -> Self {
        let now = clock.now_ms();
        let ch = round_js(opts.last_channel as f64) as i64;
        let channel = clamp_i(if ch == 0 { 1 } else { ch }, 1, CHANNELS);
        let raw = if opts.volume.is_finite() {
            opts.volume
        } else {
            1.0
        };
        let vol = clamp_i(
            round_js(if raw > 1.0 { raw / 100.0 } else { raw } * 100.0) as i64,
            0,
            100,
        );
        Self {
            plan,
            clock,
            out: Vec::new(),
            cur: now,
            seq: 0,
            timers: Vec::new(),
            nb: Neighbors::new(),
            digits: Digits::default(),
            watch: opts.watch.then(WatchTracker::new),
            flags: ZapFlags::default(),
            disposed: false,
            on: false,
            channel,
            entry: None,
            entry_seq: 0,
            view: None,
            generation: 0,
            playing: false,
            trailer_on: false,
            phase: Phase::Off,
            reason: None,
            card: None,
            banner: None,
            needs_tap: false,
            muted_by_policy: false,
            stall_armed: false,
            tick_count: 0,
            last_day: None,
            pf_active: false,
            tune_at: None,
            zaps: Vec::new(),
            vol,
            muted: opts.muted,
            last_vol: if vol > 0 { vol } else { 50 },
            vol_prev: vol,
            vol_at: now,
            volume_mode: VolumeMode::Native,
            tracks: Tracks::empty(),
        }
    }

    // ------------------------------------------------------------------ Hilfen

    fn sync_now(&mut self) {
        self.cur = self.clock.now_ms();
    }

    fn emit(&mut self, o: TvOutput) {
        self.out.push(o);
    }

    /// Holt alle aufgelaufenen Ausgaben.
    pub fn take_output(&mut self) -> Vec<TvOutput> {
        std::mem::take(&mut self.out)
    }

    /// Planer (lesend, für Anzeige-Modelle).
    pub fn plan_mut(&mut self) -> &mut P {
        &mut self.plan
    }

    fn silent(&self) -> bool {
        self.muted || self.vol == 0
    }

    /// Zustand für die Ansicht.
    pub fn state(&self) -> Snapshot {
        Snapshot {
            on: self.on,
            channel: self.channel,
            entry: self.view.clone(),
            phase: self.phase,
            reason: self.reason,
            card: self.card.clone(),
            banner: self.banner.clone(),
            needs_tap: self.needs_tap,
            muted_by_policy: self.muted_by_policy,
            volume: self.vol as u8,
            muted: self.silent() || self.muted_by_policy,
        }
    }

    fn changed(&mut self) {
        if self.disposed {
            return;
        }
        let s = self.state();
        self.emit(TvOutput::Change(Box::new(s)));
    }

    fn message(&mut self, text: &str) {
        if !self.disposed {
            self.emit(TvOutput::Message(text.to_string()));
        }
    }

    fn save_prefs(&mut self, p: PrefsPatch) {
        self.emit(TvOutput::SavePrefs(p));
    }

    // ------------------------------------------------------------------ Zeitgeber

    fn later(&mut self, kind: TimerKind, ms: i64, generation: u64) {
        self.unlater(kind);
        self.seq += 1;
        self.timers.push(Timer {
            kind,
            at: self.cur + ms,
            seq: self.seq,
            generation,
        });
    }

    fn unlater(&mut self, kind: TimerKind) {
        self.timers.retain(|t| t.kind != kind);
    }

    /// Nächster Zeitpunkt, an dem [`poll`](Self::poll) etwas bewirkt.
    pub fn next_deadline(&self) -> Option<i64> {
        let mut best: Option<i64> = self.timers.iter().map(|t| t.at).min();
        for d in [self.nb.next_deadline(), self.digits.next_deadline()]
            .into_iter()
            .flatten()
        {
            best = Some(best.map_or(d, |b| b.min(d)));
        }
        best
    }

    /// Lässt alle fälligen Zeitgeber wirken.
    pub fn poll(&mut self) {
        if self.disposed {
            return;
        }
        let now = self.clock.now_ms();
        loop {
            // Reihenfolge bei gleichem Zeitpunkt: Zeitgeber (Anlegereihenfolge), Nachbarn, Ziffern.
            let timer = self
                .timers
                .iter()
                .filter(|t| t.at <= now)
                .min_by_key(|t| (t.at, t.seq))
                .copied();
            let nb = self.nb.next_deadline().filter(|d| *d <= now);
            let dg = self.digits.next_deadline().filter(|d| *d <= now);
            let t_at = timer.map(|t| t.at);
            let pick = [(t_at, 0u8), (nb, 1), (dg, 2)]
                .into_iter()
                .filter_map(|(at, k)| at.map(|a| (a, k)))
                .min();
            let Some((at, which)) = pick else { break };
            self.cur = at;
            match which {
                0 => {
                    if let Some(t) = timer {
                        self.timers.retain(|x| x.seq != t.seq);
                        self.fire(t);
                    }
                }
                1 => self.poll_neighbors(at),
                _ => {
                    let ev = self.digits.poll(at);
                    self.handle_digits(ev);
                }
            }
        }
        self.cur = now;
    }

    fn fire(&mut self, t: Timer) {
        match t.kind {
            TimerKind::Zap => self.start_gen(t.generation, true),
            TimerKind::Advance => self.advance(t.generation),
            TimerKind::Retry => {
                if t.generation == self.generation && self.on {
                    self.start_gen(t.generation, false);
                }
            }
            TimerKind::Trailer => {
                if t.generation == self.generation
                    && self.on
                    && let Some(e) = self.entry.clone()
                {
                    self.run_filler(&e, t.generation);
                }
            }
            TimerKind::Stall => self.watch_leave(true),
            TimerKind::Tick => {
                self.timers.push(Timer {
                    kind: TimerKind::Tick,
                    at: t.at + TICK_MS,
                    seq: {
                        self.seq += 1;
                        self.seq
                    },
                    generation: 0,
                });
                self.on_tick();
            }
        }
    }

    // ------------------------------------------------------------------ Nachbarn

    fn warm_phase(&self) -> WarmPhase {
        if self.phase == Phase::Error {
            WarmPhase::Error
        } else if self.playing && self.phase == Phase::Playing {
            WarmPhase::Playing
        } else {
            WarmPhase::Other
        }
    }

    fn poll_neighbors(&mut self, at: i64) {
        let phase = self.warm_phase();
        let mut ctx = NeighborCtx {
            plan: &mut self.plan,
            channel: self.channel,
            phase,
            flags: self.flags,
            channel_count: CHANNELS,
        };
        let actions = self.nb.poll(at, &mut ctx);
        for a in actions {
            match a {
                NeighborAction::Prewarm { item_id, offset_ms } => {
                    self.emit(TvOutput::Prewarm { item_id, offset_ms })
                }
                NeighborAction::Prefetch { item_id } => self.emit(TvOutput::Prefetch { item_id }),
                NeighborAction::Note(n) => self.emit(TvOutput::WarmNote(n)),
            }
        }
    }

    /// Schalter (Netzregel, Datensparmodus) ändern; wirkt ab dem nächsten Schritt.
    pub fn set_zap_flags(&mut self, flags: ZapFlags) {
        self.flags = flags;
    }

    /// Datensparmodus an: Nachbar-Takt stoppen, vorgewärmte Sitzungen sofort beenden.
    pub fn release_warm(&mut self) {
        self.nb.stop();
        self.emit(TvOutput::ReleaseWarm);
    }

    // ------------------------------------------------------------------ Lautstärke

    fn mute_only(&self) -> bool {
        self.volume_mode == VolumeMode::MuteOnly
    }

    /// Lautstärke-Modus des Geräts (vom Player gemeldet).
    pub fn set_volume_mode(&mut self, m: VolumeMode) {
        self.volume_mode = m;
    }

    /// `native` oder `mute-only` (Messanzeige).
    pub fn volume_mode(&self) -> VolumeMode {
        self.volume_mode
    }

    /// Der Regler bleibt in allen Modi sichtbar.
    pub fn volume_supported(&self) -> bool {
        true
    }

    fn note_vol(&mut self) {
        if self.vol == self.vol_prev {
            return;
        }
        if self.vol_prev > 0 && self.cur - self.vol_at >= VOL_SETTLE_MS {
            self.last_vol = self.vol_prev;
        }
        self.vol_prev = self.vol;
        self.vol_at = self.cur;
    }

    fn apply_volume(&mut self) {
        self.note_vol();
        let v = if self.mute_only() {
            1.0
        } else {
            self.vol as f64 / 100.0
        };
        self.emit(TvOutput::SetVolume(v));
        let s = self.silent();
        self.emit(TvOutput::SetMuted(s));
    }

    /// Pegel in Prozent (gerundet auf 5er-Schritte); hebt die Autoplay-Sperre auf.
    pub fn set_volume(&mut self, v: f64) {
        if self.disposed {
            return;
        }
        self.sync_now();
        let v = if v.is_finite() { v } else { 0.0 };
        self.vol = clamp_i((round_js(v / VOL_STEP) * VOL_STEP) as i64, 0, 100);
        self.muted_by_policy = false;
        self.muted = false;
        self.apply_volume();
        let patch = PrefsPatch {
            volume: Some(self.vol as f64 / 100.0),
            muted: Some(self.muted),
            ..PrefsPatch::default()
        };
        self.save_prefs(patch);
        self.changed();
    }

    /// Stumm setzen. Entstummen bei Pegel 0 stellt den letzten Pegel über 0 wieder her.
    pub fn set_muted(&mut self, b: bool) {
        if self.disposed {
            return;
        }
        self.sync_now();
        self.muted = b;
        self.muted_by_policy = false;
        let restore = !self.muted && self.vol == 0;
        if restore {
            self.vol = self.last_vol;
        }
        self.apply_volume();
        let patch = if restore {
            PrefsPatch {
                volume: Some(self.vol as f64 / 100.0),
                muted: Some(self.muted),
                ..PrefsPatch::default()
            }
        } else {
            PrefsPatch {
                muted: Some(self.muted),
                ..PrefsPatch::default()
            }
        };
        self.save_prefs(patch);
        self.changed();
    }

    /// Stumm-Taste: unter Autoplay-Sperre schaltet der erste Druck nur den Ton ein.
    pub fn toggle_mute(&mut self) {
        if self.disposed {
            return;
        }
        self.sync_now();
        if self.muted_by_policy {
            self.muted_by_policy = false;
            self.apply_volume();
            if !self.muted {
                self.changed();
                return;
            }
        }
        let s = self.silent();
        self.set_muted(!s);
    }

    // ------------------------------------------------------------------ Gesehen

    fn watch_tune(&mut self) {
        let Some(e) = self.entry.clone() else { return };
        let off = (self.cur - e.start).max(0);
        let now = self.cur;
        let ids = self
            .watch
            .as_mut()
            .map(|w| w.tune(&e, off, now))
            .unwrap_or_default();
        self.played(ids);
    }

    fn watch_leave(&mut self, silent: bool) {
        let now = self.cur;
        let ids = self
            .watch
            .as_mut()
            .map(|w| w.leave(silent, now))
            .unwrap_or_default();
        self.played(ids);
    }

    fn played(&mut self, ids: Vec<String>) {
        for id in ids {
            self.emit(TvOutput::SetPlayed { item_id: id });
        }
    }

    fn clear_stall(&mut self) {
        self.stall_armed = false;
        self.unlater(TimerKind::Stall);
    }

    // ------------------------------------------------------------------ Karten

    fn next_title(&mut self, e: &PlanEntry) -> (Option<PlanEntry>, Option<String>) {
        let nx = next_program(&mut self.plan, self.channel, e);
        let nt = nx
            .as_ref()
            .filter(|n| playable(n.kind))
            .map(|n| title_of(&self.plan, n).title);
        (nx, nt)
    }

    fn show_pause(&mut self, e: &PlanEntry) {
        let (nx, nt) = self.next_title(e);
        self.phase = Phase::Pause;
        self.reason = None;
        self.card = Some(match (nt, nx) {
            (Some(title), Some(nx)) => Card::Next {
                title,
                at: format_clock(nx.start),
                at_ms: nx.start,
            },
            _ => Card::Pause {
                text: "Gleich geht es weiter".into(),
            },
        });
        self.changed();
    }

    fn show_closedown(&mut self, e: &PlanEntry) {
        self.phase = Phase::Closedown;
        let empty = e.reason.as_deref() == Some("empty");
        self.reason = empty.then_some(Reason::Empty);
        if empty {
            self.card = Some(Card::Empty {
                text: "Auf diesem Sender läuft gerade nichts".into(),
            });
        } else {
            let nx = next_program(&mut self.plan, self.channel, e);
            let until = format_clock(nx.map_or(e.end, |n| n.start));
            self.card = Some(Card::Closedown {
                text: "Sendepause".into(),
                until,
            });
        }
        self.changed();
    }

    fn show_error(&mut self, why: FailReason, g: u64, code: Option<&str>, hint: Option<&str>) {
        self.playing = false;
        self.clear_stall();
        self.watch_leave(true); // Störbild/Ausfall: nie als gesehen werten
        let key = reason_of_fail(why);
        self.needs_tap = why == FailReason::Gesture;
        self.phase = Phase::Error;
        self.reason = Some(key);
        let shown = code
            .filter(|c| !c.is_empty())
            .map(str::to_string)
            .or_else(|| (key == Reason::Network).then(|| "NET".to_string()));
        let (title, sub) = error_texts(key);
        self.card = Some(Card::Error {
            reason: key,
            title: title.into(),
            sub: sub.into(),
            code: shown,
            hint: hint
                .filter(|h| !h.is_empty())
                .map(|h| format!("{h} wird von diesem Gerät nicht unterstützt")),
        });
        self.unlater(TimerKind::Retry);
        if key == Reason::Network {
            self.later(TimerKind::Retry, RETRY_MS, g);
        }
        if WARM_AFTER_ERROR.contains(&key.as_str()) {
            self.nb.arm(Mode::Error, self.flags, self.cur);
        }
        self.changed();
    }

    fn handle_result(&mut self, res: &PlayResult, g: u64, entry_seq: u64) -> bool {
        if g != self.generation {
            return false;
        }
        let is_cur = self.entry.is_some() && self.entry_seq == entry_seq;
        if !res.ok {
            if res.reason == Some(FailReason::Aborted) {
                return false;
            }
            let why = res.reason.unwrap_or(FailReason::Network);
            self.show_error(why, g, res.code.as_deref(), res.hint.as_deref());
            if is_cur {
                let key_ms = self.tune_at.map(|t| (self.cur - t).max(0) as f64);
                self.emit(TvOutput::Zap(Box::new(ZapInfo {
                    channel: Some(self.channel),
                    failed: true,
                    reason: Some(why.as_str().to_string()),
                    code: res.code.clone(),
                    key_ms,
                    timing: res.timing.clone(),
                    ..ZapInfo::default()
                })));
            }
            return false;
        }
        self.card = None;
        self.reason = None;
        self.phase = Phase::Playing;
        self.playing = true;
        // iOS kann `muted` beim src-Wechsel verlieren: nur in Richtung stumm absichern
        if self.silent() {
            self.emit(TvOutput::SetMuted(true));
        }
        if is_cur {
            self.watch_tune();
        }
        let key_ms = self.tune_at.map(|t| (self.cur - t).max(0));
        if let Some(ms) = key_ms {
            self.zaps.push(ZapRecord {
                channel: self.channel,
                ms,
                timing: res.timing.clone(),
            });
            if self.zaps.len() > STATS_MAX {
                self.zaps.remove(0);
            }
            self.tune_at = None;
        }
        if is_cur {
            self.emit(TvOutput::Zap(Box::new(ZapInfo {
                channel: Some(self.channel),
                key_ms: key_ms.map(|k| k as f64),
                timing: res.timing.clone(),
                ..ZapInfo::default()
            })));
            if self
                .entry
                .as_ref()
                .is_some_and(|e| e.kind == EntryType::Program)
            {
                self.nb.arm(Mode::Play, self.flags, self.cur);
            }
        }
        self.changed();
        true
    }

    // ------------------------------------------------------------------ Abspielen

    fn arm_advance(&mut self, e: &PlanEntry, g: u64) {
        let ms = clamp_i(e.end - self.cur + 30, 50, MAX_TIMEOUT);
        self.later(TimerKind::Advance, ms, g);
    }

    fn advance(&mut self, g: u64) {
        if g != self.generation || !self.on {
            return;
        }
        let t = self.cur;
        let n = match self.plan.now_on(self.channel, t) {
            Ok(n) => n,
            Err(_) => {
                self.later(TimerKind::Advance, 1000, g);
                return;
            }
        };
        if let Some(e) = self.entry.clone()
            && n.entry.start == e.start
            && t < e.end
        {
            self.arm_advance(&e, g);
            return;
        }
        self.generation += 1;
        let ng = self.generation;
        self.watch_leave(false);
        self.play_entry(n.entry, n.offset_ms, ng);
    }

    fn play_entry(&mut self, e: PlanEntry, offset_ms: i64, g: u64) {
        self.entry = Some(e.clone());
        self.entry_seq += 1;
        self.view = Some(e.clone());
        self.playing = false;
        self.trailer_on = false;
        self.unlater(TimerKind::Retry);
        self.unlater(TimerKind::Trailer);
        self.nb.stop();
        self.arm_advance(&e, g);
        self.needs_tap = false;
        self.clear_stall();
        self.watch_leave(true); // erst nach erfolgreichem Start wieder tune (handle_result)
        self.banner = match e.kind {
            EntryType::Fill => Some("Füllprogramm".to_string()),
            EntryType::Nightfill => Some(
                e.label
                    .clone()
                    .filter(|l| !l.is_empty())
                    .unwrap_or_else(|| "Füllprogramm – reguläres Programm ab 5:00".into()),
            ),
            _ => None,
        };
        if playable(e.kind)
            && let Some(id) = e.item_id.clone()
        {
            self.phase = Phase::Loading;
            self.reason = None;
            self.card = None;
            self.changed();
            let req = TvPlayRequest::new(id, offset_ms, e.start, e.end, Token(g));
            self.emit(TvOutput::Play(req));
            return;
        }
        if e.kind == EntryType::Filler {
            self.phase = Phase::Loading;
            self.reason = None;
            self.card = None;
            self.changed();
            self.run_filler(&e, g);
            return;
        }
        self.emit(TvOutput::StopPlayer);
        if matches!(e.kind, EntryType::Fill | EntryType::Nightfill) {
            self.show_pause(&e); // Füllprogramm ohne Titel
            return;
        }
        self.show_closedown(&e); // Sendepause (oder Eintrag ohne Id): kein Testbild, keine Musik
    }

    fn run_filler(&mut self, e: &PlanEntry, g: u64) {
        let next = self.plan.now_on(self.channel, e.end).ok().map(|n| n.entry);
        if let Some(item) = trailer_target(e, next.as_ref()) {
            self.emit(TvOutput::LoadTrailer {
                token: Token(g),
                item_id: item.to_string(),
            });
            return;
        }
        self.finish_filler(
            e,
            g,
            &FillerPick::Card {
                trailer_start_ms: None,
            },
        );
    }

    fn finish_filler(&mut self, e: &PlanEntry, g: u64, pick: &FillerPick) {
        if g != self.generation {
            return;
        }
        match pick {
            FillerPick::Trailer { url, offset_ms } => {
                self.emit(TvOutput::PlayUrl {
                    token: Token(g),
                    url: url.clone(),
                    offset_ms: *offset_ms,
                });
            }
            FillerPick::Card { trailer_start_ms } => {
                self.emit(TvOutput::StopPlayer);
                self.show_pause(e);
                if let Some(ts) = trailer_start_ms.filter(|ts| *ts > self.cur) {
                    let ms = clamp_i(ts - self.cur + 20, 20, MAX_TIMEOUT);
                    self.later(TimerKind::Trailer, ms, g);
                }
            }
        }
    }

    /// Antwort auf `LoadTrailer` (`None` = kein Trailer oder Ladefehler).
    pub fn on_trailer(&mut self, token: Token, trailer: Option<TrailerInfo>) {
        self.sync_now();
        if token.0 != self.generation || self.disposed {
            return;
        }
        let Some(e) = self.entry.clone() else { return };
        let pick = pick_filler(&e, trailer.as_ref(), self.cur);
        self.finish_filler(&e, token.0, &pick);
    }

    /// Antwort auf `PlayUrl`.
    pub fn on_play_url_result(&mut self, token: Token, res: &PlayResult) {
        self.sync_now();
        if token.0 != self.generation || self.disposed {
            return;
        }
        if res.ok {
            self.trailer_on = true;
            self.phase = Phase::Playing;
            self.reason = None;
            self.card = None;
            self.changed();
            return;
        }
        if let Some(e) = self.entry.clone() {
            self.emit(TvOutput::StopPlayer);
            self.show_pause(&e);
        }
    }

    fn start_gen(&mut self, g: u64, save: bool) {
        if g != self.generation || !self.on {
            return;
        }
        if save {
            let ch = self.channel;
            self.save_prefs(PrefsPatch {
                last_channel: Some(ch),
                ..PrefsPatch::default()
            });
        }
        match self.plan.now_on(self.channel, self.cur) {
            Ok(n) => self.play_entry(n.entry, n.offset_ms, g),
            Err(_) => self.show_error(FailReason::Network, g, None, None),
        }
    }

    /// Sender 1..18 wählen (Umlauf per Modulo). Entprellt 250 ms, `immediate` sofort. Gleicher
    /// Sender mit laufender Sendung: nichts. Nur wenn an.
    pub fn tune(&mut self, no: i64, immediate: bool) {
        if !self.on || self.disposed {
            return;
        }
        self.sync_now();
        self.tune_inner(no, immediate);
    }

    fn tune_inner(&mut self, no: i64, immediate: bool) {
        let no = (no - 1).rem_euclid(CHANNELS) + 1;
        if no == self.channel && self.entry.is_some() {
            return;
        }
        self.channel = no;
        self.generation += 1;
        let g = self.generation;
        for k in [
            TimerKind::Zap,
            TimerKind::Advance,
            TimerKind::Retry,
            TimerKind::Trailer,
        ] {
            self.unlater(k);
        }
        self.nb.stop();
        self.abort_prefetch();
        self.tune_at = Some(self.cur);
        self.clear_stall();
        self.needs_tap = false;
        self.watch_leave(false);
        if !immediate {
            self.emit(TvOutput::StopPlayer);
        }
        self.playing = false;
        self.trailer_on = false;
        self.entry = None;
        self.entry_seq += 1;
        self.view = self.plan.now_on(no, self.cur).ok().map(|n| n.entry);
        self.banner = None;
        self.card = None;
        self.reason = None;
        self.phase = Phase::Loading;
        self.changed();
        if immediate {
            self.start_gen(g, true);
        } else {
            // Schon während der Entprellung die PlaybackInfo des Ziels anstoßen (Folgedruck bricht ab).
            if let Some(v) = &self.view
                && let Some(id) = v.item_id.clone()
                && playable(v.kind)
            {
                self.pf_active = true;
                self.emit(TvOutput::PrefetchTarget { item_id: id });
            }
            self.later(TimerKind::Zap, DEBOUNCE_MS, g);
        }
    }

    fn abort_prefetch(&mut self) {
        if self.pf_active {
            self.pf_active = false;
            self.emit(TvOutput::CancelPrefetch);
        }
    }

    /// Sender weiter/zurück (`+1` hoch, `-1` runter).
    pub fn zap(&mut self, delta: i64) {
        self.tune(self.channel + delta, false);
    }

    // ------------------------------------------------------------------ Takt

    fn on_tick(&mut self) {
        if !self.on || self.disposed {
            return;
        }
        let t = self.cur;
        let ids = self.watch.as_mut().map(|w| w.tick(t)).unwrap_or_default();
        self.played(ids);
        self.tick_count += 1;
        if (self.tick_count).is_multiple_of(SYNC_MS as u64 / 1000)
            && self.playing
            && let Some(e) = &self.entry
            && playable(e.kind)
            && e.item_id.is_some()
        {
            let off = t - e.start;
            self.emit(TvOutput::SyncTo(off));
        }
        let d = broadcast_day_of(t);
        if let Some(last) = self.last_day
            && d != last
        {
            self.emit(TvOutput::PrepareDays((0..7).map(|i| d + i).collect()));
            self.emit(TvOutput::Day(d));
        }
        self.last_day = Some(d);
        self.emit(TvOutput::Tick(t));
    }

    // ------------------------------------------------------------------ Ein/Aus

    /// Einschalten (SYNCHRON in der Nutzergeste: das `Unlock` sofort ausführen). Idempotent.
    pub fn start(&mut self) {
        if self.disposed {
            return;
        }
        self.sync_now();
        self.emit(TvOutput::Unlock);
        if self.on {
            return;
        }
        self.on = true;
        self.tick_count = 0;
        self.last_day = Some(broadcast_day_of(self.cur));
        if !self.timers.iter().any(|t| t.kind == TimerKind::Tick) {
            self.seq += 1;
            self.timers.push(Timer {
                kind: TimerKind::Tick,
                at: self.cur + TICK_MS,
                seq: self.seq,
                generation: 0,
            });
        }
        self.apply_volume();
        let ch = self.channel;
        self.tune_inner(ch, true);
    }

    /// Ausschalten.
    pub fn stop(&mut self) {
        if !self.on {
            return;
        }
        self.sync_now();
        self.stop_inner();
    }

    fn stop_inner(&mut self) {
        self.generation += 1;
        for k in [
            TimerKind::Zap,
            TimerKind::Advance,
            TimerKind::Retry,
            TimerKind::Trailer,
            TimerKind::Tick,
        ] {
            self.unlater(k);
        }
        self.nb.stop();
        self.abort_prefetch();
        self.tune_at = None;
        self.clear_stall();
        self.watch_leave(false);
        self.emit(TvOutput::StopPlayer);
        self.on = false;
        self.playing = false;
        self.trailer_on = false;
        self.needs_tap = false;
        self.muted_by_policy = false;
        self.entry = None;
        self.entry_seq += 1;
        self.view = None;
        self.card = None;
        self.banner = None;
        self.reason = None;
        self.phase = Phase::Off;
        self.changed();
    }

    /// Nutzer hat getippt: `Unlock` (synchron ausführen), Ton an, bei Bedarf neuer Versuch.
    pub fn gesture(&mut self) {
        if self.disposed {
            return;
        }
        self.sync_now();
        self.emit(TvOutput::Unlock);
        if !self.on {
            return;
        }
        let retry = self.needs_tap;
        let policy = self.muted_by_policy;
        self.needs_tap = false;
        self.muted_by_policy = false;
        if retry || policy {
            self.apply_volume(); // Nutzer-Stumm-Zustand wiederherstellen
        }
        if retry {
            self.card = None;
            self.reason = None;
            self.phase = Phase::Loading;
            self.changed();
            self.generation += 1;
            let g = self.generation;
            self.start_gen(g, false);
        } else if policy {
            self.changed();
        }
    }

    /// Nach einer Fehlerkarte neu versuchen.
    pub fn retry(&mut self) {
        if !self.on || self.disposed {
            return;
        }
        self.sync_now();
        self.unlater(TimerKind::Retry);
        self.unlater(TimerKind::Trailer);
        self.clear_stall();
        self.card = None;
        self.reason = None;
        self.phase = Phase::Loading;
        self.changed();
        self.generation += 1;
        let g = self.generation;
        self.start_gen(g, false);
    }

    // ------------------------------------------------------------------ Ziffern

    /// Ziffer der Zifferneingabe (nur wenn an).
    pub fn push_digit(&mut self, d: char) {
        if !self.on || self.disposed {
            return;
        }
        self.sync_now();
        let ev = self.digits.push(d, self.cur);
        self.handle_digits(ev);
    }

    fn handle_digits(&mut self, ev: Vec<DigitsEvent>) {
        for e in ev {
            match e {
                DigitsEvent::Commit(no) => self.tune_inner(i64::from(no), true),
                DigitsEvent::Preview(t) => self.emit(TvOutput::Digits(t)),
                DigitsEvent::Invalid(t) => self.message(&t),
            }
        }
    }

    // ------------------------------------------------------------------ Ton und Untertitel

    /// Spuren des laufenden Streams (vom Player gemeldet).
    pub fn set_tracks(&mut self, t: Tracks) {
        self.tracks = t;
    }

    /// Spuren; ohne Player leer.
    pub fn tracks(&self) -> &Tracks {
        &self.tracks
    }

    /// Tonspur wählen. Ist sie schon aktiv: nichts. Antwort mit [`on_audio_result`](Self::on_audio_result).
    pub fn set_audio(&mut self, index: i32) {
        if self.tracks.audio_index == Some(index) || self.disposed {
            return;
        }
        let ticket = AudioTicket {
            generation: self.generation,
            entry_seq: self.entry_seq,
        };
        self.emit(TvOutput::SetAudio { ticket, index });
    }

    /// Antwort auf `SetAudio`. Im Ladefenster löst der Wechsel den laufenden Start ab; dann zählt
    /// dessen Ergebnis. Ist die Spur nicht auflösbar, bleibt das Bild (Hinweis, kein Fehler).
    pub fn on_audio_result(&mut self, ticket: AudioTicket, res: &PlayResult) {
        if self.disposed {
            return;
        }
        self.sync_now();
        if res.ok
            && self.phase == Phase::Loading
            && ticket.generation == self.generation
            && ticket.entry_seq == self.entry_seq
            && self.entry.is_some()
        {
            self.handle_result(res, ticket.generation, ticket.entry_seq);
        }
        if !res.ok && res.reason != Some(FailReason::Aborted) {
            self.message("Diese Tonspur ist nicht verfügbar");
        }
    }

    /// Untertitel wählen (`-1` = aus).
    pub fn set_subtitle(&mut self, index: i32) {
        self.emit(TvOutput::SetSubtitle(index));
    }

    /// Der Untertitelwechsel ist gescheitert.
    pub fn on_subtitle_failed(&mut self) {
        self.message("Untertitel nicht verfügbar");
    }

    /// Sendung der aktuellen Ansicht; `None` bei Pause, Sendeschluss, Füller, aus.
    pub fn restart_target(&self) -> Option<RestartTarget> {
        let e = self.view.as_ref()?;
        let id = e.item_id.as_ref()?;
        if !self.on || !playable(e.kind) {
            return None;
        }
        let it = self.plan.item(id);
        let series_id = e
            .series_id
            .clone()
            .filter(|s| !s.is_empty())
            .or_else(|| it.as_ref().and_then(|i| i.series_id.clone()));
        let kind = match &it {
            Some(i) => i.kind.as_str().to_string(),
            None if series_id.is_some() => "episode".to_string(),
            None => "movie".to_string(),
        };
        Some(RestartTarget {
            item_id: id.clone(),
            kind,
            series_id,
        })
    }

    // ------------------------------------------------------------------ Player-Ereignisse

    /// Antwort auf `Play`.
    pub fn on_play_result(&mut self, token: Token, res: &PlayResult) {
        if self.disposed {
            return;
        }
        self.sync_now();
        let seq = self.entry_seq;
        self.handle_result(res, token.0, seq);
    }

    /// Zustand des Players.
    pub fn on_player_state(&mut self, ph: PlayerPhase) {
        if !self.on || self.disposed {
            return;
        }
        self.sync_now();
        match ph {
            PlayerPhase::Stalled => {
                if !self.stall_armed && self.entry.is_some() {
                    self.stall_armed = true;
                    self.later(TimerKind::Stall, STALL_MS, 0); // Stillstand > 60 s: still verwerfen
                }
                if self.phase == Phase::Playing {
                    self.phase = Phase::Loading;
                    self.changed();
                }
            }
            PlayerPhase::Loading => {
                if self.phase == Phase::Playing {
                    self.phase = Phase::Loading;
                    self.changed();
                }
            }
            PlayerPhase::Playing => {
                if self.stall_armed {
                    self.clear_stall();
                }
                if self.playing {
                    self.watch_tune(); // idempotent; nach Stillstand wieder aufnehmen
                }
                if self.phase == Phase::Loading && (self.playing || self.trailer_on) {
                    self.phase = Phase::Playing;
                    self.changed();
                }
            }
            PlayerPhase::Error => {
                self.clear_stall();
                self.watch_leave(true);
            }
            PlayerPhase::Ended => {
                if let Some(e) = self.entry.clone()
                    && e.kind == EntryType::Filler
                    && self.card.is_none()
                {
                    self.show_pause(&e); // Trailer-Ende: Pausenkarte
                }
            }
        }
    }

    /// Autoplay-Sperre: Start nur stumm gelungen.
    pub fn on_autoplay_muted(&mut self) {
        if !self.on || self.disposed {
            return;
        }
        self.sync_now();
        self.muted_by_policy = true;
        if !self.muted {
            self.message("Ton aus – zum Einschalten tippen");
        }
        self.changed();
    }

    /// Fehler des Players während der Wiedergabe.
    pub fn on_player_error(&mut self, reason: FailReason, code: Option<&str>, hint: Option<&str>) {
        if !self.on || self.disposed || self.phase == Phase::Error {
            return;
        }
        let Some(e) = &self.entry else { return };
        if e.kind == EntryType::Filler {
            return;
        }
        self.sync_now();
        let g = self.generation;
        self.show_error(reason, g, code, hint);
    }

    // ------------------------------------------------------------------ Messung, Aufräumen

    /// Messung: letzte Umschaltungen (Taste bis Bild).
    pub fn stats(&self) -> Stats {
        let ms: Vec<i64> = self.zaps.iter().map(|z| z.ms).collect();
        Stats {
            zaps: self.zaps.clone(),
            last_ms: ms.last().copied(),
            avg_ms: (!ms.is_empty())
                .then(|| round_js(ms.iter().sum::<i64>() as f64 / ms.len() as f64) as i64),
        }
    }

    /// Beendet alles; danach tut der Controller nichts mehr.
    pub fn dispose(&mut self) {
        if self.disposed {
            return;
        }
        self.sync_now();
        self.stop_inner_if_on();
        self.disposed = true;
        self.abort_prefetch();
        self.nb.stop();
        self.timers.clear();
        let _ = self.digits.dispose();
        if let Some(w) = self.watch.as_mut() {
            w.dispose();
        }
    }

    fn stop_inner_if_on(&mut self) {
        if self.on {
            self.stop_inner();
        }
    }
}
