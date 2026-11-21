//! W5-downloads: Automation des Offline-Modus (Feature `dev-automation`).
//!
//! Befehle: `dl_open` (`id`: Titel laden und Dialog öffnen), `dl_enqueue` (`id`, `quality` 0..2, `unplayed`),
//! `dl_check` (Erreichbarkeit sofort prüfen), `dl_sync`, `dl_play` (`id`: lokal abspielen), `dl_click`
//! (`kind`, `a`, `b`: wie ein Klick), `dl_state`.

use std::rc::Rc;

use lfx_offline::{EnqueueOptions, TranscodeProfile};
use serde_json::{Value, json};

use crate::app::{App, post};

impl App {
    pub fn dl_automation(self: &Rc<Self>, cmd: &str, msg: &Value) -> Result<Value, String> {
        match cmd {
            "dl_state" => Ok(json!({"downloads": self.dl_state_json()})),
            "dl_check" => {
                self.dl_check_now();
                Ok(json!({}))
            }
            "dl_sync" => {
                self.dl_sync();
                Ok(json!({}))
            }
            "dl_play" => {
                let id = msg["id"].as_str().ok_or("id fehlt")?.to_owned();
                self.dl_play_item(&id);
                Ok(json!({}))
            }
            "dl_click" => {
                let k = msg["kind"].as_str().ok_or("kind fehlt")?.to_owned();
                self.dl_event(
                    &k,
                    msg["a"].as_i64().unwrap_or(0) as i32,
                    msg["b"].as_i64().unwrap_or(0) as i32,
                );
                Ok(json!({}))
            }
            "dl_open" => {
                let id = msg["id"].as_str().ok_or("id fehlt")?.to_owned();
                let client = self
                    .session
                    .active()
                    .map(|a| a.client.clone())
                    .ok_or("keine Sitzung")?;
                self.rt.spawn(async move {
                    if let Ok(item) = client.item(&id, &[]).await {
                        post(move |a| a.dl_open_dialog(&item));
                    }
                });
                Ok(json!({}))
            }
            "dl_enqueue" => {
                let id = msg["id"].as_str().ok_or("id fehlt")?.to_owned();
                let mgr = self.dl_manager().ok_or("kein Manager")?;
                let mut opts = match msg["quality"].as_u64().unwrap_or(0) {
                    0 => EnqueueOptions::original(),
                    1 => EnqueueOptions::transcoded(TranscodeProfile::medium()),
                    _ => EnqueueOptions::transcoded(TranscodeProfile::small()),
                };
                opts.only_unplayed = msg["unplayed"].as_bool().unwrap_or(false);
                self.rt.spawn(async move {
                    let r = mgr.enqueue(&id, opts).await;
                    post(move |a| {
                        if let Err(e) = r {
                            tracing::warn!("dl_enqueue: {e}");
                        }
                        a.dl_schedule_refresh();
                    });
                });
                Ok(json!({}))
            }
            _ => Err("unbekannter Befehl".into()),
        }
    }

    /// Zustand für Tests (keine Pfade, keine Adressen).
    pub fn dl_state_json(&self) -> Value {
        let st = self.st.borrow();
        let d = &st.downloads;
        let pending = d
            .outbox
            .as_ref()
            .zip(self.session.active())
            .and_then(|(o, a)| {
                o.pending(&a.server_id, &a.user_id, lfx_offline::OUTBOX_KIND)
                    .ok()
            })
            .map_or(0, |v| v.len());
        drop(st);
        let recs: Vec<Value> = self
            .dl_records()
            .iter()
            .map(|r| {
                json!({
                    "item_id": r.item_id, "name": r.name, "state": r.state.as_str(),
                    "done": r.bytes_done, "total": r.bytes_total,
                    "position_ticks": r.local.as_ref().map(|l| l.position_ticks),
                    "played": r.local.as_ref().map(|l| l.played),
                })
            })
            .collect();
        let st = self.st.borrow();
        let d = &st.downloads;
        json!({
            "offline": d.offline,
            "manager": d.mgr.is_some(),
            "records": recs,
            "rows": d.page.rows.iter().map(|r| json!({"kind": r.kind, "title": r.title, "progress": r.progress})).collect::<Vec<_>>(),
            "focus": d.page.focus,
            "confirm": d.page.confirm.is_some(),
            "dialog": d.dialog.is_some(),
            "syncs_done": d.syncs_done,
            "last_sync": d.last_sync,
            "outbox_pending": pending,
        })
    }
}
