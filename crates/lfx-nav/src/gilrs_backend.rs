//! Echte Gamepads über gilrs (Feature `gamepad`). Die Umsetzung in [`GamepadEvent`] ist rein
//! und ohne Gerät testbar ([`convert`]).

use gilrs::{Axis, Button as GButton, EventType, Gilrs};

use crate::gamepad::{Button, GamepadAxis, GamepadEvent};

fn button(b: GButton) -> Option<Button> {
    Some(match b {
        GButton::South => Button::South,
        GButton::East => Button::East,
        GButton::West => Button::West,
        GButton::North => Button::North,
        GButton::Start => Button::Start,
        GButton::Select => Button::Select,
        GButton::LeftTrigger => Button::LeftShoulder,
        GButton::RightTrigger => Button::RightShoulder,
        GButton::DPadUp => Button::DPadUp,
        GButton::DPadDown => Button::DPadDown,
        GButton::DPadLeft => Button::DPadLeft,
        GButton::DPadRight => Button::DPadRight,
        _ => return None,
    })
}

/// Setzt ein gilrs-Ereignis in ein [`GamepadEvent`] um (`None` = ungenutzt).
pub fn convert(ev: &EventType) -> Option<GamepadEvent> {
    match ev {
        EventType::ButtonPressed(b, _) => button(*b).map(GamepadEvent::ButtonDown),
        EventType::ButtonReleased(b, _) => button(*b).map(GamepadEvent::ButtonUp),
        EventType::ButtonChanged(GButton::LeftTrigger2, v, _) => {
            Some(GamepadEvent::Axis(GamepadAxis::LeftTrigger, *v))
        }
        EventType::ButtonChanged(GButton::RightTrigger2, v, _) => {
            Some(GamepadEvent::Axis(GamepadAxis::RightTrigger, *v))
        }
        EventType::AxisChanged(Axis::LeftStickX, v, _) => {
            Some(GamepadEvent::Axis(GamepadAxis::LeftStickX, *v))
        }
        EventType::AxisChanged(Axis::LeftStickY, v, _) => {
            Some(GamepadEvent::Axis(GamepadAxis::LeftStickY, *v))
        }
        _ => None,
    }
}

/// Fragt angeschlossene Gamepads ab.
pub struct GilrsPump {
    gilrs: Gilrs,
}

impl GilrsPump {
    /// `None`, wenn das System keine Gamepad-Schnittstelle bietet.
    pub fn new() -> Option<Self> {
        Gilrs::new().ok().map(|gilrs| Self { gilrs })
    }

    /// Nächstes verwertbares Ereignis, `None` wenn die Warteschlange leer ist.
    pub fn next_event(&mut self) -> Option<GamepadEvent> {
        while let Some(ev) = self.gilrs.next_event() {
            if let Some(e) = convert(&ev.event) {
                return Some(e);
            }
        }
        None
    }
}
