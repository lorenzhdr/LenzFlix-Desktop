//! Tastatur- und Fernbedienungs-Abbildung. Eigene, fensterunabhängige Tastenliste; die UI
//! übersetzt ihre Tastenereignisse (Slint, winit) einmal in [`Key`].

use crate::action::NavAction;
use crate::context::{InputContext, direction_action};
use crate::geometry::Direction;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    Enter,
    Space,
    Escape,
    Backspace,
    PageUp,
    PageDown,
    Home,
    End,
    Tab,
    /// Funktionstaste F1 bis F12.
    F(u8),
    /// Ziffer (Hauptblock und Ziffernblock).
    Digit(u8),
    /// Buchstabe oder Zeichen (Groß-/Kleinschreibung egal).
    Char(char),
    MediaPlayPause,
    MediaPlay,
    MediaPause,
    MediaStop,
    MediaNext,
    MediaPrevious,
    MediaFastForward,
    MediaRewind,
    VolumeUp,
    VolumeDown,
    VolumeMute,
    // Fernbedienungstasten
    ChannelUp,
    ChannelDown,
    ContextMenu,
    Info,
    BrowserBack,
    BrowserHome,
    BrowserSearch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub meta: bool,
}

impl Modifiers {
    pub const NONE: Self = Self {
        shift: false,
        ctrl: false,
        alt: false,
        meta: false,
    };
    pub const SHIFT: Self = Self {
        shift: true,
        ..Self::NONE
    };
    pub const CTRL: Self = Self {
        ctrl: true,
        ..Self::NONE
    };
    pub const ALT: Self = Self {
        alt: true,
        ..Self::NONE
    };

    fn command(self) -> bool {
        self.ctrl || self.alt || self.meta
    }
}

/// Medien- und Fernbedienungstasten gelten in jedem Kontext.
fn global_key(key: Key) -> Option<NavAction> {
    Some(match key {
        Key::MediaPlayPause => NavAction::PlayPause,
        Key::MediaPlay => NavAction::Play,
        Key::MediaPause => NavAction::Pause,
        Key::MediaStop => NavAction::Stop,
        Key::MediaNext => NavAction::Next,
        Key::MediaPrevious => NavAction::Previous,
        Key::MediaFastForward => NavAction::SeekForward,
        Key::MediaRewind => NavAction::SeekBackward,
        Key::VolumeUp => NavAction::VolumeUp,
        Key::VolumeDown => NavAction::VolumeDown,
        Key::VolumeMute => NavAction::Mute,
        Key::ChannelUp => NavAction::ChannelUp,
        Key::ChannelDown => NavAction::ChannelDown,
        Key::ContextMenu => NavAction::Menu,
        Key::Info => NavAction::Info,
        Key::BrowserBack => NavAction::Back,
        Key::BrowserHome => NavAction::Home,
        Key::BrowserSearch => NavAction::Search,
        Key::F(11) => NavAction::Fullscreen,
        _ => return None,
    })
}

/// Aktion zu einer Taste im jeweiligen Kontext; `None` = Taste gehört nicht der Navigation.
///
/// Spulen: `J`/`L` und Links/Rechts im Player liefern `SeekBackward`/`SeekForward`; die
/// Schrittweite (10 s, beschleunigt per [`crate::repeat::RepeatTick::step`]) legt der Player fest.
pub fn map_key(key: Key, mods: Modifiers, ctx: InputContext) -> Option<NavAction> {
    if let Some(a) = global_key(key) {
        return Some(a);
    }
    // Alt+Links = Zurück (wie im Browser), unabhängig vom Kontext außer Texteingabe.
    if mods.alt && key == Key::Left && ctx != InputContext::TextInput {
        return Some(NavAction::Back);
    }
    if ctx == InputContext::TextInput {
        return match key {
            Key::Enter => Some(NavAction::Activate),
            Key::Escape => Some(NavAction::Back),
            _ => None,
        };
    }
    let dir = match key {
        Key::Up => Some(Direction::Up),
        Key::Down => Some(Direction::Down),
        Key::Left => Some(Direction::Left),
        Key::Right => Some(Direction::Right),
        _ => None,
    };
    if let Some(d) = dir {
        return (!mods.command()).then(|| direction_action(d, ctx));
    }
    match key {
        Key::Enter => return Some(NavAction::Activate),
        Key::Escape | Key::Backspace => return Some(NavAction::Back),
        _ => {}
    }
    let in_player = ctx == InputContext::Player;
    match key {
        Key::Space if in_player => return Some(NavAction::PlayPause),
        Key::Space => return Some(NavAction::Activate),
        Key::PageUp | Key::PageDown => {
            let up = key == Key::PageUp;
            return Some(match (ctx, up) {
                (InputContext::TvMode, true) => NavAction::ChannelUp,
                (InputContext::TvMode, false) => NavAction::ChannelDown,
                (_, true) => NavAction::PageUp,
                (_, false) => NavAction::PageDown,
            });
        }
        Key::Home => return Some(NavAction::Home),
        Key::Digit(n) if n <= 9 && !mods.shift && !mods.command() => {
            return Some(NavAction::Digit(n));
        }
        _ => {}
    }
    let Key::Char(c) = key else {
        return None;
    };
    let c = c.to_ascii_lowercase();
    if c == '/' && !mods.command() || c == 'f' && mods.ctrl && ctx == InputContext::Browse {
        return Some(NavAction::Search);
    }
    if mods.command() {
        return None;
    }
    match (ctx, c) {
        (InputContext::Player, 'k') => Some(NavAction::PlayPause),
        (InputContext::Player, 'j') => Some(NavAction::SeekBackward),
        (InputContext::Player, 'l') => Some(NavAction::SeekForward),
        (InputContext::Player, 'n') if mods.shift => Some(NavAction::Next),
        (InputContext::Player, 'p') if mods.shift => Some(NavAction::Previous),
        (InputContext::Player | InputContext::TvMode, 'm') => Some(NavAction::Mute),
        (InputContext::Player | InputContext::TvMode, 'f') => Some(NavAction::Fullscreen),
        (InputContext::Player | InputContext::TvMode, 'i') => Some(NavAction::Info),
        _ => None,
    }
}
