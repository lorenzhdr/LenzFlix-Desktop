//! W2-detail: Automationsbefehle für Hovercard, Modal, Detailseite und Kontextmenü (nur mit Feature `dev-automation`).
//!
//! `detail_open {id, mode: modal|page}`, `detail_close`, `detail_hover {row, idx}` / `detail_unhover {row, idx}`
//! (Karte der Startseite, Rechteck aus der gemeldeten Geometrie), `detail_menu {row, idx}` oder `{id}`,
//! `detail_click {what: play|fav|like|played|more}`, `detail_season {index}`, `detail_cycle {kind}`.

use std::rc::Rc;
use std::sync::mpsc::Sender;

use serde_json::{Value, json};
use slint::{ComponentHandle, Model};

use crate::app::App;
use crate::detail::Surface;
use crate::detail_logic::RectF;
use crate::route::Route;

impl App {
    /// Befehle mit dem Präfix `detail_`.
    pub fn detail_automation(self: &Rc<Self>, cmd: &str, msg: &Value, reply: Sender<Value>) {
        let ok = |extra: Value| {
            let mut v = json!({"ok": true});
            if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
                o.extend(e.clone());
            }
            let _ = reply.send(v);
        };
        let fail = |text: &str| {
            let _ = reply.send(json!({"ok": false, "error": text}));
        };
        let row = msg["row"].as_i64().unwrap_or(0) as i32;
        let idx = msg["idx"].as_i64().unwrap_or(0) as i32;
        match cmd {
            "detail_open" => {
                let Some(id) = msg["id"].as_str() else {
                    return fail("id fehlt");
                };
                if msg["mode"].as_str() == Some("page") {
                    self.go(Route::Detail { id: id.to_owned() });
                } else {
                    self.open_modal(id, None);
                }
                ok(json!({}));
            }
            "detail_close" => {
                self.close_modal();
                self.detail_close_menu();
                ok(json!({}));
            }
            "detail_hover" | "detail_unhover" => {
                let Some(g) = self.st.borrow().geo else {
                    return fail("keine Startseitengeometrie");
                };
                let rect = RectF {
                    x: g.gutter + idx as f32 * g.pitch,
                    y: g.nav_h + g.hero_h + row as f32 * g.row_h + 36.0,
                    w: g.card_w,
                    h: g.card_h,
                };
                self.on_card_hover(row, idx, cmd == "detail_hover", rect);
                ok(json!({}));
            }
            "detail_menu" => {
                let item = match msg["id"].as_str() {
                    Some(id) => self
                        .st
                        .borrow()
                        .home
                        .rows
                        .iter()
                        .flat_map(|r| r.items.iter())
                        .find(|i| i.id == id)
                        .cloned()
                        .or_else(|| self.st.borrow().detail.item.clone().filter(|i| i.id == id)),
                    None => self
                        .st
                        .borrow()
                        .home
                        .rows
                        .get(row as usize)
                        .and_then(|r| r.items.get(idx as usize).cloned()),
                };
                match item {
                    Some(item) => {
                        let (w, h) = self.window_logical();
                        self.open_menu_for(item, w * 0.35, h * 0.45, false);
                        ok(json!({}));
                    }
                    None => fail("Titel nicht gefunden"),
                }
            }
            "detail_click" => {
                match msg["what"].as_str().unwrap_or("") {
                    "play" => self.detail_play(),
                    "fav" => self.detail_toggle_fav(),
                    "like" => self.detail_toggle_like(),
                    "played" => self.detail_toggle_played(),
                    "more" => {
                        let id = self.st.borrow().detail.item.as_ref().map(|i| i.id.clone());
                        if let Some(id) = id {
                            self.open_detail_page(&id);
                        }
                    }
                    _ => return fail("unbekannte Aktion"),
                }
                ok(json!({}));
            }
            "detail_season" => {
                self.pick_season(msg["index"].as_u64().unwrap_or(0) as usize);
                ok(json!({}));
            }
            "detail_cycle" => {
                self.cycle_track(msg["kind"].as_i64().unwrap_or(1) as i32);
                ok(json!({}));
            }
            _ => fail("unbekannter Detail-Befehl"),
        }
    }

    /// Zustand der Detail-Teile für `state`.
    pub fn detail_state_json(&self) -> Value {
        let win = self.win();
        let ui = win.global::<lfx_ui::DetailState>();
        let st = self.st.borrow();
        let d = &st.detail;
        json!({
            "surface": match d.surface { Surface::None => "none", Surface::Modal => "modal", Surface::Page => "page" },
            "modal_open": ui.get_modal_open(),
            "loading": ui.get_loading(),
            "error": ui.get_error().to_string(),
            "title": d.item.as_ref().map(|i| i.name.clone()),
            "kind": d.item.as_ref().map(|i| format!("{:?}", i.kind)),
            "has_backdrop": ui.get_data().has_backdrop,
            "fav": ui.get_data().fav,
            "like": ui.get_data().like,
            "played": ui.get_data().played,
            "play_label": ui.get_data().play_label.to_string(),
            "seasons": d.seasons.len(),
            "season_idx": d.season_idx,
            "episodes": d.episodes.len(),
            "similar": d.similar.len(),
            "audio": ui.get_audio_text().to_string(),
            "sub": ui.get_sub_text().to_string(),
            "focus_sec": ui.get_focus_sec(),
            "focus_idx": ui.get_focus_idx(),
            "hovercard": {"visible": ui.get_hc_visible(), "open": ui.get_hc_open(), "title": ui.get_hc().title.to_string(),
                          "fav": ui.get_hc().fav},
            "menu": {"open": ui.get_menu_open(), "entries": ui.get_menu_entries().iter().map(|e| e.label.to_string()).collect::<Vec<_>>(),
                     "focus": ui.get_menu_focus()},
        })
    }
}
