//! Typisierte Nachrichten des Jellyfin-Session-WebSockets (`/socket`).
//!
//! Umschlag: `{"MessageType": "...", "MessageId": "...", "Data": ...}`. Bekannte Typen werden nach der
//! OpenAPI-Spezifikation geparst, alle übrigen landen unverändert in [`ServerMessage::Unknown`].

use crate::enums::{
    GeneralCommandName, GroupStateType, PlayCommand, PlayQueueUpdateReason, PlaybackRequestType,
    PlaystateCommand, SendCommandType,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// Fehler beim Parsen einer Nachricht. Enthält nie den Nachrichteninhalt (könnte Namen/Ids tragen).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    /// Kein gültiges JSON oder kein Umschlag.
    #[error("Nachricht ist kein gültiger Umschlag")]
    Envelope,
    /// `Data` passt nicht zum Nachrichtentyp.
    #[error("Daten der Nachricht „{kind}“ ungültig")]
    Data {
        /// Nachrichtentyp.
        kind: String,
    },
}

/// Eingehende Nachricht samt Umschlagdaten.
#[derive(Debug, Clone, PartialEq)]
pub struct Inbound {
    /// `MessageId` (falls vorhanden).
    pub message_id: Option<String>,
    /// Typisierter Inhalt.
    pub message: ServerMessage,
}

/// `Play`: Fernsteuerung startet Wiedergabe.
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
pub struct PlayRequest {
    /// Zu spielende Elemente.
    pub item_ids: Vec<String>,
    /// Startposition des ersten Elements.
    pub start_position_ticks: Option<i64>,
    /// Art des Abspielens.
    pub play_command: Option<PlayCommand>,
    /// Steuernder Benutzer.
    pub controlling_user_id: Option<String>,
    /// Untertitelspur.
    pub subtitle_stream_index: Option<i32>,
    /// Tonspur.
    pub audio_stream_index: Option<i32>,
    /// Medienquelle.
    pub media_source_id: Option<String>,
    /// Startindex in `item_ids`.
    pub start_index: Option<i32>,
}

/// `Playstate`: Wiedergabesteuerung.
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
pub struct PlaystateRequest {
    /// Befehl.
    pub command: Option<PlaystateCommand>,
    /// Zielposition bei `Seek`.
    pub seek_position_ticks: Option<i64>,
    /// Steuernder Benutzer.
    pub controlling_user_id: Option<String>,
}

/// `GeneralCommand`: Allgemeiner Fernbefehl.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct GeneralCommand {
    /// Befehlsname.
    pub name: GeneralCommandName,
    /// Steuernder Benutzer.
    pub controlling_user_id: Option<String>,
    /// Argumente (alle als Text).
    pub arguments: BTreeMap<String, String>,
}

impl Default for GeneralCommand {
    fn default() -> Self {
        Self {
            name: GeneralCommandName::Unknown(String::new()),
            controlling_user_id: None,
            arguments: BTreeMap::new(),
        }
    }
}

/// Typisierte Sicht auf die häufigen [`GeneralCommand`]s.
#[derive(Debug, Clone, PartialEq)]
pub enum TypedCommand {
    /// Meldung anzeigen.
    DisplayMessage {
        /// Titel.
        header: Option<String>,
        /// Text.
        text: String,
        /// Anzeigedauer in ms.
        timeout_ms: Option<i64>,
    },
    /// Lautstärke 0 bis 100.
    SetVolume(u8),
    /// Ton aus.
    Mute,
    /// Ton an.
    Unmute,
    /// Ton umschalten.
    ToggleMute,
    /// Tonspur wählen (`-1` = keine).
    SetAudioStreamIndex(i32),
    /// Untertitelspur wählen (`-1` = aus).
    SetSubtitleStreamIndex(i32),
    /// Vollbild umschalten.
    ToggleFullscreen,
    /// Zur Startseite.
    GoHome,
    /// Element anzeigen.
    DisplayContent {
        /// Element-Id.
        item_id: String,
        /// Elementart (Text).
        item_type: Option<String>,
        /// Anzeigename.
        item_name: Option<String>,
    },
    /// Alles andere (Name plus Argumente).
    Other {
        /// Befehlsname.
        name: GeneralCommandName,
        /// Argumente.
        arguments: BTreeMap<String, String>,
    },
}

impl GeneralCommand {
    /// Wertet Name und Argumente aus. Fehlende oder kaputte Pflichtargumente ergeben [`TypedCommand::Other`].
    pub fn typed(&self) -> TypedCommand {
        let arg = |k: &str| self.arguments.get(k).map(String::as_str);
        let other = || TypedCommand::Other {
            name: self.name.clone(),
            arguments: self.arguments.clone(),
        };
        match &self.name {
            GeneralCommandName::DisplayMessage => match arg("Text") {
                Some(text) => TypedCommand::DisplayMessage {
                    header: arg("Header").map(str::to_owned),
                    text: text.to_owned(),
                    timeout_ms: arg("TimeoutMs").and_then(|v| v.parse().ok()),
                },
                None => other(),
            },
            GeneralCommandName::SetVolume => {
                match arg("Volume").and_then(|v| v.parse::<f64>().ok()) {
                    Some(v) if v.is_finite() => {
                        TypedCommand::SetVolume(v.clamp(0.0, 100.0).round() as u8)
                    }
                    _ => other(),
                }
            }
            GeneralCommandName::Mute => TypedCommand::Mute,
            GeneralCommandName::Unmute => TypedCommand::Unmute,
            GeneralCommandName::ToggleMute => TypedCommand::ToggleMute,
            GeneralCommandName::SetAudioStreamIndex => {
                match arg("Index").and_then(|v| v.parse().ok()) {
                    Some(i) => TypedCommand::SetAudioStreamIndex(i),
                    None => other(),
                }
            }
            GeneralCommandName::SetSubtitleStreamIndex => {
                match arg("Index").and_then(|v| v.parse().ok()) {
                    Some(i) => TypedCommand::SetSubtitleStreamIndex(i),
                    None => other(),
                }
            }
            GeneralCommandName::ToggleFullscreen => TypedCommand::ToggleFullscreen,
            GeneralCommandName::GoHome => TypedCommand::GoHome,
            GeneralCommandName::DisplayContent => match arg("ItemId") {
                Some(id) => TypedCommand::DisplayContent {
                    item_id: id.to_owned(),
                    item_type: arg("ItemType").map(str::to_owned),
                    item_name: arg("ItemName").map(str::to_owned),
                },
                None => other(),
            },
            _ => other(),
        }
    }
}

/// Eintrag in `UserDataChanged`.
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
pub struct UserItemData {
    /// Element-Id.
    pub item_id: String,
    /// Gespeicherte Position.
    pub playback_position_ticks: i64,
    /// Anzahl Wiedergaben.
    pub play_count: i32,
    /// Favorit?
    pub is_favorite: bool,
    /// Gesehen?
    pub played: bool,
    /// Ungesehene Unterelemente.
    pub unplayed_item_count: Option<i32>,
    /// Gesehen in Prozent.
    pub played_percentage: Option<f64>,
}

/// `UserDataChanged`.
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
pub struct UserDataChangeInfo {
    /// Betroffener Benutzer.
    pub user_id: Option<String>,
    /// Geänderte Daten.
    pub user_data_list: Vec<UserItemData>,
}

/// `LibraryChanged`.
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
pub struct LibraryUpdateInfo {
    /// Ordner mit neuen Elementen.
    pub folders_added_to: Vec<String>,
    /// Ordner mit entfernten Elementen.
    pub folders_removed_from: Vec<String>,
    /// Neue Elemente.
    pub items_added: Vec<String>,
    /// Entfernte Elemente.
    pub items_removed: Vec<String>,
    /// Geänderte Elemente.
    pub items_updated: Vec<String>,
    /// Betroffene Sammlungsordner.
    pub collection_folders: Vec<String>,
}

/// SyncPlay-Kommando (`SyncPlayCommand`). Zeiten in Mikrosekunden seit 1970 (UTC, Serveruhr).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct SendCommand {
    /// Gruppe.
    #[serde(default)]
    pub group_id: String,
    /// Wiedergabelisten-Id des laufenden Elements.
    #[serde(default)]
    pub playlist_item_id: String,
    /// Ausführungszeit (Serveruhr).
    #[serde(with = "crate::time::rfc3339")]
    pub when: i64,
    /// Position in Ticks.
    #[serde(default)]
    pub position_ticks: Option<i64>,
    /// Befehl.
    pub command: SendCommandType,
    /// Sendezeit (Serveruhr).
    #[serde(with = "crate::time::rfc3339", default)]
    pub emitted_at: i64,
}

/// Gruppeninfo (`GroupInfoDto`).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
#[serde(rename_all = "PascalCase", default)]
pub struct GroupInfo {
    /// Gruppen-Id.
    pub group_id: String,
    /// Name.
    pub group_name: String,
    /// Zustand.
    pub state: Option<GroupStateType>,
    /// Teilnehmer (Anzeigenamen).
    pub participants: Vec<String>,
}

/// Eintrag der Gruppen-Wiedergabeliste.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
#[serde(rename_all = "PascalCase", default)]
pub struct SyncPlayQueueItem {
    /// Element-Id.
    pub item_id: String,
    /// Id des Listeneintrags.
    pub playlist_item_id: String,
}

/// Wiedergabeliste der Gruppe (`PlayQueueUpdate`).
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
pub struct PlayQueueUpdate {
    /// Anlass.
    pub reason: Option<PlayQueueUpdateReason>,
    /// Zeitpunkt der letzten Änderung in Mikrosekunden (Serveruhr), 0 = unbekannt.
    #[serde(with = "crate::time::rfc3339")]
    pub last_update: i64,
    /// Liste.
    pub playlist: Vec<SyncPlayQueueItem>,
    /// Index des laufenden Elements.
    pub playing_item_index: i32,
    /// Startposition zum Zeitpunkt `last_update`.
    pub start_position_ticks: i64,
    /// Läuft die Wiedergabe?
    pub is_playing: bool,
    /// Mischmodus (Text).
    pub shuffle_mode: Option<String>,
    /// Wiederholmodus (Text).
    pub repeat_mode: Option<String>,
}

/// Gruppen-Update (`SyncPlayGroupUpdate`), Unterscheidung nach `Type`.
#[derive(Debug, Clone, PartialEq)]
pub enum GroupUpdate {
    /// Benutzer beigetreten (Anzeigename).
    UserJoined {
        /// Gruppe.
        group_id: String,
        /// Anzeigename.
        user: String,
    },
    /// Benutzer ausgetreten.
    UserLeft {
        /// Gruppe.
        group_id: String,
        /// Anzeigename.
        user: String,
    },
    /// Dieser Client ist beigetreten.
    GroupJoined {
        /// Gruppe.
        group_id: String,
        /// Infos.
        info: GroupInfo,
    },
    /// Dieser Client hat die Gruppe verlassen.
    GroupLeft {
        /// Gruppe.
        group_id: String,
    },
    /// Zustand geändert.
    StateUpdate {
        /// Gruppe.
        group_id: String,
        /// Neuer Zustand.
        state: GroupStateType,
        /// Anlass.
        reason: Option<PlaybackRequestType>,
    },
    /// Wiedergabeliste geändert.
    PlayQueue {
        /// Gruppe.
        group_id: String,
        /// Liste.
        update: PlayQueueUpdate,
    },
    /// Nicht in einer Gruppe.
    NotInGroup {
        /// Gruppe.
        group_id: String,
    },
    /// Gruppe gibt es nicht.
    GroupDoesNotExist {
        /// Gruppe.
        group_id: String,
    },
    /// Kein Zugriff auf die Bibliothek der Gruppe.
    LibraryAccessDenied {
        /// Gruppe.
        group_id: String,
    },
    /// Unbekannter Typ.
    Unknown {
        /// Typ.
        kind: String,
        /// Gruppe.
        group_id: String,
        /// Rohdaten.
        data: Value,
    },
}

/// Typisierte Server-Nachricht.
#[derive(Debug, Clone, PartialEq)]
pub enum ServerMessage {
    /// Wiedergabe starten.
    Play(PlayRequest),
    /// Wiedergabe steuern.
    Playstate(PlaystateRequest),
    /// Allgemeiner Befehl.
    GeneralCommand(GeneralCommand),
    /// Benutzerdaten geändert.
    UserDataChanged(UserDataChangeInfo),
    /// Bibliothek geändert.
    LibraryChanged(LibraryUpdateInfo),
    /// Abo auf Sitzungsliste (`Data` = `"Anfangsverzögerung,Intervall"` in ms).
    SessionsStart {
        /// Anfangsverzögerung.
        initial_delay_ms: u64,
        /// Intervall.
        interval_ms: u64,
    },
    /// Abo auf Sitzungsliste beendet.
    SessionsStop,
    /// Sitzungsliste (Rohdaten).
    Sessions(Value),
    /// Server verlangt Lebenszeichen alle `Data` Sekunden.
    ForceKeepAlive {
        /// Zeitfenster in Sekunden.
        seconds: u32,
    },
    /// Lebenszeichen.
    KeepAlive,
    /// SyncPlay-Befehl.
    SyncPlayCommand(SendCommand),
    /// SyncPlay-Gruppenänderung.
    SyncPlayGroupUpdate(GroupUpdate),
    /// Server startet neu.
    ServerRestarting,
    /// Server fährt herunter.
    ServerShuttingDown,
    /// Neustart nötig.
    RestartRequired,
    /// Benutzer gelöscht (Id).
    UserDeleted(String),
    /// Alle übrigen Typen (Timer, Pakete, Aufgaben, Protokoll …) unverändert.
    Unknown {
        /// `MessageType`.
        kind: String,
        /// `Data` (Null, wenn leer).
        data: Value,
    },
}

fn data_err(kind: &str) -> ParseError {
    ParseError::Data {
        kind: kind.to_owned(),
    }
}

fn from<T: serde::de::DeserializeOwned>(kind: &str, data: Value) -> Result<T, ParseError> {
    serde_json::from_value(data).map_err(|_| data_err(kind))
}

fn str_of(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_owned()
}

fn parse_group_update(data: Value) -> Result<GroupUpdate, ParseError> {
    let kind = str_of(&data["Type"]);
    let group_id = str_of(&data["GroupId"]);
    let inner = data.get("Data").cloned().unwrap_or(Value::Null);
    let err = || data_err("SyncPlayGroupUpdate");
    Ok(match kind.as_str() {
        "UserJoined" => GroupUpdate::UserJoined {
            group_id,
            user: str_of(&inner),
        },
        "UserLeft" => GroupUpdate::UserLeft {
            group_id,
            user: str_of(&inner),
        },
        "GroupJoined" => GroupUpdate::GroupJoined {
            group_id,
            info: serde_json::from_value(inner).map_err(|_| err())?,
        },
        "GroupLeft" => GroupUpdate::GroupLeft { group_id },
        "StateUpdate" => GroupUpdate::StateUpdate {
            group_id,
            state: serde_json::from_value(inner["State"].clone()).map_err(|_| err())?,
            reason: inner
                .get("Reason")
                .and_then(|r| serde_json::from_value(r.clone()).ok()),
        },
        "PlayQueue" => GroupUpdate::PlayQueue {
            group_id,
            update: serde_json::from_value(inner).map_err(|_| err())?,
        },
        "NotInGroup" => GroupUpdate::NotInGroup { group_id },
        "GroupDoesNotExist" => GroupUpdate::GroupDoesNotExist { group_id },
        "LibraryAccessDenied" => GroupUpdate::LibraryAccessDenied { group_id },
        _ => GroupUpdate::Unknown {
            kind,
            group_id,
            data: inner,
        },
    })
}

/// Parst eine Textnachricht des WebSockets.
pub fn parse_message(text: &str) -> Result<Inbound, ParseError> {
    let env: Value = serde_json::from_str(text).map_err(|_| ParseError::Envelope)?;
    let kind = env
        .get("MessageType")
        .and_then(Value::as_str)
        .ok_or(ParseError::Envelope)?
        .to_owned();
    let message_id = env
        .get("MessageId")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let data = env.get("Data").cloned().unwrap_or(Value::Null);
    let message = match kind.as_str() {
        "Play" => ServerMessage::Play(from(&kind, data)?),
        "Playstate" => ServerMessage::Playstate(from(&kind, data)?),
        "GeneralCommand" => ServerMessage::GeneralCommand(from(&kind, data)?),
        "UserDataChanged" => ServerMessage::UserDataChanged(from(&kind, data)?),
        "LibraryChanged" => ServerMessage::LibraryChanged(from(&kind, data)?),
        "SessionsStart" => {
            let text = data.as_str().ok_or_else(|| data_err(&kind))?;
            let mut it = text.split(',').map(|p| p.trim().parse::<u64>());
            match (it.next(), it.next()) {
                (Some(Ok(a)), Some(Ok(b))) => ServerMessage::SessionsStart {
                    initial_delay_ms: a,
                    interval_ms: b,
                },
                _ => return Err(data_err(&kind)),
            }
        }
        "SessionsStop" => ServerMessage::SessionsStop,
        "Sessions" => ServerMessage::Sessions(data),
        "ForceKeepAlive" => {
            let seconds = data
                .as_u64()
                .or_else(|| data.as_str().and_then(|s| s.parse().ok()))
                .ok_or_else(|| data_err(&kind))?;
            ServerMessage::ForceKeepAlive {
                seconds: seconds.min(u64::from(u32::MAX)) as u32,
            }
        }
        "KeepAlive" => ServerMessage::KeepAlive,
        "SyncPlayCommand" => ServerMessage::SyncPlayCommand(from(&kind, data)?),
        "SyncPlayGroupUpdate" => ServerMessage::SyncPlayGroupUpdate(parse_group_update(data)?),
        "ServerRestarting" => ServerMessage::ServerRestarting,
        "ServerShuttingDown" => ServerMessage::ServerShuttingDown,
        "RestartRequired" => ServerMessage::RestartRequired,
        "UserDeleted" => ServerMessage::UserDeleted(str_of(&data)),
        _ => ServerMessage::Unknown { kind, data },
    };
    Ok(Inbound {
        message_id,
        message,
    })
}

/// Lebenszeichen des Clients.
pub fn keep_alive_message() -> String {
    json!({ "MessageType": "KeepAlive" }).to_string()
}

/// Abo auf die Sitzungsliste (nur für Verwaltungsansichten nötig).
pub fn sessions_start_message(initial_delay_ms: u64, interval_ms: u64) -> String {
    json!({ "MessageType": "SessionsStart", "Data": format!("{initial_delay_ms},{interval_ms}") })
        .to_string()
}

/// Abo beenden.
pub fn sessions_stop_message() -> String {
    json!({ "MessageType": "SessionsStop" }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(v: Value) -> ServerMessage {
        parse_message(&v.to_string()).unwrap().message
    }

    #[test]
    fn play_message() {
        let m = parse_message(
            &json!({"MessageType":"Play","MessageId":"m-1","Data":{
                "ItemIds":["a1","b2"],"StartPositionTicks":12000000,"PlayCommand":"PlayNow",
                "ControllingUserId":"u1","AudioStreamIndex":2,"SubtitleStreamIndex":-1,"StartIndex":1}})
            .to_string(),
        )
        .unwrap();
        assert_eq!(m.message_id.as_deref(), Some("m-1"));
        let ServerMessage::Play(r) = m.message else {
            panic!()
        };
        assert_eq!(r.item_ids, ["a1", "b2"]);
        assert_eq!(r.start_position_ticks, Some(12_000_000));
        assert_eq!(r.play_command, Some(PlayCommand::PlayNow));
        assert_eq!(r.subtitle_stream_index, Some(-1));
        assert_eq!(r.start_index, Some(1));
    }

    #[test]
    fn play_variants_and_missing_fields() {
        let ServerMessage::Play(r) =
            p(json!({"MessageType":"Play","Data":{"ItemIds":["x"],"PlayCommand":"PlayLast"}}))
        else {
            panic!()
        };
        assert_eq!(r.play_command, Some(PlayCommand::PlayLast));
        assert_eq!(r.start_position_ticks, None);
    }

    #[test]
    fn playstate_messages() {
        let ServerMessage::Playstate(r) = p(
            json!({"MessageType":"Playstate","Data":{"Command":"Seek","SeekPositionTicks":50000000}}),
        ) else {
            panic!()
        };
        assert_eq!(r.command, Some(PlaystateCommand::Seek));
        assert_eq!(r.seek_position_ticks, Some(50_000_000));
        for c in ["Stop", "Pause", "Unpause", "NextTrack", "PreviousTrack"] {
            let ServerMessage::Playstate(r) =
                p(json!({"MessageType":"Playstate","Data":{"Command":c}}))
            else {
                panic!()
            };
            assert_eq!(r.command.unwrap().as_str(), c);
        }
    }

    #[test]
    fn general_commands_typed() {
        let cmd = |name: &str, args: Value| -> TypedCommand {
            let ServerMessage::GeneralCommand(g) =
                p(json!({"MessageType":"GeneralCommand","Data":{"Name":name,"Arguments":args}}))
            else {
                panic!()
            };
            g.typed()
        };
        assert_eq!(
            cmd(
                "DisplayMessage",
                json!({"Header":"Hallo","Text":"Testtext","TimeoutMs":"2000"})
            ),
            TypedCommand::DisplayMessage {
                header: Some("Hallo".into()),
                text: "Testtext".into(),
                timeout_ms: Some(2000)
            }
        );
        assert_eq!(
            cmd("SetVolume", json!({"Volume":"140"})),
            TypedCommand::SetVolume(100)
        );
        assert_eq!(
            cmd("SetVolume", json!({"Volume":"35"})),
            TypedCommand::SetVolume(35)
        );
        assert_eq!(cmd("Mute", json!({})), TypedCommand::Mute);
        assert_eq!(
            cmd("SetAudioStreamIndex", json!({"Index":"3"})),
            TypedCommand::SetAudioStreamIndex(3)
        );
        assert_eq!(
            cmd("SetSubtitleStreamIndex", json!({"Index":"-1"})),
            TypedCommand::SetSubtitleStreamIndex(-1)
        );
        assert_eq!(
            cmd("ToggleFullscreen", json!({})),
            TypedCommand::ToggleFullscreen
        );
        assert_eq!(cmd("GoHome", json!({})), TypedCommand::GoHome);
        assert_eq!(
            cmd(
                "DisplayContent",
                json!({"ItemId":"i1","ItemType":"Movie","ItemName":"Testfilm"})
            ),
            TypedCommand::DisplayContent {
                item_id: "i1".into(),
                item_type: Some("Movie".into()),
                item_name: Some("Testfilm".into())
            }
        );
        // Kaputtes Pflichtargument und unbekannte Befehle fallen auf Other zurück.
        assert!(matches!(
            cmd("SetVolume", json!({"Volume":"laut"})),
            TypedCommand::Other { .. }
        ));
        assert!(matches!(
            cmd("MoveUp", json!({})),
            TypedCommand::Other { .. }
        ));
        assert!(matches!(
            cmd("Erfunden", json!({})),
            TypedCommand::Other { .. }
        ));
    }

    #[test]
    fn user_data_and_library() {
        let ServerMessage::UserDataChanged(u) = p(json!({"MessageType":"UserDataChanged","Data":{
            "UserId":"u1","UserDataList":[{"ItemId":"i1","PlaybackPositionTicks":99,"PlayCount":2,
            "IsFavorite":true,"Played":false,"Key":"k","PlayedPercentage":12.5}]}}))
        else {
            panic!()
        };
        assert_eq!(u.user_data_list[0].item_id, "i1");
        assert!(u.user_data_list[0].is_favorite);
        assert_eq!(u.user_data_list[0].played_percentage, Some(12.5));
        let ServerMessage::LibraryChanged(l) = p(json!({"MessageType":"LibraryChanged","Data":{
            "ItemsAdded":["a"],"ItemsRemoved":["b"],"ItemsUpdated":[],"FoldersAddedTo":["f"],
            "FoldersRemovedFrom":[],"CollectionFolders":["c"]}}))
        else {
            panic!()
        };
        assert_eq!(l.items_added, ["a"]);
        assert_eq!(l.collection_folders, ["c"]);
    }

    #[test]
    fn sessions_and_keepalive() {
        assert_eq!(
            p(json!({"MessageType":"SessionsStart","Data":"0,1500"})),
            ServerMessage::SessionsStart {
                initial_delay_ms: 0,
                interval_ms: 1500
            }
        );
        assert_eq!(
            p(json!({"MessageType":"SessionsStop"})),
            ServerMessage::SessionsStop
        );
        assert_eq!(
            p(json!({"MessageType":"ForceKeepAlive","Data":60})),
            ServerMessage::ForceKeepAlive { seconds: 60 }
        );
        assert_eq!(
            p(json!({"MessageType":"KeepAlive"})),
            ServerMessage::KeepAlive
        );
        assert!(parse_message(r#"{"MessageType":"SessionsStart","Data":"x"}"#).is_err());
        assert_eq!(keep_alive_message(), r#"{"MessageType":"KeepAlive"}"#);
        assert_eq!(
            serde_json::from_str::<Value>(&sessions_start_message(0, 1500)).unwrap()["Data"],
            "0,1500"
        );
    }

    #[test]
    fn syncplay_command() {
        let ServerMessage::SyncPlayCommand(c) = p(json!({"MessageType":"SyncPlayCommand","Data":{
            "GroupId":"g1","PlaylistItemId":"pl1","When":"2026-11-20T12:00:05.5000000Z",
            "PositionTicks":30000000,"Command":"Unpause","EmittedAt":"2026-11-20T12:00:04.0000000Z"}}))
        else {
            panic!()
        };
        assert_eq!(c.command, SendCommandType::Unpause);
        assert_eq!(c.position_ticks, Some(30_000_000));
        assert_eq!(c.when - c.emitted_at, 1_500_000);
    }

    #[test]
    fn syncplay_group_updates() {
        let upd = |v: Value| -> GroupUpdate {
            let ServerMessage::SyncPlayGroupUpdate(g) =
                p(json!({"MessageType":"SyncPlayGroupUpdate","Data":v}))
            else {
                panic!()
            };
            g
        };
        assert_eq!(
            upd(json!({"GroupId":"g1","Type":"UserJoined","Data":"Gast"})),
            GroupUpdate::UserJoined {
                group_id: "g1".into(),
                user: "Gast".into()
            }
        );
        let GroupUpdate::GroupJoined { info, .. } =
            upd(json!({"GroupId":"g1","Type":"GroupJoined",
            "Data":{"GroupId":"g1","GroupName":"Filmabend","State":"Idle","Participants":["A","B"],
            "LastUpdatedAt":"2026-11-20T12:00:00Z"}}))
        else {
            panic!()
        };
        assert_eq!(info.group_name, "Filmabend");
        assert_eq!(info.participants.len(), 2);
        assert_eq!(
            upd(
                json!({"GroupId":"g1","Type":"StateUpdate","Data":{"State":"Waiting","Reason":"Seek"}})
            ),
            GroupUpdate::StateUpdate {
                group_id: "g1".into(),
                state: GroupStateType::Waiting,
                reason: Some(PlaybackRequestType::Seek)
            }
        );
        let GroupUpdate::PlayQueue { update, .. } = upd(json!({"GroupId":"g1","Type":"PlayQueue",
            "Data":{"Reason":"NewPlaylist","LastUpdate":"2026-11-20T12:00:00Z",
            "Playlist":[{"ItemId":"i1","PlaylistItemId":"p1"},{"ItemId":"i2","PlaylistItemId":"p2"}],
            "PlayingItemIndex":1,"StartPositionTicks":5000,"IsPlaying":true,
            "ShuffleMode":"Sorted","RepeatMode":"RepeatNone"}}))
        else {
            panic!()
        };
        assert_eq!(update.playlist.len(), 2);
        assert_eq!(update.playing_item_index, 1);
        assert!(update.is_playing);
        assert!(matches!(
            upd(json!({"GroupId":"g","Type":"NotInGroup"})),
            GroupUpdate::NotInGroup { .. }
        ));
        assert!(matches!(
            upd(json!({"GroupId":"g","Type":"GroupLeft","Data":"g"})),
            GroupUpdate::GroupLeft { .. }
        ));
        assert!(matches!(
            upd(json!({"GroupId":"g","Type":"Neu","Data":1})),
            GroupUpdate::Unknown { .. }
        ));
    }

    #[test]
    fn unknown_and_broken() {
        let m = p(json!({"MessageType":"TimerCreated","Data":{"Id":"t"}}));
        assert!(matches!(m, ServerMessage::Unknown { ref kind, .. } if kind == "TimerCreated"));
        assert_eq!(parse_message("kein json"), Err(ParseError::Envelope));
        assert_eq!(parse_message(r#"{"Data":1}"#), Err(ParseError::Envelope));
        assert!(matches!(
            parse_message(r#"{"MessageType":"Play","Data":{"ItemIds":5}}"#),
            Err(ParseError::Data { .. })
        ));
        assert!(matches!(
            parse_message(
                r#"{"MessageType":"SyncPlayCommand","Data":{"When":"kaputt","Command":"Pause"}}"#
            ),
            Err(ParseError::Data { .. })
        ));
    }
}
