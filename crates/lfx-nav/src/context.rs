//! Eingabekontext und die kontextabhängige Richtungsabbildung (gilt für Tastatur und Gamepad).

use crate::action::NavAction;
use crate::geometry::Direction;

/// In welchem Modus die Oberfläche gerade Eingaben deutet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum InputContext {
    /// Katalog, Detailseiten, Einstellungen: Pfeile bewegen den Fokus.
    #[default]
    Browse,
    /// Player: links/rechts spulen, hoch/runter Lautstärke.
    Player,
    /// TV-Modus: hoch/runter zappt, Ziffern wählen den Sender.
    TvMode,
    /// Ein Textfeld hat den Fokus: nur Enter/Esc und Medientasten.
    TextInput,
}

/// Aktion für eine Richtungstaste im jeweiligen Kontext.
pub fn direction_action(dir: Direction, ctx: InputContext) -> NavAction {
    match (ctx, dir) {
        (InputContext::Player, Direction::Left) => NavAction::SeekBackward,
        (InputContext::Player, Direction::Right) => NavAction::SeekForward,
        (InputContext::Player, Direction::Up) => NavAction::VolumeUp,
        (InputContext::Player, Direction::Down) => NavAction::VolumeDown,
        (InputContext::TvMode, Direction::Up) => NavAction::ChannelUp,
        (InputContext::TvMode, Direction::Down) => NavAction::ChannelDown,
        (_, Direction::Up) => NavAction::Up,
        (_, Direction::Down) => NavAction::Down,
        (_, Direction::Left) => NavAction::Left,
        (_, Direction::Right) => NavAction::Right,
    }
}
