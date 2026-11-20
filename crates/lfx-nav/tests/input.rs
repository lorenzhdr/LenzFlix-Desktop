//! Tests für Tastatur-Abbildung, Wiederholung, Gamepad und Eingabeart.

use lfx_nav::gamepad::{Button, GamepadAxis, GamepadEvent, GamepadMapper};
use lfx_nav::{
    InputContext as C, InputKind, InputTracker, Key, KeyRepeater, Modifiers as M, NavAction as A,
    RepeatConfig, RepeatEvent, RepeatTick, map_key,
};

// ---- Tastatur ----

#[test]
fn browse_keys() {
    let k = |key| map_key(key, M::NONE, C::Browse);
    assert_eq!(k(Key::Up), Some(A::Up));
    assert_eq!(k(Key::Enter), Some(A::Activate));
    assert_eq!(k(Key::Space), Some(A::Activate));
    assert_eq!(k(Key::Escape), Some(A::Back));
    assert_eq!(k(Key::Backspace), Some(A::Back));
    assert_eq!(k(Key::PageDown), Some(A::PageDown));
    assert_eq!(k(Key::Digit(7)), Some(A::Digit(7)));
    assert_eq!(k(Key::F(11)), Some(A::Fullscreen));
    assert_eq!(k(Key::Char('/')), Some(A::Search));
    assert_eq!(map_key(Key::Char('f'), M::CTRL, C::Browse), Some(A::Search));
    assert_eq!(map_key(Key::Left, M::ALT, C::Browse), Some(A::Back));
    assert_eq!(map_key(Key::Digit(1), M::SHIFT, C::Browse), None);
    assert_eq!(k(Key::Tab), None);
}

#[test]
fn player_shortcuts_like_web_player() {
    let k = |key| map_key(key, M::NONE, C::Player);
    assert_eq!(k(Key::Space), Some(A::PlayPause));
    assert_eq!(k(Key::Char('k')), Some(A::PlayPause));
    assert_eq!(k(Key::Char('K')), Some(A::PlayPause));
    assert_eq!(k(Key::Char('j')), Some(A::SeekBackward));
    assert_eq!(k(Key::Char('l')), Some(A::SeekForward));
    assert_eq!(k(Key::Left), Some(A::SeekBackward));
    assert_eq!(k(Key::Right), Some(A::SeekForward));
    assert_eq!(k(Key::Up), Some(A::VolumeUp));
    assert_eq!(k(Key::Down), Some(A::VolumeDown));
    assert_eq!(k(Key::Char('m')), Some(A::Mute));
    assert_eq!(k(Key::Char('f')), Some(A::Fullscreen));
    assert_eq!(map_key(Key::Char('n'), M::SHIFT, C::Player), Some(A::Next));
    assert_eq!(
        map_key(Key::Char('p'), M::SHIFT, C::Player),
        Some(A::Previous)
    );
}

#[test]
fn tv_mode_keys() {
    let k = |key| map_key(key, M::NONE, C::TvMode);
    assert_eq!(k(Key::Up), Some(A::ChannelUp));
    assert_eq!(k(Key::PageUp), Some(A::ChannelUp));
    assert_eq!(k(Key::Down), Some(A::ChannelDown));
    assert_eq!(k(Key::PageDown), Some(A::ChannelDown));
    assert_eq!(k(Key::Digit(4)), Some(A::Digit(4)));
    assert_eq!(k(Key::Char('M')), Some(A::Mute));
    assert_eq!(k(Key::Char('f')), Some(A::Fullscreen));
    assert_eq!(map_key(Key::Digit(4), M::SHIFT, C::TvMode), None);
}

#[test]
fn text_input_only_enter_escape_and_media() {
    let k = |key| map_key(key, M::NONE, C::TextInput);
    assert_eq!(k(Key::Char('k')), None);
    assert_eq!(k(Key::Digit(3)), None);
    assert_eq!(k(Key::Left), None);
    assert_eq!(k(Key::Space), None);
    assert_eq!(k(Key::Enter), Some(A::Activate));
    assert_eq!(k(Key::Escape), Some(A::Back));
    assert_eq!(k(Key::MediaPlayPause), Some(A::PlayPause));
    assert_eq!(k(Key::F(11)), Some(A::Fullscreen));
}

#[test]
fn media_and_remote_keys_work_everywhere() {
    for ctx in [C::Browse, C::Player, C::TvMode, C::TextInput] {
        let k = |key| map_key(key, M::NONE, ctx);
        assert_eq!(k(Key::MediaNext), Some(A::Next));
        assert_eq!(k(Key::MediaPrevious), Some(A::Previous));
        assert_eq!(k(Key::MediaStop), Some(A::Stop));
        assert_eq!(k(Key::MediaFastForward), Some(A::SeekForward));
        assert_eq!(k(Key::VolumeMute), Some(A::Mute));
        assert_eq!(k(Key::ChannelUp), Some(A::ChannelUp));
        assert_eq!(k(Key::BrowserBack), Some(A::Back));
        assert_eq!(k(Key::ContextMenu), Some(A::Menu));
        assert_eq!(k(Key::Info), Some(A::Info));
    }
}

// ---- Wiederholung ----

fn cfg() -> RepeatConfig {
    RepeatConfig {
        delay_ms: 400,
        initial_interval_ms: 100,
        min_interval_ms: 40,
        decay: 0.5,
        accel_every: 3,
        max_step: 4,
        long_press_ms: 1000,
    }
}

#[test]
fn no_repeat_before_delay_then_accelerating() {
    let mut r = KeyRepeater::new(cfg());
    assert!(r.press(1, 0));
    assert!(!r.press(1, 10), "OS-Repeat ist kein neuer Druck");
    assert_eq!(r.poll(399), None);
    // Erste Wiederholung bei 400, dann Intervalle 100, 50, 40 (Untergrenze), 40 ...
    assert_eq!(
        r.poll(400),
        Some(RepeatEvent::Repeat(RepeatTick { count: 1, step: 1 }))
    );
    assert_eq!(r.poll(499), None);
    assert_eq!(
        r.poll(500),
        Some(RepeatEvent::Repeat(RepeatTick { count: 2, step: 1 }))
    );
    assert_eq!(
        r.poll(550),
        Some(RepeatEvent::Repeat(RepeatTick { count: 3, step: 2 }))
    );
    assert_eq!(
        r.poll(590),
        Some(RepeatEvent::Repeat(RepeatTick { count: 4, step: 2 }))
    );
    assert_eq!(r.poll(629), None);
    assert_eq!(
        r.poll(630),
        Some(RepeatEvent::Repeat(RepeatTick { count: 5, step: 2 }))
    );
}

#[test]
fn step_is_capped_and_catch_up_yields_multiple_events() {
    let mut r = KeyRepeater::new(cfg());
    r.press(1, 0);
    let mut max_step = 0;
    let mut n = 0;
    while let Some(RepeatEvent::Repeat(t)) = r.poll(900) {
        max_step = max_step.max(t.step);
        n += 1;
    }
    assert!(n > 5);
    assert_eq!(max_step, 4);
}

#[test]
fn release_stops_and_other_key_replaces() {
    let mut r = KeyRepeater::new(cfg());
    r.press(1, 0);
    assert!(r.release(1));
    assert_eq!(r.poll(10_000), None);
    r.press(1, 0);
    r.press(2, 100);
    assert_eq!(r.held_id(), Some(2));
    assert!(!r.release(1));
    assert_eq!(r.poll(499), None);
    assert!(matches!(r.poll(500), Some(RepeatEvent::Repeat(_))));
}

#[test]
fn long_press_fires_once() {
    let mut r = KeyRepeater::new(RepeatConfig {
        delay_ms: 5000,
        ..cfg()
    });
    r.press(1, 0);
    assert_eq!(r.poll(999), None);
    assert_eq!(r.poll(1000), Some(RepeatEvent::LongPress));
    assert_eq!(r.poll(2000), None);
}

#[test]
fn os_repeat_counting_accelerates() {
    let mut r = KeyRepeater::new(cfg());
    r.press(7, 0);
    let steps: Vec<u32> = (0..9).map(|_| r.note_os_repeat(7).unwrap().step).collect();
    assert_eq!(steps, vec![1, 1, 2, 2, 2, 4, 4, 4, 4]);
    assert_eq!(r.note_os_repeat(8), None);
}

// ---- Gamepad ----

fn first(m: &mut GamepadMapper, ev: GamepadEvent, t: u64, c: C) -> Option<A> {
    m.handle(ev, t, c).map(|i| i.action)
}

#[test]
fn stick_deadzone_ignores_drift() {
    let mut m = GamepadMapper::default();
    assert_eq!(
        first(
            &mut m,
            GamepadEvent::Axis(GamepadAxis::LeftStickX, 0.2),
            0,
            C::Browse
        ),
        None
    );
    // Radial: 0.25/0.25 hat Betrag 0.35 > 0.3, ist aber nur diagonal.
    assert_eq!(
        first(
            &mut m,
            GamepadEvent::Axis(GamepadAxis::LeftStickX, 0.2),
            0,
            C::Browse
        ),
        None
    );
    assert_eq!(
        first(
            &mut m,
            GamepadEvent::Axis(GamepadAxis::LeftStickY, 0.2),
            0,
            C::Browse
        ),
        None
    );
    assert_eq!(m.poll(10_000), None);
}

#[test]
fn stick_press_repeat_release_with_hysteresis() {
    let mut m = GamepadMapper::default();
    let x = |v| GamepadEvent::Axis(GamepadAxis::LeftStickX, v);
    assert_eq!(first(&mut m, x(0.9), 0, C::Browse), Some(A::Right));
    // Bewegung innerhalb derselben Richtung löst nichts Neues aus.
    assert_eq!(first(&mut m, x(0.7), 10, C::Browse), None);
    assert_eq!(m.poll(399), None);
    let rep = m.poll(400).unwrap();
    assert_eq!((rep.action, rep.repeat), (A::Right, true));
    // Knapp unter der Totzone, aber über Hysterese-Grenze (0.24): bleibt gehalten.
    assert_eq!(first(&mut m, x(0.27), 450, C::Browse), None);
    assert!(m.poll(560).is_some());
    // Unter 0.24: losgelassen, keine Wiederholung mehr.
    assert_eq!(first(&mut m, x(0.1), 600, C::Browse), None);
    assert_eq!(m.poll(10_000), None);
    // Erneut anlegen = neue erste Aktion.
    assert_eq!(first(&mut m, x(-0.9), 700, C::Browse), Some(A::Left));
}

#[test]
fn stick_direction_change_and_y_up_positive() {
    let mut m = GamepadMapper::default();
    let y = |v| GamepadEvent::Axis(GamepadAxis::LeftStickY, v);
    let x = |v| GamepadEvent::Axis(GamepadAxis::LeftStickX, v);
    assert_eq!(first(&mut m, y(0.9), 0, C::Browse), Some(A::Up));
    assert_eq!(first(&mut m, y(-0.9), 10, C::Browse), Some(A::Down));
    assert_eq!(first(&mut m, y(0.0), 20, C::Browse), None);
    assert_eq!(first(&mut m, x(0.95), 30, C::Browse), Some(A::Right));
}

#[test]
fn dpad_repeat_accelerates_and_stops_on_release() {
    let mut m = GamepadMapper::default();
    assert_eq!(
        first(
            &mut m,
            GamepadEvent::ButtonDown(Button::DPadDown),
            0,
            C::Browse
        ),
        Some(A::Down)
    );
    let mut last_step = 1;
    let mut n = 0;
    while let Some(i) = m.poll(2000) {
        assert_eq!(i.action, A::Down);
        last_step = i.step;
        n += 1;
    }
    assert!(n >= 8);
    assert!(last_step > 1);
    m.handle(GamepadEvent::ButtonUp(Button::DPadDown), 2000, C::Browse);
    assert_eq!(m.poll(9000), None);
}

#[test]
fn face_buttons_per_context() {
    let mut m = GamepadMapper::default();
    let mut f = |b, c| first(&mut m, GamepadEvent::ButtonDown(b), 0, c);
    assert_eq!(f(Button::South, C::Browse), Some(A::Activate));
    assert_eq!(f(Button::South, C::Player), Some(A::PlayPause));
    assert_eq!(f(Button::East, C::Browse), Some(A::Back));
    assert_eq!(f(Button::Start, C::Browse), Some(A::Menu));
    assert_eq!(f(Button::North, C::Player), Some(A::Info));
    assert_eq!(f(Button::West, C::TvMode), Some(A::Mute));
    assert_eq!(f(Button::LeftShoulder, C::Browse), Some(A::PageUp));
    assert_eq!(f(Button::RightShoulder, C::Browse), Some(A::PageDown));
    assert_eq!(f(Button::RightShoulder, C::TvMode), Some(A::ChannelUp));
    assert_eq!(f(Button::LeftShoulder, C::Player), Some(A::Previous));
    assert_eq!(f(Button::DPadLeft, C::Player), Some(A::SeekBackward));
    assert_eq!(f(Button::DPadUp, C::TvMode), Some(A::ChannelUp));
}

#[test]
fn triggers_seek_with_hysteresis() {
    let mut m = GamepadMapper::default();
    let rt = |v| GamepadEvent::Axis(GamepadAxis::RightTrigger, v);
    assert_eq!(first(&mut m, rt(0.4), 0, C::Player), None);
    assert_eq!(first(&mut m, rt(0.8), 10, C::Player), Some(A::SeekForward));
    assert_eq!(first(&mut m, rt(0.5), 20, C::Player), None);
    assert!(m.poll(500).is_some(), "bei 0.5 weiter gehalten");
    assert_eq!(first(&mut m, rt(0.1), 600, C::Player), None);
    assert_eq!(m.poll(5000), None);
    let lt = GamepadEvent::Axis(GamepadAxis::LeftTrigger, 1.0);
    assert_eq!(first(&mut m, lt, 6000, C::Player), Some(A::SeekBackward));
}

#[test]
fn reset_drops_held_inputs() {
    let mut m = GamepadMapper::default();
    m.handle(GamepadEvent::ButtonDown(Button::DPadRight), 0, C::Browse);
    m.reset();
    assert_eq!(m.poll(5000), None);
}

// ---- Eingabeart ----

#[test]
fn tracker_couch_look_and_mouse_jitter() {
    let mut t = InputTracker::new();
    assert!(!t.couch_look());
    assert!(t.note(InputKind::Gamepad));
    assert!(t.couch_look());
    assert!(!t.note(InputKind::Gamepad));
    // Erste Mausposition und Zittern ändern nichts.
    assert!(!t.note_mouse_moved(100.0, 100.0));
    assert!(!t.note_mouse_moved(101.0, 101.0));
    assert!(t.couch_look());
    assert!(t.note_mouse_moved(200.0, 150.0));
    assert!(!t.couch_look());
    assert!(t.note(InputKind::Remote));
    assert!(t.couch_look());
    t.note(InputKind::Keyboard);
    assert!(!t.couch_look());
}
