//! Die gemeinsame Aktionssprache aller Eingabearten.

/// Eine Navigations- oder Steuerungsaktion, unabhängig vom Eingabegerät.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NavAction {
    Up,
    Down,
    Left,
    Right,
    Activate,
    Back,
    Menu,
    Info,
    Home,
    Search,
    PageUp,
    PageDown,
    PlayPause,
    Play,
    Pause,
    Stop,
    Next,
    Previous,
    SeekForward,
    SeekBackward,
    VolumeUp,
    VolumeDown,
    Mute,
    Fullscreen,
    /// Ziffer 0 bis 9 (TV-Modus: Senderwahl).
    Digit(u8),
    ChannelUp,
    ChannelDown,
}

impl NavAction {
    /// Richtungsaktionen, die der Fokusgraph selbst bewegt.
    pub fn is_direction(self) -> bool {
        matches!(self, Self::Up | Self::Down | Self::Left | Self::Right)
    }
}
