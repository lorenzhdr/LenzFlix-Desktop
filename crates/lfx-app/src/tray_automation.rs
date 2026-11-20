//! W5-tray: Entwickler-Befehle (Feature `dev-automation`) für Infobereich-Symbol und Update-Dialog.
//!
//! `tray_state`, `tray_pref` (`on`), `tray_close_request` (wie das X der Titelleiste), `tray_action` (`name`:
//! `open|pause|quit`), `update_state`, `update_check`, `update_press` (`button`: `install|later|skip|cancel|close`),
//! `update_reset`.

use std::rc::Rc;

use serde_json::{Value, json};

use crate::app::App;
use crate::tray_logic::TrayAction;

impl App {
    /// Befehle `tray_*` und `update_*`.
    pub fn tray_automation(self: &Rc<Self>, cmd: &str, msg: &Value) -> Result<Value, String> {
        match cmd {
            "tray_state" | "update_state" => {}
            "tray_pref" => {
                let on = msg["on"].as_bool().ok_or("on fehlt")?;
                self.tray_set_close_enabled(on);
                if self.on_account_page() {
                    self.acc_rebuild();
                }
            }
            "tray_close_request" => self.request_close(),
            "tray_action" => {
                let name = msg["name"].as_str().ok_or("name fehlt")?;
                let action = match name {
                    "open" => TrayAction::Open,
                    "pause" => TrayAction::TogglePause,
                    "quit" => TrayAction::Quit,
                    _ => return Err("name unbekannt (open|pause|quit)".into()),
                };
                self.tray_action(action);
            }
            "update_check" => self.update_check(msg["manual"].as_bool().unwrap_or(true)),
            "update_press" => {
                let name = msg["button"].as_str().ok_or("button fehlt")?;
                let idx = self.update_state_json()["buttons"]
                    .as_array()
                    .and_then(|b| b.iter().position(|x| x == name))
                    .ok_or_else(|| format!("Knopf nicht vorhanden: {name}"))?;
                self.update_press(idx);
            }
            "update_reset" => self.update_reset_for_test(),
            _ => return Err("unbekannter Befehl".into()),
        }
        Ok(json!({"tray": self.tray_state_json(), "update": self.update_state_json()}))
    }
}
