//! Fähigkeiten dieses Clients (`POST /Sessions/Capabilities/Full`, `ClientCapabilitiesDto`).

use crate::enums::GeneralCommandName;
use serde::Serialize;

/// Körper der Fähigkeitsmeldung.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Capabilities {
    /// Abspielbare Medienarten (`Video`, `Audio`).
    pub playable_media_types: Vec<String>,
    /// Befehle, die die App ausführen kann.
    pub supported_commands: Vec<GeneralCommandName>,
    /// Fernsteuerbar?
    pub supports_media_control: bool,
    /// Stabile Gerätekennung vorhanden?
    pub supports_persistent_identifier: bool,
}

impl Default for Capabilities {
    /// Video und Audio, die Befehle, die [`crate::PlayerAction`] abbildet, plus einfache Navigation.
    fn default() -> Self {
        use GeneralCommandName as G;
        Self {
            playable_media_types: vec!["Video".into(), "Audio".into()],
            supported_commands: vec![
                G::DisplayMessage,
                G::SetVolume,
                G::Mute,
                G::Unmute,
                G::ToggleMute,
                G::VolumeUp,
                G::VolumeDown,
                G::SetAudioStreamIndex,
                G::SetSubtitleStreamIndex,
                G::ToggleFullscreen,
                G::GoHome,
                G::GoToSettings,
                G::GoToSearch,
                G::DisplayContent,
                G::Back,
                G::Select,
                G::MoveUp,
                G::MoveDown,
                G::MoveLeft,
                G::MoveRight,
            ],
            supports_media_control: true,
            supports_persistent_identifier: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_shape() {
        let v = serde_json::to_value(Capabilities::default()).unwrap();
        assert_eq!(
            v["PlayableMediaTypes"],
            serde_json::json!(["Video", "Audio"])
        );
        assert_eq!(v["SupportsMediaControl"], true);
        assert!(
            v["SupportedCommands"]
                .as_array()
                .unwrap()
                .contains(&"SetVolume".into())
        );
    }
}
