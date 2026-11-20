//! W4-account: Entwickler-Befehle (Feature `dev-automation`) für die Konto-, Einstellungs- und Verwaltungsseiten.
//!
//! `account_section` (`index` oder `name`), `account_row` (`label` enthält …; optional `adjust` ±1), `account_field`
//! (`key`, `value`), `account_confirm` (`yes`), `account_back`, `account_state`.

use std::rc::Rc;

use serde_json::{Value, json};

use crate::account::View;
use crate::app::App;

impl App {
    /// Zustand der Seite für Tests (Passwörter nie im Klartext).
    pub fn account_state_json(&self) -> Value {
        let st = self.st.borrow();
        let a = &st.account;
        json!({
            "admin": a.admin_mode,
            "sections": self.acc_section_names(),
            "section": a.section,
            "zone": a.zone,
            "focus": a.focus,
            "editing": a.editing,
            "view": format!("{:?}", a.view),
            "loading": !a.pending.is_empty() || a.discovering,
            "status": a.status,
            "toast": self.win().get_toast().to_string(),
            "confirm": a.confirm.as_ref().map(|c| c.text.clone()),
            "opened_url": a.opened_url,
            "rows": a.rows.iter().map(|r| json!({
                "kind": r.kind as i32, "label": r.label,
                "value": if r.secret { String::new() } else { r.value.clone() },
                "hint": r.hint, "on": r.on, "enabled": r.enabled,
            })).collect::<Vec<_>>(),
        })
    }

    /// Befehl `account_*`.
    pub fn account_automation(self: &Rc<Self>, cmd: &str, msg: &Value) -> Result<Value, String> {
        match cmd {
            "account_state" => {}
            "account_section" => {
                let names = self.acc_section_names();
                let idx = match (msg["index"].as_u64(), msg["name"].as_str()) {
                    (Some(i), _) => i as usize,
                    (None, Some(n)) => names
                        .iter()
                        .position(|s| s.contains(n))
                        .ok_or("Bereich unbekannt")?,
                    _ => return Err("index oder name fehlt".into()),
                };
                if msg["activate"].as_bool() == Some(true) {
                    self.acc_event("side", idx as i32, 0);
                } else {
                    self.acc_select_section(idx);
                }
            }
            "account_row" => {
                let want = msg["label"].as_str().ok_or("label fehlt")?;
                let i = self
                    .st
                    .borrow()
                    .account
                    .rows
                    .iter()
                    .position(|r| r.focusable() && r.label.contains(want))
                    .ok_or_else(|| format!("Zeile nicht gefunden: {want}"))?;
                match msg["adjust"].as_i64() {
                    Some(d) => self.acc_adjust(i, d as i32),
                    None => self.acc_row_activate(i),
                }
            }
            "account_field" => {
                let k = msg["key"].as_str().ok_or("key fehlt")?;
                let v = msg["value"].as_str().unwrap_or("");
                if !self.acc_set_field(k, v) {
                    return Err("Feld unbekannt".into());
                }
            }
            "account_confirm" => self.acc_confirm(msg["yes"].as_bool().unwrap_or(true)),
            "account_back" => self.acc_back(),
            "account_focus" => {
                // Fokus auf eine Zeile setzen (für Screenshots)
                let want = msg["label"].as_str().ok_or("label fehlt")?;
                let i = self
                    .st
                    .borrow()
                    .account
                    .rows
                    .iter()
                    .position(|r| r.focusable() && r.label.contains(want))
                    .ok_or("Zeile nicht gefunden")?;
                {
                    let mut st = self.st.borrow_mut();
                    st.account.zone = 1;
                    st.account.focus = i;
                }
                self.acc_push();
            }
            _ => return Err("unbekannter Befehl".into()),
        }
        let _ = View::Root;
        Ok(json!({"account": self.account_state_json()}))
    }
}
