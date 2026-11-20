//! W2-tvui: TV-Modus-Seite (Route `TvMode`) – Zustand und Hilfen. Die Abläufe stehen in `tvui_host.rs`.
//!
//! Die Seite verbindet `lfx-tv` (Sans-IO-Controller, Senderliste, Programm-Raster, Info-Karte) mit dem eigenen
//! Player (`lfx-player`, ohne Fortschrittsmeldung) und der Video-Ebene der Shell. Alles im UI-Thread (der Planer
//! hält `Rc`). Es gibt kein Bild-in-Bild am Desktop: Verlassen der Seite stoppt die Wiedergabe sofort.

use std::rc::Rc;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use lfx_player::{Player, PrepareError};
use lfx_shell::Rect;
use lfx_tv::catalog::MemoryCatalogCache;
use lfx_tv::channel_list::ChannelList;
use lfx_tv::clock::Clock;
use lfx_tv::controller::{Card, Phase, Reason, Snapshot, TvController};
use lfx_tv::epg::EpgGrid;
use lfx_tv::info_card::{Details, InfoCard};
use lfx_tv::request::{PlayResult, PlayerPhase, Token};
use lfx_tv::stream::FailReason;
use lfx_tv_schedule::Scheduler;
use lfx_ui::TvChanRow;
use slint::{Timer, VecModel};

use crate::video::VideoPipe;

/// Uhr des TV-Modus: Systemzeit, in der Entwicklung (`LFX_TV_NOW`) ab einem festen Zeitpunkt laufend.
#[derive(Clone, Copy)]
pub(crate) struct TvClock {
    fixed: Option<i64>,
    started: Instant,
}

impl TvClock {
    pub(crate) fn new() -> Self {
        let fixed = if cfg!(feature = "dev-automation") {
            std::env::var("LFX_TV_NOW").ok().and_then(|s| parse_utc(&s))
        } else {
            None
        };
        Self {
            fixed,
            started: Instant::now(),
        }
    }
}

impl Clock for TvClock {
    fn now_ms(&self) -> i64 {
        match self.fixed {
            Some(t) => t + self.started.elapsed().as_millis() as i64,
            None => lfx_tv::SystemClock.now_ms(),
        }
    }
}

/// `YYYY-MM-DDTHH:MM` (UTC) in ms; nur für die Entwicklung.
pub(crate) fn parse_utc(s: &str) -> Option<i64> {
    let (date, time) = s.trim_end_matches('Z').split_once('T')?;
    let mut d = date.split('-').map(|p| p.parse::<i64>().ok());
    let (y, m, dd) = (d.next()??, d.next()??, d.next()??);
    let mut t = time.split(':').map(|p| p.parse::<i64>().ok());
    let (h, mi) = (t.next()??, t.next()??);
    Some(lfx_tv_schedule::time::days_from_civil(y, m, dd) * 86_400_000 + (h * 60 + mi) * 60_000)
}

/// Katalog-Cache im Speicher (je Benutzer und Sendetag), lebt über Besuche hinweg.
pub(crate) fn catalog_cache() -> &'static Mutex<MemoryCatalogCache> {
    static CACHE: OnceLock<Mutex<MemoryCatalogCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(MemoryCatalogCache::new()))
}

/// Zustand der TV-Modus-Seite (nur UI-Thread).
#[derive(Default)]
pub struct TvUi {
    pub active: bool,
    /// Zählt Besuche; späte Antworten alter Besuche werden verworfen.
    pub(crate) visit: u64,
    pub(crate) ctl: Option<TvController<Scheduler>>,
    pub(crate) clock: Option<TvClock>,
    pub(crate) list: Option<ChannelList>,
    pub(crate) list_model: Option<Rc<VecModel<TvChanRow>>>,
    pub(crate) grid: Option<EpgGrid>,
    pub(crate) card: Option<InfoCard>,
    pub(crate) details: Option<Details>,
    pub(crate) snap: Option<Snapshot>,
    pub(crate) last_channel: i64,
    pub(crate) tab: i32,
    pub(crate) fullscreen: bool,
    pub(crate) rect: Option<Rect>,
    pub(crate) player: Option<Player>,
    pub(crate) pipe: Option<VideoPipe>,
    /// Letzter Startwunsch des Controllers (Antworten mit anderer Kennung sind veraltet).
    pub(crate) want: Option<Token>,
    /// Wartet auf das Ergebnis dieses Starts; Wert = Player-Generation vor `open`.
    pub(crate) pending: Option<(Token, u64)>,
    /// Generation des laufenden Starts (für Rückmeldungen im Betrieb).
    pub(crate) playing_gen: Option<u64>,
    pub(crate) last_phase: Option<PlayerPhase>,
    pub(crate) play_session: Option<String>,
    pub(crate) plays: u64,
    pub(crate) last_epg_ms: i64,
    pub(crate) poll: Timer,
    pub(crate) host: Timer,
    pub(crate) zap_hide: Timer,
    pub(crate) controls_hide: Timer,
    pub(crate) toast_hide: Timer,
}

/// Ergebnis eines gescheiterten `prepare` als Antwort an den Controller (Codes ohne Adressen und Schlüssel).
pub(crate) fn fail_result(e: &PrepareError) -> PlayResult {
    match e {
        PrepareError::Api(_) => PlayResult::fail_code(FailReason::Network, "A1"),
        PrepareError::NoPlayableSource(_) => PlayResult::fail_code(FailReason::Video, "V1"),
    }
}

/// Anzeigewerte der Info unter dem Player und der Zapp-Einblendung.
pub(crate) struct InfoView {
    pub has: bool,
    pub no: i32,
    pub color: slint::Color,
    pub ink: slint::Color,
    pub short: String,
    pub title: String,
    pub sub: String,
    pub time: String,
    pub badges: Vec<String>,
    pub next: String,
    pub progress: f32,
}

pub(crate) fn phase_index(p: Phase) -> i32 {
    match p {
        Phase::Off => 0,
        Phase::Loading => 1,
        Phase::Playing => 2,
        Phase::Pause => 3,
        Phase::Closedown => 4,
        Phase::Error => 5,
    }
}

/// Zustandskarte im Player: Art (0 keine, 1 Gleich, 2 Pause, 3 Sendepause, 4 leer, 5 Fehler) und Texte.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct CardTexts {
    pub kind: i32,
    pub title: String,
    pub text: String,
    pub hint: String,
    pub code: String,
    pub button: String,
}

pub(crate) fn card_texts(snap: &Snapshot) -> CardTexts {
    let Some(card) = &snap.card else {
        return CardTexts::default();
    };
    let or = |s: &str, d: &str| {
        if s.is_empty() {
            d.to_owned()
        } else {
            s.to_owned()
        }
    };
    match card {
        Card::Next { title, at, .. } => CardTexts {
            kind: 1,
            title: format!("Gleich: {title} · {at}"),
            ..CardTexts::default()
        },
        Card::Pause { text } => CardTexts {
            kind: 2,
            title: or(text, "Gleich geht es weiter"),
            ..CardTexts::default()
        },
        Card::Closedown { text, until } => CardTexts {
            kind: 3,
            title: or(text, "Sendepause"),
            text: if until.is_empty() {
                String::new()
            } else {
                format!("Weiter um {until}")
            },
            ..CardTexts::default()
        },
        Card::Empty { text } => CardTexts {
            kind: 4,
            title: or(text, "Auf diesem Sender läuft gerade nichts"),
            ..CardTexts::default()
        },
        Card::Error {
            reason,
            title,
            sub,
            code,
            hint,
        } => CardTexts {
            kind: 5,
            title: or(title, "Fehler"),
            text: sub.clone(),
            hint: hint.clone().unwrap_or_default(),
            code: code
                .as_ref()
                .map(|c| format!("Code: {c}"))
                .unwrap_or_default(),
            button: if *reason == Reason::Gesture || snap.needs_tap {
                "Zum Starten tippen".into()
            } else {
                "Erneut versuchen".into()
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev_zeit_wird_gelesen() {
        // 2026-11-20 20:15 UTC
        let t = parse_utc("2026-11-20T20:15").expect("gültig");
        assert_eq!(t % 86_400_000, (20 * 60 + 15) * 60_000);
        assert!(parse_utc("kein Datum").is_none());
    }

    #[test]
    fn karte_ohne_inhalt_ist_leer() {
        let snap = Snapshot {
            on: true,
            channel: 1,
            entry: None,
            phase: Phase::Playing,
            reason: None,
            card: Some(Card::Closedown {
                text: String::new(),
                until: "05:00".into(),
            }),
            banner: None,
            needs_tap: false,
            muted_by_policy: false,
            volume: 100,
            muted: false,
        };
        let c = card_texts(&snap);
        assert_eq!(c.kind, 3);
        assert_eq!(c.title, "Sendepause");
        assert_eq!(c.text, "Weiter um 05:00");
    }
}
