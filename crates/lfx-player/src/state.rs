//! Zustand des Players (per `watch` veröffentlicht) und Ereignisse (per `broadcast`).

use std::fmt;

/// Art einer mpv-Spur.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackKind {
    Video,
    Audio,
    Subtitle,
}

/// Eine Spur aus mpv (`track-list`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Track {
    /// mpv-Spur-Id (1-basiert je Art), Wert für `aid`/`sid`.
    pub id: i64,
    pub kind: TrackKind,
    /// Sprache (ISO 639), falls angegeben.
    pub lang: Option<String>,
    pub title: Option<String>,
    pub codec: Option<String>,
    pub default: bool,
    pub forced: bool,
    pub external: bool,
    /// Aktuell ausgewählt.
    pub selected: bool,
}

/// Kapitel.
#[derive(Debug, Clone, PartialEq)]
pub struct ChapterInfo {
    pub title: Option<String>,
    /// Beginn in Sekunden.
    pub start: f64,
}

/// Videoparameter nach dem Decoder (`video-params`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VideoParams {
    pub width: i64,
    pub height: i64,
    /// Farbprimärwerte (`bt.709`, `bt.2020`, ...).
    pub primaries: Option<String>,
    /// Transfer (`bt.1886`, `pq`, `hlg`, ...). Achtung: mpv meldet bei manchen PQ-Clips `bt.1886`;
    /// HDR daher zusätzlich über `sig_peak` erkennen ([`VideoParams::is_hdr`]).
    pub gamma: Option<String>,
    /// Signalspitze (relativ zu SDR-Weiß = 1,0).
    pub sig_peak: Option<f64>,
    /// MaxCLL in Nits (aus den Metadaten).
    pub max_cll: Option<f64>,
    /// MaxFALL in Nits.
    pub max_fall: Option<f64>,
}

impl VideoParams {
    /// Ist der Inhalt HDR (PQ/HLG oder Signalspitze über SDR-Weiß)?
    pub fn is_hdr(&self) -> bool {
        let g = self.gamma.as_deref().unwrap_or("");
        matches!(g, "pq" | "hlg" | "st2084" | "arib-std-b67")
            || self.sig_peak.is_some_and(|p| p > 1.0 + 1e-3)
            || self.max_cll.is_some_and(|c| c > 203.0)
    }
}

/// Fehler der Wiedergabe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerError {
    /// mpv-Fehlercode (0 = keiner bekannt).
    pub code: i32,
    /// Lesbarer Text (nie mit Token/Adresse).
    pub text: String,
}

impl fmt::Display for PlayerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (mpv {})", self.text, self.code)
    }
}

impl std::error::Error for PlayerError {}

/// Gesamtzustand. Wird bei jeder Änderung neu veröffentlicht.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerState {
    /// Zählt jedes `open`; Ereignisse und Zustand lassen sich so einer Wiedergabe zuordnen.
    pub generation: u64,
    /// Datei geladen (seit `FileLoaded`, bis zum Ende/Stop).
    pub loaded: bool,
    /// Position in Sekunden.
    pub position: f64,
    /// Dauer, sofern bekannt.
    pub duration: Option<f64>,
    pub paused: bool,
    /// Puffert gerade (`paused-for-cache`).
    pub buffering: bool,
    /// Ende erreicht (`eof-reached`, bei `keep-open`).
    pub eof: bool,
    pub tracks: Vec<Track>,
    pub chapters: Vec<ChapterInfo>,
    pub video: VideoParams,
    /// Aktiver Hardware-Decoder (`hwdec-current`), z. B. `d3d11va`; `None` = Software.
    pub hwdec: Option<String>,
    /// Lautstärke (mpv-Skala, 100 = Normal).
    pub volume: f64,
    pub muted: bool,
    pub speed: f64,
    /// Letzter Fehler (beim nächsten `open` zurückgesetzt).
    pub error: Option<PlayerError>,
}

impl Default for PlayerState {
    fn default() -> Self {
        Self {
            generation: 0,
            loaded: false,
            position: 0.0,
            duration: None,
            paused: false,
            buffering: false,
            eof: false,
            tracks: Vec::new(),
            chapters: Vec::new(),
            video: VideoParams::default(),
            hwdec: None,
            volume: 100.0,
            muted: false,
            speed: 1.0,
            error: None,
        }
    }
}

impl PlayerState {
    pub fn audio_tracks(&self) -> impl Iterator<Item = &Track> {
        self.tracks.iter().filter(|t| t.kind == TrackKind::Audio)
    }

    pub fn subtitle_tracks(&self) -> impl Iterator<Item = &Track> {
        self.tracks.iter().filter(|t| t.kind == TrackKind::Subtitle)
    }

    /// Gewählte Spur einer Art.
    pub fn selected(&self, kind: TrackKind) -> Option<&Track> {
        self.tracks.iter().find(|t| t.kind == kind && t.selected)
    }
}

/// Ende einer Wiedergabe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EndKind {
    /// Natürliches Ende (nur ohne `keep-open`; mit `keep-open` kommt [`PlayerEvent::Eof`]).
    Eof,
    /// Durch `stop` oder ein neues `open` beendet.
    Stop,
    /// Fehler beim Laden/Abspielen.
    Error(PlayerError),
    /// mpv wird beendet.
    Quit,
}

/// Ereignisse für Fortschrittsmelder und UI. Jedes trägt die Generation des `open`.
#[derive(Debug, Clone, PartialEq)]
pub enum PlayerEvent {
    /// `open` wurde verarbeitet (vor dem Laden).
    Opened {
        generation: u64,
        item_id: String,
        media_source_id: String,
    },
    /// Datei geladen, Wiedergabe beginnt.
    FileLoaded {
        generation: u64,
    },
    PauseChanged {
        generation: u64,
        paused: bool,
    },
    /// Springen abgeschlossen (neue Position im Zustand).
    Seeked {
        generation: u64,
    },
    /// Ton-/Untertitelspur wurde gewechselt.
    TrackChanged {
        generation: u64,
    },
    /// Ende des Inhalts erreicht (`eof-reached`, Datei bleibt wegen `keep-open` stehen).
    Eof {
        generation: u64,
    },
    /// Wiedergabe beendet.
    Ended {
        generation: u64,
        kind: EndKind,
    },
}

impl PlayerEvent {
    /// Generation des Ereignisses.
    pub fn generation(&self) -> u64 {
        match self {
            Self::Opened { generation, .. }
            | Self::FileLoaded { generation }
            | Self::PauseChanged { generation, .. }
            | Self::Seeked { generation }
            | Self::TrackChanged { generation }
            | Self::Eof { generation }
            | Self::Ended { generation, .. } => *generation,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hdr_detection_uses_peak_when_gamma_lies() {
        let mut v = VideoParams {
            gamma: Some("bt.1886".into()),
            ..Default::default()
        };
        assert!(!v.is_hdr());
        v.sig_peak = Some(4.93);
        assert!(v.is_hdr());
        let hlg = VideoParams {
            gamma: Some("hlg".into()),
            ..Default::default()
        };
        assert!(hlg.is_hdr());
    }
}
