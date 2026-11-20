//! W4-media: Automation der Medienseiten (Feature `dev-automation`).
//!
//! Befehle: `media` (`kind`, `id`, `name`, `tab`), `media_view` (`ctype`, `tab`: öffnet die Bibliothek dieses
//! Typs aus `/UserViews`), `media_event` (`kind`, `a`, `b`: wie ein Mausklick), `media_shot` (`path`, nur UI),
//! `media_state`.

use slint::ComponentHandle;
use std::rc::Rc;

use lfx_nav::NavAction;
use serde_json::{Value, json};

use crate::app::{App, post};
use crate::media_model::{MediaKind, MediaRoute, kind_for_collection};
use crate::route::Route;

fn kind_from_slug(s: &str) -> Option<MediaKind> {
    Some(match s {
        "music" => MediaKind::Music,
        "album" => MediaKind::Album,
        "artist" => MediaKind::Artist,
        "playlist" => MediaKind::Playlist,
        "books" => MediaKind::Books,
        "book" => MediaKind::Book,
        "photos" => MediaKind::Photos,
        "livetv" => MediaKind::LiveTv,
        _ => return None,
    })
}

impl App {
    pub fn media_automation(self: &Rc<Self>, cmd: &str, msg: &Value) -> Result<Value, String> {
        match cmd {
            "media" => {
                let kind = msg["kind"]
                    .as_str()
                    .and_then(kind_from_slug)
                    .ok_or("kind fehlt")?;
                let id = msg["id"].as_str().unwrap_or("").to_owned();
                let name = msg["name"].as_str().unwrap_or("Medien").to_owned();
                let tab = msg["tab"].as_u64().unwrap_or(0) as usize;
                self.go(Route::Media(MediaRoute::new(kind, id, name).with_tab(tab)));
                Ok(json!({}))
            }
            "media_view" => {
                let ctype = msg["ctype"].as_str().ok_or("ctype fehlt")?.to_owned();
                let tab = msg["tab"].as_u64().unwrap_or(0) as usize;
                let kind = kind_for_collection(&ctype).ok_or("unbekannter Typ")?;
                let client = self.media_client().ok_or("keine Sitzung")?;
                self.rt.spawn(async move {
                    let views = client.views().await.map(|v| v.items).unwrap_or_default();
                    post(move |app| {
                        if let Some(v) = views
                            .iter()
                            .find(|v| v.collection_type.as_deref() == Some(ctype.as_str()))
                        {
                            app.go(Route::Media(
                                MediaRoute::new(kind, v.id.clone(), v.name.clone()).with_tab(tab),
                            ));
                        }
                    });
                });
                Ok(json!({}))
            }
            "media_event" => {
                let k = msg["kind"].as_str().ok_or("kind fehlt")?;
                self.media_event(
                    k,
                    msg["a"].as_i64().unwrap_or(0) as i32,
                    msg["b"].as_i64().unwrap_or(0) as i32,
                );
                Ok(json!({}))
            }
            "media_text" => {
                let t = msg["text"].as_str().unwrap_or("");
                self.with_media(|g| g.set_prompt_text(t.into()));
                Ok(json!({}))
            }
            "media_action" => {
                // Wie `nav`, aber mit Namen aus der Medienwelt (Dauerlauf-freundlich)
                let a = match msg["action"].as_str().unwrap_or("") {
                    "PlayPause" => NavAction::PlayPause,
                    "Next" => NavAction::Next,
                    "Previous" => NavAction::Previous,
                    "Stop" => NavAction::Stop,
                    _ => return Err("unbekannte Aktion".into()),
                };
                self.apply_action(a);
                Ok(json!({}))
            }
            "media_slide_tick" => {
                self.viewer_step(1);
                Ok(json!({}))
            }
            "media_shot" => {
                let path = msg["path"].as_str().ok_or("path fehlt")?;
                let snap = self
                    .win()
                    .window()
                    .take_snapshot()
                    .map_err(|e| format!("Schnappschuss nicht möglich: {e}"))?;
                crate::capture::write_png(
                    std::path::Path::new(path),
                    snap.width(),
                    snap.height(),
                    snap.as_bytes(),
                )?;
                Ok(json!({"width": snap.width(), "height": snap.height()}))
            }
            "media_state" => Ok(self.media_state_json()),
            _ => Err("unbekannter Befehl".into()),
        }
    }

    pub fn media_state_json(&self) -> Value {
        let st = self.st.borrow();
        let m = &st.media;
        let viewer = m.viewer.as_ref().map(|v| {
            json!({"index": v.index, "count": v.photos.len(), "zoom": v.zoom,
                   "slide": v.slide})
        });
        let ui = self.win();
        let g = ui.global::<lfx_ui::MediaUi>();
        use slint::Model;
        json!({
            "route": m.route.as_ref().map(MediaRoute::slug),
            "tab": m.route.as_ref().map(|r| r.tab),
            "title": g.get_title().to_string(),
            "view": format!("{:?}", m.view),
            "items": m.items.iter().map(|i| i.name.clone()).collect::<Vec<_>>(),
            "kinds": m.items.iter().map(|i| format!("{:?}", i.kind)).collect::<Vec<_>>(),
            "cards": g.get_cards().row_count(),
            "tracks": g.get_tracks().row_count(),
            "timers": m.timers.len(),
            "guide_rows": m.guide.rows.len(),
            "guide_cells": m.guide.rows.iter().map(Vec::len).collect::<Vec<_>>(),
            "zone": m.zone,
            "index": m.index,
            "col": m.col,
            "cols": m.cols,
            "loading": g.get_loading(),
            "error": g.get_error().to_string(),
            "empty": g.get_empty().to_string(),
            "menu": m.menu.as_ref().map(|x| x.entries.iter().map(|(l, _)| l.clone()).collect::<Vec<_>>()),
            "prompt": m.prompt.is_some(),
            "viewer": viewer,
            "viewer_has_image": g.get_viewer_has_image(),
            "opened_files": m.opened_files.iter().map(|p| p.file_name().map(|f| f.to_string_lossy().into_owned())).collect::<Vec<_>>(),
            "audio": drop_audio(&st, self),
        })
    }
}

fn drop_audio(_st: &crate::app::State, app: &App) -> Value {
    // `audio_state_json` leiht den Zustand selbst; hier nur weiterreichen (kein doppeltes borrow_mut).
    app.audio_state_json()
}
