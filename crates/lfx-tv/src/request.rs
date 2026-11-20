//! Datentypen zwischen TV-Controller und Player: Wiedergabeauftrag, Ergebnis, Messung.
//!
//! Der Auftrag trägt weder Adressen noch Schlüssel; der Player löst Stream und Zugang selbst auf.
//! Der TV-Modus meldet nie Fortschritt (`report_progress` ist immer `false`, keine Spuren in
//! „Weiterschauen“).

use crate::stream::FailReason;

/// Kennung eines Starts (Generationszähler). Antworten mit veralteter Kennung werden verworfen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Token(pub u64);

/// Auftrag an den Player: eine Sendung ab der Live-Position starten.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TvPlayRequest {
    /// Jellyfin-Element.
    pub item_id: String,
    /// Startposition in der Sendung (jetzt minus Beginn) zum Zeitpunkt des Auftrags.
    pub offset_ms: i64,
    /// Beginn der Sendung (UTC, ms). Dauert Auflösung/Rückfall, rechnet der Player die Position
    /// aus `now - entry_start` neu.
    pub entry_start: i64,
    /// Ende der Sendung.
    pub entry_end: i64,
    /// Gewünschte Tonspur (Jellyfin-Index), sonst Wahl nach Nutzerwunsch.
    pub audio_index: Option<i32>,
    /// Gewünschter Untertitel; `Some(-1)` = aus.
    pub subtitle_index: Option<i32>,
    /// Fortschritt melden? Im TV-Modus immer `false`.
    pub report_progress: bool,
    /// Kennung für die Antwort.
    pub token: Token,
}

impl TvPlayRequest {
    /// Auftrag für eine Sendung; Tonspur und Untertitel nach Voreinstellung.
    pub fn new(
        item_id: impl Into<String>,
        offset_ms: i64,
        entry_start: i64,
        entry_end: i64,
        token: Token,
    ) -> Self {
        Self {
            item_id: item_id.into(),
            offset_ms: offset_ms.max(0),
            entry_start,
            entry_end,
            audio_index: None,
            subtitle_index: None,
            report_progress: false,
            token,
        }
    }

    /// Startposition in Sekunden.
    pub fn start_seconds(&self) -> f64 {
        self.offset_ms as f64 / 1000.0
    }

    /// Live-Position in Sekunden zum Zeitpunkt `now` (für verspäteten Start nach Rückfall).
    pub fn live_seconds_at(&self, now: i64) -> f64 {
        ((now - self.entry_start).max(0)) as f64 / 1000.0
    }
}

/// Quelle einer Wiedergabe (nur Kürzel, nie Adressen).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SrcInfo {
    pub container: String,
    pub v: String,
    pub a: String,
    pub fmt: String,
    pub reasons: Vec<String>,
}

/// Messung des Vorwärmens.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WarmInfo {
    pub on: bool,
    pub used: bool,
    pub partial: bool,
    pub skip: Option<String>,
    pub started: Option<u32>,
    pub done: Option<u32>,
    pub age_ms: Option<f64>,
    pub playlist_ms: Option<f64>,
    pub init_ms: Option<f64>,
    pub seg_first_byte_ms: Option<f64>,
}

/// Messung der Ressourcen (Anzahl Abrufe, Zeit bis zum ersten Segment).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResInfo {
    pub n: u32,
    pub first_seg_ms: Option<f64>,
}

/// Messung eines Starts (siehe Messanzeige).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Timing {
    pub total_ms: f64,
    pub info_ms: Option<f64>,
    pub load_ms: Option<f64>,
    pub start_ms: Option<f64>,
    /// Versuchskette, z. B. `D>T`.
    pub chain: String,
    pub cached: bool,
    pub mode: String,
    pub hint: Option<String>,
    /// Nach `loadedmetadata`/Laden gesprungen? `None` = Messfelder nicht vorhanden.
    pub jumped: Option<bool>,
    pub forced: bool,
    pub dev_s: Option<f64>,
    pub seeking_ms: Option<f64>,
    pub seeked_ms: Option<f64>,
    pub canplay_ms: Option<f64>,
    pub buf_seeked: Option<[f64; 2]>,
    pub buf_playing: Option<[f64; 2]>,
    pub wait_count: u32,
    pub wait_ms: f64,
    pub src: Option<SrcInfo>,
    pub warm: Option<WarmInfo>,
    pub res: Option<ResInfo>,
}

/// Ergebnis eines Starts (`play`, `play_url`, Tonwechsel).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PlayResult {
    pub ok: bool,
    pub reason: Option<FailReason>,
    pub code: Option<String>,
    pub hint: Option<String>,
    pub timing: Option<Timing>,
}

impl PlayResult {
    /// Erfolg.
    pub fn ok() -> Self {
        Self {
            ok: true,
            ..Self::default()
        }
    }

    /// Fehler mit Grund.
    pub fn fail(reason: FailReason) -> Self {
        Self {
            ok: false,
            reason: Some(reason),
            ..Self::default()
        }
    }

    /// Fehler mit Grund und Code.
    pub fn fail_code(reason: FailReason, code: &str) -> Self {
        Self {
            ok: false,
            reason: Some(reason),
            code: Some(code.to_string()),
            ..Self::default()
        }
    }
}

/// Phase des Players.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerPhase {
    Loading,
    Playing,
    Stalled,
    Error,
    Ended,
}

/// Lautstärke-Modus des Geräts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VolumeMode {
    /// Pegel einstellbar.
    #[default]
    Native,
    /// Nur stumm/laut (z. B. iOS).
    MuteOnly,
}

impl VolumeMode {
    /// Name wie im Web.
    pub fn as_str(self) -> &'static str {
        match self {
            VolumeMode::Native => "native",
            VolumeMode::MuteOnly => "mute-only",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auftrag_meldet_nie_fortschritt() {
        let r = TvPlayRequest::new("film1", 90_000, 1_000_000, 7_000_000, Token(3));
        assert!(!r.report_progress);
        assert_eq!(r.start_seconds(), 90.0);
        assert_eq!(r.live_seconds_at(1_100_000), 100.0);
        assert_eq!(r.live_seconds_at(0), 0.0);
        assert_eq!(TvPlayRequest::new("x", -5, 0, 1, Token(1)).offset_ms, 0);
    }

    #[test]
    fn ergebnis_konstruktoren() {
        assert!(PlayResult::ok().ok);
        let f = PlayResult::fail_code(FailReason::Video, "V-CODEC");
        assert!(!f.ok);
        assert_eq!(f.code.as_deref(), Some("V-CODEC"));
    }
}
