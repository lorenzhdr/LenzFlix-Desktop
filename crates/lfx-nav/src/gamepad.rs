//! Gamepad-Abbildung (rein, ohne Gerätezugriff): Totzone, Wiederholung, kontextabhängige Tasten.
//! Stick-Konvention: Y positiv = oben (wie gilrs).

use crate::action::NavAction;
use crate::context::{InputContext, direction_action};
use crate::geometry::Direction;
use crate::repeat::{KeyRepeater, RepeatConfig, RepeatEvent};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    /// Untere Fronttaste (Bestätigen).
    South,
    /// Rechte Fronttaste (Zurück).
    East,
    West,
    North,
    Start,
    Select,
    LeftShoulder,
    RightShoulder,
    DPadUp,
    DPadDown,
    DPadLeft,
    DPadRight,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GamepadAxis {
    LeftStickX,
    LeftStickY,
    /// Analoger Trigger 0..1.
    LeftTrigger,
    RightTrigger,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GamepadEvent {
    ButtonDown(Button),
    ButtonUp(Button),
    Axis(GamepadAxis, f32),
}

#[derive(Clone, Copy, Debug)]
pub struct GamepadConfig {
    /// Radiale Totzone des Sticks (0..1).
    pub deadzone: f32,
    /// Hysterese: Loslassen erst unter `deadzone * release_ratio`.
    pub release_ratio: f32,
    pub trigger_on: f32,
    pub trigger_off: f32,
    pub repeat: RepeatConfig,
}

impl Default for GamepadConfig {
    fn default() -> Self {
        Self {
            deadzone: 0.3,
            release_ratio: 0.8,
            trigger_on: 0.6,
            trigger_off: 0.3,
            repeat: RepeatConfig::default(),
        }
    }
}

/// Ergebnis der Abbildung: Aktion, ob Wiederholung, und Schrittweite (Beschleunigung).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NavInput {
    pub action: NavAction,
    pub repeat: bool,
    pub step: u32,
}

impl NavInput {
    fn first(action: NavAction) -> Self {
        Self {
            action,
            repeat: false,
            step: 1,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Src {
    Stick,
    Button(Button),
    Trigger(bool),
}

impl Src {
    fn id(self) -> u32 {
        match self {
            Src::Stick => 1000,
            Src::Trigger(left) => 1001 + u32::from(left),
            Src::Button(b) => b as u32,
        }
    }
}

#[derive(Debug)]
pub struct GamepadMapper {
    cfg: GamepadConfig,
    stick: (f32, f32),
    stick_dir: Option<Direction>,
    triggers: [bool; 2],
    repeater: KeyRepeater,
    held_action: Option<NavAction>,
}

impl Default for GamepadMapper {
    fn default() -> Self {
        Self::new(GamepadConfig::default())
    }
}

impl GamepadMapper {
    pub fn new(cfg: GamepadConfig) -> Self {
        Self {
            cfg,
            stick: (0.0, 0.0),
            stick_dir: None,
            triggers: [false; 2],
            repeater: KeyRepeater::new(cfg.repeat),
            held_action: None,
        }
    }

    fn start_hold(&mut self, src: Src, action: NavAction, now_ms: u64) -> Option<NavInput> {
        self.repeater.press(src.id(), now_ms);
        self.held_action = Some(action);
        Some(NavInput::first(action))
    }

    fn stop_hold(&mut self, src: Src) {
        if self.repeater.release(src.id()) {
            self.held_action = None;
        }
    }

    pub fn handle(&mut self, ev: GamepadEvent, now_ms: u64, ctx: InputContext) -> Option<NavInput> {
        match ev {
            GamepadEvent::ButtonDown(b) => self.button_down(b, now_ms, ctx),
            GamepadEvent::ButtonUp(b) => {
                self.stop_hold(Src::Button(b));
                None
            }
            GamepadEvent::Axis(GamepadAxis::LeftStickX, v) => {
                self.stick.0 = v;
                self.stick_update(now_ms, ctx)
            }
            GamepadEvent::Axis(GamepadAxis::LeftStickY, v) => {
                self.stick.1 = v;
                self.stick_update(now_ms, ctx)
            }
            GamepadEvent::Axis(axis, v) => {
                let left = axis == GamepadAxis::LeftTrigger;
                self.trigger_update(left, v, now_ms, ctx)
            }
        }
    }

    /// Fällige Wiederholungen (so lange aufrufen, bis `None` kommt).
    pub fn poll(&mut self, now_ms: u64) -> Option<NavInput> {
        let action = self.held_action?;
        // Langdruck-Ereignisse sind hier ohne Bedeutung und werden übersprungen.
        while let Some(ev) = self.repeater.poll(now_ms) {
            if let RepeatEvent::Repeat(t) = ev {
                return Some(NavInput {
                    action,
                    repeat: true,
                    step: t.step,
                });
            }
        }
        None
    }

    /// Gerät getrennt oder Kontextwechsel: gehaltene Eingaben verwerfen.
    pub fn reset(&mut self) {
        self.repeater.clear();
        self.held_action = None;
        self.stick = (0.0, 0.0);
        self.stick_dir = None;
        self.triggers = [false; 2];
    }

    fn button_down(&mut self, b: Button, now_ms: u64, ctx: InputContext) -> Option<NavInput> {
        let player = ctx == InputContext::Player;
        let tv = ctx == InputContext::TvMode;
        let held = |this: &mut Self, a: NavAction| this.start_hold(Src::Button(b), a, now_ms);
        match b {
            Button::DPadUp => held(self, direction_action(Direction::Up, ctx)),
            Button::DPadDown => held(self, direction_action(Direction::Down, ctx)),
            Button::DPadLeft => held(self, direction_action(Direction::Left, ctx)),
            Button::DPadRight => held(self, direction_action(Direction::Right, ctx)),
            Button::LeftShoulder | Button::RightShoulder => {
                let left = b == Button::LeftShoulder;
                let a = match (ctx, left) {
                    (InputContext::Player, true) => NavAction::Previous,
                    (InputContext::Player, false) => NavAction::Next,
                    (InputContext::TvMode, true) => NavAction::ChannelDown,
                    (InputContext::TvMode, false) => NavAction::ChannelUp,
                    (_, true) => NavAction::PageUp,
                    (_, false) => NavAction::PageDown,
                };
                // Vor/Zurück im Player nur einmalig, Blättern/Zappen wiederholt.
                if player {
                    Some(NavInput::first(a))
                } else {
                    held(self, a)
                }
            }
            Button::South => Some(NavInput::first(if player {
                NavAction::PlayPause
            } else {
                NavAction::Activate
            })),
            Button::East => Some(NavInput::first(NavAction::Back)),
            Button::Start => Some(NavInput::first(NavAction::Menu)),
            Button::Select => Some(NavInput::first(NavAction::Search)),
            Button::West => Some(NavInput::first(if tv {
                NavAction::Mute
            } else if player {
                NavAction::Info
            } else {
                NavAction::Search
            })),
            Button::North => Some(NavInput::first(if player || tv {
                NavAction::Info
            } else {
                NavAction::Menu
            })),
        }
    }

    fn stick_update(&mut self, now_ms: u64, ctx: InputContext) -> Option<NavInput> {
        let (x, y) = self.stick;
        let mag = (x * x + y * y).sqrt();
        let limit = if self.stick_dir.is_some() {
            self.cfg.deadzone * self.cfg.release_ratio
        } else {
            self.cfg.deadzone
        };
        if mag < limit {
            if self.stick_dir.take().is_some() {
                self.stop_hold(Src::Stick);
            }
            return None;
        }
        let dir = if x.abs() >= y.abs() {
            if x > 0.0 {
                Direction::Right
            } else {
                Direction::Left
            }
        } else if y > 0.0 {
            Direction::Up
        } else {
            Direction::Down
        };
        if self.stick_dir == Some(dir) {
            return None;
        }
        self.stick_dir = Some(dir);
        self.start_hold(Src::Stick, direction_action(dir, ctx), now_ms)
    }

    fn trigger_update(
        &mut self,
        left: bool,
        v: f32,
        now_ms: u64,
        ctx: InputContext,
    ) -> Option<NavInput> {
        let slot = usize::from(left);
        let on = self.triggers[slot];
        if !on && v >= self.cfg.trigger_on {
            self.triggers[slot] = true;
            let a = if left {
                NavAction::SeekBackward
            } else {
                NavAction::SeekForward
            };
            if ctx == InputContext::TextInput {
                return None;
            }
            return self.start_hold(Src::Trigger(left), a, now_ms);
        }
        if on && v <= self.cfg.trigger_off {
            self.triggers[slot] = false;
            self.stop_hold(Src::Trigger(left));
        }
        None
    }
}
