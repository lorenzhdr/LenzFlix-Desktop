//! W3-picker: Couch-Modus. Sobald Gamepad oder Fernbedienung bedient wird (Einstellung „automatisch“) oder der Nutzer
//! ihn einschaltet, zeigt die App den Vollbild-Couch-Look im LENZFLIX-TV-Stil: Design immer LENZFLIX, 125 % Zoom
//! (wie `html{font-size:125%}` im Web-TV-Layout), dicker Fokusrahmen. Mauseingabe holt den normalen Look zurück.

use std::rc::Rc;

use lfx_nav::{InputKind, InputTracker};
use lfx_ui::{Design, Theme};
use slint::ComponentHandle;

use crate::app::App;

/// Einstellung „Couch-Modus“.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CouchMode {
    #[default]
    Auto,
    On,
    Off,
}

impl CouchMode {
    pub fn slug(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::On => "on",
            Self::Off => "off",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "auto" => Self::Auto,
            "on" => Self::On,
            "off" => Self::Off,
            _ => return None,
        })
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Couch-Modus: automatisch",
            Self::On => "Couch-Modus: an",
            Self::Off => "Couch-Modus: aus",
        }
    }
    fn next(self) -> Self {
        match self {
            Self::Auto => Self::On,
            Self::On => Self::Off,
            Self::Off => Self::Auto,
        }
    }
}

/// Zustand des Couch-Modus.
#[derive(Default)]
pub struct CouchCtl {
    pub mode: CouchMode,
    pub tracker: InputTracker,
    /// Couch-Look gerade aktiv.
    pub active: bool,
    #[cfg(windows)]
    pub last_cursor: Option<(i32, i32)>,
}

impl App {
    pub fn couch_label(&self) -> &'static str {
        self.st.borrow().couch.mode.label()
    }

    /// Meldet eine Eingabe (Tastatur, Gamepad, Fernbedienung, Maus).
    pub fn note_input(self: &Rc<Self>, kind: InputKind) {
        let changed = self.st.borrow_mut().couch.tracker.note(kind);
        if changed {
            self.couch_recompute();
        }
    }

    /// Quelle als Text (Automation): gamepad, remote, keyboard, mouse.
    pub fn note_input_named(self: &Rc<Self>, name: &str) -> bool {
        let kind = match name {
            "gamepad" => InputKind::Gamepad,
            "remote" => InputKind::Remote,
            "keyboard" => InputKind::Keyboard,
            "mouse" => InputKind::Mouse,
            _ => return false,
        };
        self.note_input(kind);
        true
    }

    /// Mausposition (Fensterpixel); kleine Bewegungen zählen nicht.
    pub fn couch_pointer(self: &Rc<Self>, x: f32, y: f32) {
        let changed = self.st.borrow_mut().couch.tracker.note_mouse_moved(x, y);
        if changed {
            self.couch_recompute();
        }
    }

    /// Schaltet die Einstellung weiter (automatisch, an, aus).
    pub fn couch_cycle(self: &Rc<Self>) {
        let next = self.st.borrow().couch.mode.next();
        self.set_couch_mode(next);
    }

    pub fn set_couch_mode(self: &Rc<Self>, mode: CouchMode) {
        self.st.borrow_mut().couch.mode = mode;
        self.save_setting(
            "ui.couch",
            serde_json::Value::String(mode.slug().to_owned()),
        );
        self.couch_recompute();
        self.picker_push();
    }

    /// Lädt die gespeicherte Einstellung (nach der Anmeldung).
    pub fn couch_load(self: &Rc<Self>, server_id: &str, user_id: &str) {
        if let Ok(Some(v)) = self.store.get_setting(server_id, user_id, "ui.couch")
            && let Some(m) = v.as_str().and_then(CouchMode::parse)
        {
            self.st.borrow_mut().couch.mode = m;
            self.couch_recompute();
            self.picker_push();
        }
    }

    /// Berechnet, ob der Couch-Look gilt, und wendet ihn bei Änderung an.
    pub fn couch_recompute(self: &Rc<Self>) {
        let (want, was) = {
            let st = self.st.borrow();
            let want = match st.couch.mode {
                CouchMode::On => true,
                CouchMode::Off => false,
                CouchMode::Auto => st.couch.tracker.couch_look(),
            };
            (want, st.couch.active)
        };
        if want != was {
            self.st.borrow_mut().couch.active = want;
            self.couch_apply();
        }
    }

    /// Setzt Theme (Design im Couch-Look immer LENZFLIX) und Zoom.
    pub fn couch_apply(&self) {
        let (active, design) = {
            let st = self.st.borrow();
            (st.couch.active, st.design)
        };
        let win = self.win();
        let theme = win.global::<Theme>();
        theme.set_couch(active);
        theme.set_design(if active {
            Design::Flix
        } else {
            crate::app::design_to_slint(design)
        });
    }

    /// Zoomfaktor der Seiten (für Fensterkoordinaten, z. B. Trailer-Rechteck).
    pub fn couch_zoom(&self) -> f32 {
        let st = self.st.borrow();
        if st.couch.active
            && !matches!(
                st.router.current(),
                crate::route::Route::Player { .. } | crate::route::Route::TvMode
            )
        {
            1.25
        } else {
            1.0
        }
    }

    pub fn couch_state_json(&self) -> serde_json::Value {
        let shown = match self.win().global::<Theme>().get_design() {
            Design::Flix => "flix",
            Design::Plus => "plus",
            Design::Prime => "prime",
            Design::Retro => "retro",
        };
        let st = self.st.borrow();
        serde_json::json!({
            "shown_design": shown,
            "mode": st.couch.mode.slug(),
            "active": st.couch.active,
            "last_input": format!("{:?}", st.couch.tracker.last()),
            "picker_open": st.picker.open,
            "picker_index": st.picker.index,
        })
    }

    /// Fragt den Mauszeiger ab (Windows) und meldet echte Bewegungen innerhalb des Fensters.
    pub fn start_couch_watch(self: &Rc<Self>) {
        #[cfg(windows)]
        {
            if std::env::var_os("LFX_NO_CURSOR_POLL").is_some() {
                return;
            }
            let weak = Rc::downgrade(self);
            self.timers.couch.start(
                slint::TimerMode::Repeated,
                std::time::Duration::from_millis(120),
                move || {
                    if let Some(app) = weak.upgrade() {
                        app.poll_cursor();
                    }
                },
            );
        }
    }

    #[cfg(windows)]
    fn poll_cursor(self: &Rc<Self>) {
        use std::ffi::c_void;
        use windows::Win32::Foundation::{HWND, POINT, RECT};
        use windows::Win32::Graphics::Gdi::ScreenToClient;
        use windows::Win32::UI::WindowsAndMessaging::{GetClientRect, GetCursorPos};
        let hwnd = HWND(self.shell.hwnd_raw() as *mut c_void);
        let mut p = POINT::default();
        let mut rc = RECT::default();
        // SAFETY: reine Abfragen mit lokalen Strukturen.
        let inside = unsafe {
            GetCursorPos(&mut p).is_ok()
                && ScreenToClient(hwnd, &mut p).as_bool()
                && GetClientRect(hwnd, &mut rc).is_ok()
                && p.x >= 0
                && p.y >= 0
                && p.x < rc.right
                && p.y < rc.bottom
        };
        if !inside {
            self.st.borrow_mut().couch.last_cursor = None;
            return;
        }
        let prev = self.st.borrow_mut().couch.last_cursor.replace((p.x, p.y));
        if let Some((px, py)) = prev
            && (px != p.x || py != p.y)
        {
            self.couch_pointer(p.x as f32, p.y as f32);
        }
    }
}
