//! W3-retro: Entwickler-Befehle `retro_*` (Feature `dev-automation`).
//!
//! `retro_state`, `retro_boot` (`action`: start|skip), `retro_overdrive`, `retro_rain` (`on`), `retro_term`
//! (`action`: open|close|line mit `text`), `retro_key` (`text`), `retro_pref` (`name`, `on`).

use std::rc::Rc;

use serde_json::{Value, json};

use crate::app::App;

impl App {
    /// Führt einen Retro-Befehl aus.
    pub fn retro_automation(self: &Rc<Self>, cmd: &str, msg: &Value) -> Result<Value, String> {
        match cmd {
            "retro_state" => {}
            "retro_boot" => match msg["action"].as_str().unwrap_or("skip") {
                "start" => self.retro_boot_start(true),
                "skip" => self.retro_boot_skip(),
                _ => return Err("action: start|skip".into()),
            },
            "retro_overdrive" => self.retro_overdrive(),
            "retro_rain" => {
                if msg["on"].as_bool().unwrap_or(true) {
                    self.retro_rain_start();
                } else {
                    self.retro_rain_stop();
                }
            }
            "retro_term" => match msg["action"].as_str().unwrap_or("open") {
                "open" => self.retro_term_open(),
                "close" => self.retro_term_close(),
                "line" => {
                    let text = msg["text"].as_str().unwrap_or("");
                    for c in text.chars() {
                        self.retro_term_key(&c.to_string());
                    }
                    self.retro_term_submit();
                }
                _ => return Err("action: open|close|line".into()),
            },
            "retro_key" => {
                let t = msg["text"].as_str().ok_or("text fehlt")?;
                let used = self.retro_key(t);
                return Ok(json!({"used": used, "retro": self.retro_state_json()}));
            }
            "retro_pref" => {
                let name = msg["name"].as_str().ok_or("name fehlt")?;
                self.retro_set_pref(name, msg["on"].as_bool().unwrap_or(true));
            }
            _ => return Err("unbekannter Retro-Befehl".into()),
        }
        Ok(json!({"retro": self.retro_state_json()}))
    }
}
