//! Abbildung von winit-Ereignissen auf Slint-Ereignisse (eigene Zuordnung, Windows-Variante).

use i_slint_core::items::BuiltInMouseCursor as B;
use slint::SharedString;
use slint::platform::{Key, PointerEventButton, WindowEvent};
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta};
use winit::keyboard::{Key as WKey, NamedKey};
use winit::window::CursorIcon;

/// Pixel pro Zeile bei zeilenweisem Mausrad (wie die üblichen 3 Zeilen à 20 px).
const LINE_PX: f32 = 60.0;

pub(crate) fn named_key(k: NamedKey) -> Option<Key> {
    use NamedKey as N;
    Some(match k {
        N::Backspace => Key::Backspace,
        N::Tab => Key::Tab,
        N::Enter => Key::Return,
        N::Escape => Key::Escape,
        N::Delete => Key::Delete,
        N::Shift => Key::Shift,
        N::Control => Key::Control,
        N::Alt => Key::Alt,
        N::AltGraph => Key::AltGr,
        N::CapsLock => Key::CapsLock,
        N::Super | N::Meta => Key::Meta,
        N::Space => Key::Space,
        N::ArrowUp => Key::UpArrow,
        N::ArrowDown => Key::DownArrow,
        N::ArrowLeft => Key::LeftArrow,
        N::ArrowRight => Key::RightArrow,
        N::F1 => Key::F1,
        N::F2 => Key::F2,
        N::F3 => Key::F3,
        N::F4 => Key::F4,
        N::F5 => Key::F5,
        N::F6 => Key::F6,
        N::F7 => Key::F7,
        N::F8 => Key::F8,
        N::F9 => Key::F9,
        N::F10 => Key::F10,
        N::F11 => Key::F11,
        N::F12 => Key::F12,
        N::Insert => Key::Insert,
        N::Home => Key::Home,
        N::End => Key::End,
        N::PageUp => Key::PageUp,
        N::PageDown => Key::PageDown,
        N::ScrollLock => Key::ScrollLock,
        N::Pause => Key::Pause,
        N::PrintScreen => Key::SysReq,
        N::ContextMenu => Key::Menu,
        _ => return None,
    })
}

/// Text für ein Tastenereignis. Rechte Umschalttasten tragen eigene Codes, damit Slint den
/// Modifikatorzustand pflegen kann.
pub(crate) fn key_text(ev: &KeyEvent) -> Option<SharedString> {
    use winit::keyboard::KeyLocation;
    match &ev.logical_key {
        WKey::Named(n) => {
            let right = ev.location == KeyLocation::Right;
            let k = match (n, right) {
                (NamedKey::Shift, true) => Key::ShiftR,
                (NamedKey::Control, true) => Key::ControlR,
                (NamedKey::Super | NamedKey::Meta, true) => Key::MetaR,
                _ => named_key(*n)?,
            };
            Some(k.into())
        }
        WKey::Character(s) => Some(SharedString::from(s.as_str())),
        WKey::Dead(_) | WKey::Unidentified(_) => {
            // Tote Tasten und Unbekanntes: nichts senden (Zeichen kommt als Ime-Commit/Folgetaste).
            None
        }
    }
}

pub(crate) fn key_event(ev: &KeyEvent) -> Option<WindowEvent> {
    let text = key_text(ev)?;
    Some(match (ev.state, ev.repeat) {
        (ElementState::Pressed, false) => WindowEvent::KeyPressed { text },
        (ElementState::Pressed, true) => WindowEvent::KeyPressRepeated { text },
        (ElementState::Released, _) => WindowEvent::KeyReleased { text },
    })
}

pub(crate) fn mouse_button(b: MouseButton) -> PointerEventButton {
    match b {
        MouseButton::Left => PointerEventButton::Left,
        MouseButton::Right => PointerEventButton::Right,
        MouseButton::Middle => PointerEventButton::Middle,
        MouseButton::Back => PointerEventButton::Back,
        MouseButton::Forward => PointerEventButton::Forward,
        MouseButton::Other(_) => PointerEventButton::Other,
    }
}

/// Mausrad in logischen Pixeln.
pub(crate) fn scroll_delta(d: MouseScrollDelta, scale: f64) -> (f32, f32) {
    match d {
        MouseScrollDelta::LineDelta(x, y) => (x * LINE_PX, y * LINE_PX),
        MouseScrollDelta::PixelDelta(p) => ((p.x / scale) as f32, (p.y / scale) as f32),
    }
}

/// Slint-Cursor auf winit-Cursor abbilden. `None` heißt: Zeiger ausblenden.
pub(crate) fn cursor_icon(c: &B) -> Option<CursorIcon> {
    Some(match c {
        B::None => return None,
        B::Help => CursorIcon::Help,
        B::Pointer => CursorIcon::Pointer,
        B::Progress => CursorIcon::Progress,
        B::Wait => CursorIcon::Wait,
        B::Crosshair => CursorIcon::Crosshair,
        B::Text => CursorIcon::Text,
        B::Alias => CursorIcon::Alias,
        B::Copy => CursorIcon::Copy,
        B::Move => CursorIcon::Move,
        B::NoDrop => CursorIcon::NoDrop,
        B::NotAllowed => CursorIcon::NotAllowed,
        B::Grab => CursorIcon::Grab,
        B::Grabbing => CursorIcon::Grabbing,
        B::ColResize => CursorIcon::ColResize,
        B::RowResize => CursorIcon::RowResize,
        B::NResize => CursorIcon::NResize,
        B::EResize => CursorIcon::EResize,
        B::SResize => CursorIcon::SResize,
        B::WResize => CursorIcon::WResize,
        B::NeResize => CursorIcon::NeResize,
        B::NwResize => CursorIcon::NwResize,
        B::SeResize => CursorIcon::SeResize,
        B::SwResize => CursorIcon::SwResize,
        B::EwResize => CursorIcon::EwResize,
        B::NsResize => CursorIcon::NsResize,
        B::NeswResize => CursorIcon::NeswResize,
        B::NwseResize => CursorIcon::NwseResize,
        _ => CursorIcon::Default,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn special_keys_map() {
        assert_eq!(named_key(NamedKey::Enter), Some(Key::Return));
        assert_eq!(named_key(NamedKey::ArrowLeft), Some(Key::LeftArrow));
        assert_eq!(named_key(NamedKey::F5), Some(Key::F5));
        assert_eq!(named_key(NamedKey::MediaPlay), None);
    }

    #[test]
    fn scroll_units() {
        assert_eq!(
            scroll_delta(MouseScrollDelta::LineDelta(0.0, 1.0), 2.0),
            (0.0, 60.0)
        );
        let p = winit::dpi::PhysicalPosition::new(20.0, -40.0);
        assert_eq!(
            scroll_delta(MouseScrollDelta::PixelDelta(p), 2.0),
            (10.0, -20.0)
        );
    }

    #[test]
    fn cursor_mapping() {
        assert_eq!(cursor_icon(&B::Pointer), Some(CursorIcon::Pointer));
        assert_eq!(cursor_icon(&B::None), None);
        assert_eq!(cursor_icon(&B::Default), Some(CursorIcon::Default));
    }
}
