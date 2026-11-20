//! W3-picker: Automation (nur mit `dev-automation`): `picker_open`, `picker_close`, `picker_choose{index}`,
//! `picker_day{day}`, `couch{mode}`, `input{source}`.

use std::rc::Rc;

use serde_json::{Value, json};

use crate::app::App;
use crate::picker_couch::CouchMode;

impl App {
    pub fn picker_automation(self: &Rc<Self>, cmd: &str, msg: &Value) -> Result<Value, String> {
        match cmd {
            "picker_open" => {
                self.picker_open();
                Ok(json!({}))
            }
            "picker_close" => {
                self.picker_close();
                Ok(json!({}))
            }
            "picker_choose" => {
                let i = msg["index"].as_i64().ok_or("index fehlt")? as i32;
                self.picker_choose(i);
                Ok(json!({}))
            }
            "picker_day" => {
                self.st.borrow_mut().picker.day_override = msg["day"].as_str().map(str::to_owned);
                self.picker_push();
                Ok(json!({"order": crate::picker::cover_order(&self.picker_day())}))
            }
            "couch" => {
                let m = msg["mode"]
                    .as_str()
                    .and_then(CouchMode::parse)
                    .ok_or("mode: auto, on oder off")?;
                self.set_couch_mode(m);
                Ok(json!({}))
            }
            "input" => {
                let s = msg["source"].as_str().unwrap_or("");
                if self.note_input_named(s) {
                    Ok(json!({}))
                } else {
                    Err("source: gamepad, remote, keyboard, mouse".into())
                }
            }
            _ => Err("unbekannter Befehl".into()),
        }
    }
}
