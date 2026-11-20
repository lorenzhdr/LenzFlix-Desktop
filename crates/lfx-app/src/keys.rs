//! Übersetzt Slint-Tastentexte in die Tasten von `lfx-nav`.

use lfx_nav::keyboard::{Key, Modifiers};
use slint::platform::Key as SlintKey;

fn is(text: &str, k: SlintKey) -> bool {
    let c: char = k.into();
    let mut it = text.chars();
    it.next() == Some(c) && it.next().is_none()
}

/// Taste aus dem Text eines Slint-Tastenereignisses; `None` bei Tasten ohne Navigationsbedeutung.
pub fn from_slint(text: &str) -> Option<Key> {
    let table: [(SlintKey, Key); 11] = [
        (SlintKey::UpArrow, Key::Up),
        (SlintKey::DownArrow, Key::Down),
        (SlintKey::LeftArrow, Key::Left),
        (SlintKey::RightArrow, Key::Right),
        (SlintKey::Return, Key::Enter),
        (SlintKey::Escape, Key::Escape),
        (SlintKey::Backspace, Key::Backspace),
        (SlintKey::PageUp, Key::PageUp),
        (SlintKey::PageDown, Key::PageDown),
        (SlintKey::Home, Key::Home),
        (SlintKey::End, Key::End),
    ];
    for (s, k) in table {
        if is(text, s) {
            return Some(k);
        }
    }
    if is(text, SlintKey::Tab) {
        return Some(Key::Tab);
    }
    let mut chars = text.chars();
    let c = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    match c {
        ' ' => Some(Key::Space),
        '\n' | '\r' => Some(Key::Enter),
        '0'..='9' => Some(Key::Digit(c as u8 - b'0')),
        c if c.is_alphabetic() => Some(Key::Char(c)),
        _ => None,
    }
}

/// Umschalttasten.
pub fn modifiers(ctrl: bool, shift: bool, alt: bool) -> Modifiers {
    Modifiers {
        ctrl,
        shift,
        alt,
        ..Modifiers::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrows_and_enter() {
        let up: char = SlintKey::UpArrow.into();
        assert_eq!(from_slint(&up.to_string()), Some(Key::Up));
        let ret: char = SlintKey::Return.into();
        assert_eq!(from_slint(&ret.to_string()), Some(Key::Enter));
        let esc: char = SlintKey::Escape.into();
        assert_eq!(from_slint(&esc.to_string()), Some(Key::Escape));
    }

    #[test]
    fn plain_characters() {
        assert_eq!(from_slint(" "), Some(Key::Space));
        assert_eq!(from_slint("7"), Some(Key::Digit(7)));
        assert_eq!(from_slint("k"), Some(Key::Char('k')));
        assert_eq!(from_slint("ab"), None);
        assert_eq!(from_slint(""), None);
        assert_eq!(from_slint("#"), None);
    }
}
