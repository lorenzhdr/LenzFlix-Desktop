//! Zeichenketten-Aufzählungen der Jellyfin-Spezifikation mit Rückfallwert `Unknown(String)`,
//! damit neue Server-Werte nie einen Parse-Fehler auslösen.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

macro_rules! string_enum {
    ($(#[$m:meta])* $name:ident { $($var:ident),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub enum $name {
            $(
                #[allow(missing_docs)]
                $var,
            )+
            /// Wert, den diese Version nicht kennt.
            Unknown(String),
        }

        impl $name {
            /// Name wie auf der Leitung.
            pub fn as_str(&self) -> &str {
                match self {
                    $(Self::$var => stringify!($var),)+
                    Self::Unknown(s) => s,
                }
            }
            /// Liest den Namen (unbekannte Werte werden `Unknown`).
            pub fn parse(s: &str) -> Self {
                match s {
                    $(stringify!($var) => Self::$var,)+
                    other => Self::Unknown(other.to_owned()),
                }
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(self.as_str())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                Ok(Self::parse(&String::deserialize(d)?))
            }
        }
    };
}

string_enum! {
    /// `PlayCommand` einer Play-Nachricht.
    PlayCommand { PlayNow, PlayNext, PlayLast, PlayInstantMix, PlayShuffle }
}

string_enum! {
    /// `PlaystateCommand` einer Playstate-Nachricht.
    PlaystateCommand { Stop, Pause, Unpause, NextTrack, PreviousTrack, Seek, Rewind, FastForward, PlayPause }
}

string_enum! {
    /// Befehl eines SyncPlay-Kommandos (`SendCommandType`).
    SendCommandType { Unpause, Pause, Stop, Seek }
}

string_enum! {
    /// Zustand einer SyncPlay-Gruppe.
    GroupStateType { Idle, Waiting, Paused, Playing }
}

string_enum! {
    /// Anlass einer Wiedergabelisten-Änderung.
    PlayQueueUpdateReason {
        NewPlaylist, SetCurrentItem, RemoveItems, MoveItem, Queue, QueueNext,
        NextItem, PreviousItem, RepeatMode, ShuffleMode
    }
}

string_enum! {
    /// Anlass einer Zustandsänderung der Gruppe.
    PlaybackRequestType {
        Play, SetPlaylistItem, RemoveFromPlaylist, MovePlaylistItem, Queue, Unpause, Pause, Stop,
        Seek, Buffer, Ready, NextItem, PreviousItem, SetRepeatMode, SetShuffleMode, Ping, IgnoreWait
    }
}

string_enum! {
    /// Modus beim Einreihen in die Gruppen-Wiedergabeliste.
    GroupQueueMode { Queue, QueueNext }
}

string_enum! {
    /// Wiederholmodus der Gruppe.
    GroupRepeatMode { RepeatOne, RepeatAll, RepeatNone }
}

string_enum! {
    /// Mischmodus der Gruppe.
    GroupShuffleMode { Sorted, Shuffle }
}

string_enum! {
    /// Allgemeiner Fernbefehl (`GeneralCommandType`).
    GeneralCommandName {
        MoveUp, MoveDown, MoveLeft, MoveRight, PageUp, PageDown, PreviousLetter, NextLetter,
        ToggleOsd, ToggleContextMenu, Select, Back, TakeScreenshot, SendKey, SendString, GoHome,
        GoToSettings, VolumeUp, VolumeDown, Mute, Unmute, ToggleMute, SetVolume, SetAudioStreamIndex,
        SetSubtitleStreamIndex, ToggleFullscreen, DisplayContent, GoToSearch, DisplayMessage,
        SetRepeatMode, ChannelUp, ChannelDown, Guide, ToggleStats, PlayMediaSource, PlayTrailers,
        SetShuffleQueue, PlayState, PlayNext, ToggleOsdMenu, Play, SetMaxStreamingBitrate,
        SetPlaybackOrder
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_and_unknown_values() {
        assert_eq!(PlayCommand::parse("PlayNow"), PlayCommand::PlayNow);
        assert_eq!(
            PlaystateCommand::parse("Zukunft"),
            PlaystateCommand::Unknown("Zukunft".into())
        );
        let v: GroupStateType = serde_json::from_str("\"Waiting\"").unwrap();
        assert_eq!(v, GroupStateType::Waiting);
        assert_eq!(serde_json::to_string(&v).unwrap(), "\"Waiting\"");
        let u: GroupStateType = serde_json::from_str("\"Neu\"").unwrap();
        assert_eq!(serde_json::to_string(&u).unwrap(), "\"Neu\"");
    }
}
