//! Fokus- und Eingabe-Fundament. UI-frei: keine Slint-Abhängigkeit, Zeit kommt als
//! Millisekunden-Zähler von außen (deterministisch testbar).
//!
//! Alle Eingabearten (Maus-Hover, Tastatur, Gamepad, Fernbedienung, Medientasten) werden zu
//! [`NavAction`] und laufen durch denselben [`FocusGraph`].

pub mod action;
pub mod context;
pub mod gamepad;
pub mod geometry;
#[cfg(feature = "gamepad")]
pub mod gilrs_backend;
pub mod graph;
pub mod keyboard;
pub mod repeat;
pub mod tracker;

pub use action::NavAction;
pub use context::InputContext;
pub use gamepad::{Button, GamepadAxis, GamepadConfig, GamepadEvent, GamepadMapper, NavInput};
pub use geometry::{Direction, Rect};
pub use graph::{FocusGraph, Layout, NavOutcome, Target, Zone, ZoneId};
pub use keyboard::{Key, Modifiers, map_key};
pub use repeat::{KeyRepeater, RepeatConfig, RepeatEvent, RepeatTick};
pub use tracker::{InputKind, InputTracker};
