//! W3-picker: Design-Auswahl (Dialog wie im Web: Vorschau-Karten, sofortiger Wechsel, Wahl je Benutzer gespeichert).
//!
//! Der Dialog liegt in der Oberfläche (`PickerOverlay`), Fokus und Aktivierung laufen über Rust (Tastatur, Gamepad,
//! Automation) in [`App::picker_action`]. Die VIDEOTHEK LENZ ist nur ein ausgegrauter Platz („bald“).

use std::rc::Rc;

use lfx_core::dayseed::{day_key_from_epoch_ms, seeded_shuffle};
use lfx_nav::NavAction;
use lfx_ui::{PickerState, ToastKind};
use slint::{Color, ComponentHandle, ModelRc, VecModel};

use crate::app::App;
use crate::cli::DesignId;

/// Zahl der Karten (vier Designs plus VIDEOTHEK-Platz).
pub const CARDS: i32 = 5;
/// Karte der VIDEOTHEK LENZ (nur Platz).
pub const STORE_CARD: i32 = 4;
/// Spalten des Rasters bei breitem Dialog (Fokusbewegung hoch/runter).
const COLS: i32 = 3;

/// Basisfarben der Platzhalter-Cover (13 Regalplätze).
const COVER_COLORS: [u32; 13] = [
    0xe5484d, 0xf76b15, 0xffb224, 0x46a758, 0x12a594, 0x0091ff, 0x6e56cf, 0xd6409f, 0xab4aba,
    0x8d8d8d, 0x3e63dd, 0x30a46c, 0xf5d90a,
];

/// Zustand des Pickers.
#[derive(Default)]
pub struct PickerCtl {
    pub open: bool,
    pub index: i32,
    /// Tag für die Mischung (Test-Überschreibung); sonst das heutige Datum.
    pub day_override: Option<String>,
}

/// Reihenfolge der Platzhalter-Cover für einen Tag (gleicher Tag = gleiche Mischung).
pub fn cover_order(day: &str) -> Vec<u32> {
    seeded_shuffle(&COVER_COLORS, &format!("{day}:picker-covers"))
}

fn today() -> String {
    // Mitteleuropäische Zeit (Winterzeit) reicht für die Tagesgrenze der Vorschau.
    day_key_from_epoch_ms(App::now_ms() as i64, 60)
}

impl App {
    /// Tag der Cover-Mischung.
    pub fn picker_day(&self) -> String {
        self.st
            .borrow()
            .picker
            .day_override
            .clone()
            .unwrap_or_else(today)
    }

    /// Schreibt den Pickerzustand in die Oberfläche.
    pub fn picker_push(&self) {
        let ui = self.win();
        let (open, index, active) = {
            let st = self.st.borrow();
            (st.picker.open, st.picker.index, st.design.index())
        };
        let covers: Vec<Color> = cover_order(&self.picker_day())
            .into_iter()
            .map(|c| Color::from_rgb_u8((c >> 16) as u8, (c >> 8) as u8, c as u8))
            .collect();
        let g = ui.global::<PickerState>();
        g.set_covers(ModelRc::new(VecModel::from(covers)));
        g.set_active(active);
        g.set_index(index);
        g.set_open(open);
        g.set_couch_label(self.couch_label().into());
    }

    pub fn picker_open(self: &Rc<Self>) {
        {
            let mut st = self.st.borrow_mut();
            st.picker.open = true;
            st.picker.index = st.design.index();
        }
        self.home_close_menu();
        self.picker_push();
    }

    pub fn picker_close(self: &Rc<Self>) {
        if !self.st.borrow().picker.open {
            return;
        }
        self.st.borrow_mut().picker.open = false;
        self.picker_push();
        self.win().invoke_focus_root();
    }

    /// Wählt eine Karte: Design sofort anwenden und speichern, Dialog schließen.
    pub fn picker_choose(self: &Rc<Self>, i: i32) {
        if i == STORE_CARD {
            self.toast(
                "VIDEOTHEK LENZ kommt in einer späteren Version.",
                ToastKind::Info,
            );
            return;
        }
        if !(0..STORE_CARD).contains(&i) {
            return;
        }
        self.choose_design(DesignId::from_index(i));
        self.picker_close();
    }

    /// Navigationsaktion bei offenem Dialog; `true`, wenn der Dialog sie genommen hat.
    pub fn picker_action(self: &Rc<Self>, action: NavAction) -> bool {
        let (open, idx) = {
            let st = self.st.borrow();
            (st.picker.open, st.picker.index)
        };
        if !open {
            return false;
        }
        let next = match action {
            NavAction::Back | NavAction::Menu => {
                self.picker_close();
                return true;
            }
            NavAction::Activate => {
                self.picker_choose(idx);
                return true;
            }
            NavAction::Left => idx - 1,
            NavAction::Right => idx + 1,
            NavAction::Up => idx - COLS,
            NavAction::Down => idx + COLS,
            _ => return true,
        };
        if (0..CARDS).contains(&next) {
            self.st.borrow_mut().picker.index = next;
            self.picker_push();
        }
        true
    }

    /// Verbindet die Rückrufe des Dialogs.
    pub fn picker_wire(app: &Rc<Self>, ui: &lfx_ui::MainWindow) {
        let g = ui.global::<PickerState>();
        let a = app.clone();
        g.on_choose(move |i| a.picker_choose(i));
        let a = app.clone();
        g.on_close(move || a.picker_close());
        let a = app.clone();
        g.on_open_request(move || a.picker_open());
        let a = app.clone();
        g.on_couch_cycle(move || a.couch_cycle());
        let a = app.clone();
        g.on_pointer_moved(move |x, y| a.couch_pointer(x, y));
        app.picker_push();
    }
}
