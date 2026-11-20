//! Schnittstelle zur Wiedergabe der App: [`PlayerControl`].
//!
//! Die Crate kennt keinen Player. Die App setzt [`PlayerControl`] um (z. B. über `lfx-player`); alle Fernbefehle
//! und SyncPlay-Korrekturen kommen als [`PlayerAction`], der Zustand wird per [`PlayerSnapshot`] abgefragt.

use crate::enums::PlayCommand;
use crate::enums::PlaystateCommand;
use crate::message::{GeneralCommand, PlayRequest, PlaystateRequest, TypedCommand};
use std::sync::Mutex;

/// Momentaufnahme des Players.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PlayerSnapshot {
    /// Laufendes Element (Jellyfin-Id), `None` = nichts geladen.
    pub item_id: Option<String>,
    /// Position in Ticks (100 ns).
    pub position_ticks: i64,
    /// `true` = läuft (nicht pausiert).
    pub playing: bool,
    /// `true` = puffert gerade / Medium noch nicht bereit.
    pub buffering: bool,
    /// Aktuelle Wiedergabegeschwindigkeit (1.0 = normal).
    pub rate: f64,
}

/// Aktion, die der Player ausführen soll.
#[derive(Debug, Clone, PartialEq)]
pub enum PlayerAction {
    /// Wiedergabe starten oder einreihen.
    Play {
        /// Elemente.
        item_ids: Vec<String>,
        /// Index des ersten zu spielenden Elements in `item_ids`.
        start_index: Option<i32>,
        /// Startposition in Ticks.
        start_position_ticks: Option<i64>,
        /// Art (jetzt, als Nächstes, ans Ende …).
        command: PlayCommand,
        /// Medienquelle.
        media_source_id: Option<String>,
        /// Tonspur.
        audio_stream_index: Option<i32>,
        /// Untertitelspur.
        subtitle_stream_index: Option<i32>,
    },
    /// Pausieren.
    Pause,
    /// Fortsetzen.
    Unpause,
    /// Pause/Fortsetzen umschalten.
    PlayPause,
    /// Stoppen.
    Stop,
    /// Zur Position springen.
    SeekTicks(i64),
    /// Zurückspulen.
    Rewind,
    /// Vorspulen.
    FastForward,
    /// Nächster Titel.
    NextTrack,
    /// Vorheriger Titel.
    PreviousTrack,
    /// Geschwindigkeit setzen (nur SyncPlay-Drift-Korrektur).
    SetRate(f64),
    /// Lautstärke 0 bis 100.
    SetVolume(u8),
    /// Stumm schalten (`true`) oder Ton an.
    SetMuted(bool),
    /// Stummschaltung umschalten.
    ToggleMute,
    /// Tonspur.
    SetAudioStreamIndex(i32),
    /// Untertitelspur (`-1` = aus).
    SetSubtitleStreamIndex(i32),
    /// Vollbild umschalten.
    ToggleFullscreen,
    /// Zur Startseite (UI).
    GoHome,
    /// Meldung anzeigen (UI).
    DisplayMessage {
        /// Titel.
        header: Option<String>,
        /// Text.
        text: String,
        /// Dauer in ms.
        timeout_ms: Option<i64>,
    },
    /// Element anzeigen (UI).
    DisplayContent {
        /// Element-Id.
        item_id: String,
        /// Elementart.
        item_type: Option<String>,
    },
    /// Sonstiger allgemeiner Befehl, den die App selbst deuten kann (Navigation u. a.).
    General(GeneralCommand),
}

impl PlayerAction {
    /// Aus einer Play-Nachricht.
    pub fn from_play(r: &PlayRequest) -> Self {
        Self::Play {
            item_ids: r.item_ids.clone(),
            start_index: r.start_index,
            start_position_ticks: r.start_position_ticks,
            command: r.play_command.clone().unwrap_or(PlayCommand::PlayNow),
            media_source_id: r.media_source_id.clone(),
            audio_stream_index: r.audio_stream_index,
            subtitle_stream_index: r.subtitle_stream_index,
        }
    }

    /// Aus einer Playstate-Nachricht; `None` bei fehlendem oder unbekanntem Befehl.
    pub fn from_playstate(r: &PlaystateRequest) -> Option<Self> {
        Some(match r.command.as_ref()? {
            PlaystateCommand::Stop => Self::Stop,
            PlaystateCommand::Pause => Self::Pause,
            PlaystateCommand::Unpause => Self::Unpause,
            PlaystateCommand::PlayPause => Self::PlayPause,
            PlaystateCommand::NextTrack => Self::NextTrack,
            PlaystateCommand::PreviousTrack => Self::PreviousTrack,
            PlaystateCommand::Seek => Self::SeekTicks(r.seek_position_ticks?),
            PlaystateCommand::Rewind => Self::Rewind,
            PlaystateCommand::FastForward => Self::FastForward,
            PlaystateCommand::Unknown(_) => return None,
        })
    }

    /// Aus einem allgemeinen Befehl.
    pub fn from_general(g: &GeneralCommand) -> Self {
        match g.typed() {
            TypedCommand::DisplayMessage {
                header,
                text,
                timeout_ms,
            } => Self::DisplayMessage {
                header,
                text,
                timeout_ms,
            },
            TypedCommand::SetVolume(v) => Self::SetVolume(v),
            TypedCommand::Mute => Self::SetMuted(true),
            TypedCommand::Unmute => Self::SetMuted(false),
            TypedCommand::ToggleMute => Self::ToggleMute,
            TypedCommand::SetAudioStreamIndex(i) => Self::SetAudioStreamIndex(i),
            TypedCommand::SetSubtitleStreamIndex(i) => Self::SetSubtitleStreamIndex(i),
            TypedCommand::ToggleFullscreen => Self::ToggleFullscreen,
            TypedCommand::GoHome => Self::GoHome,
            TypedCommand::DisplayContent {
                item_id, item_type, ..
            } => Self::DisplayContent { item_id, item_type },
            TypedCommand::Other { .. } => Self::General(g.clone()),
        }
    }
}

/// Vom Player der App umzusetzen. Aufrufe kommen aus dem Laufzeit-Task und müssen schnell zurückkehren
/// (nicht blockieren, ggf. in die UI-Schleife weiterreichen).
pub trait PlayerControl: Send + Sync + 'static {
    /// Führt eine Aktion aus.
    fn apply(&self, action: PlayerAction);
    /// Aktueller Zustand (für Drift-Korrektur und Ready/Buffering-Meldungen).
    fn snapshot(&self) -> PlayerSnapshot;
}

/// Player, der nichts tut (Wiedergabe nicht verfügbar).
#[derive(Debug, Clone, Copy, Default)]
pub struct NullPlayer;

impl PlayerControl for NullPlayer {
    fn apply(&self, _action: PlayerAction) {}
    fn snapshot(&self) -> PlayerSnapshot {
        PlayerSnapshot {
            rate: 1.0,
            ..PlayerSnapshot::default()
        }
    }
}

/// Zeichnet Aktionen auf und liefert einen einstellbaren Zustand (Tests, Entwicklung ohne Player).
#[derive(Debug, Default)]
pub struct RecordingPlayer {
    actions: Mutex<Vec<PlayerAction>>,
    state: Mutex<PlayerSnapshot>,
}

impl RecordingPlayer {
    /// Neu mit Normalgeschwindigkeit.
    pub fn new() -> Self {
        let p = Self::default();
        p.state.lock().unwrap().rate = 1.0;
        p
    }
    /// Alle bisherigen Aktionen.
    pub fn actions(&self) -> Vec<PlayerAction> {
        self.actions.lock().unwrap().clone()
    }
    /// Leert die Aufzeichnung.
    pub fn clear(&self) {
        self.actions.lock().unwrap().clear();
    }
    /// Setzt den gemeldeten Zustand.
    pub fn set_state(&self, state: PlayerSnapshot) {
        *self.state.lock().unwrap() = state;
    }
}

impl PlayerControl for RecordingPlayer {
    fn apply(&self, action: PlayerAction) {
        self.actions.lock().unwrap().push(action);
    }
    fn snapshot(&self) -> PlayerSnapshot {
        self.state.lock().unwrap().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{ServerMessage, parse_message};

    fn msg(json: &str) -> ServerMessage {
        parse_message(json).unwrap().message
    }

    #[test]
    fn mapping() {
        let ServerMessage::Playstate(p) =
            msg(r#"{"MessageType":"Playstate","Data":{"Command":"Seek","SeekPositionTicks":7}}"#)
        else {
            panic!()
        };
        assert_eq!(
            PlayerAction::from_playstate(&p),
            Some(PlayerAction::SeekTicks(7))
        );
        let ServerMessage::Playstate(p) =
            msg(r#"{"MessageType":"Playstate","Data":{"Command":"Seek"}}"#)
        else {
            panic!()
        };
        assert_eq!(PlayerAction::from_playstate(&p), None);
        let ServerMessage::GeneralCommand(g) =
            msg(r#"{"MessageType":"GeneralCommand","Data":{"Name":"Unmute"}}"#)
        else {
            panic!()
        };
        assert_eq!(
            PlayerAction::from_general(&g),
            PlayerAction::SetMuted(false)
        );
        let ServerMessage::Play(p) =
            msg(r#"{"MessageType":"Play","Data":{"ItemIds":["a"],"StartPositionTicks":5}}"#)
        else {
            panic!()
        };
        let PlayerAction::Play {
            command,
            start_position_ticks,
            ..
        } = PlayerAction::from_play(&p)
        else {
            panic!()
        };
        assert_eq!(command, PlayCommand::PlayNow);
        assert_eq!(start_position_ticks, Some(5));
    }
}
