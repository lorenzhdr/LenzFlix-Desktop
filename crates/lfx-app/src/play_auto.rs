//! Dev-Automation des Players (Feature `dev-automation`): Befehl `player` und erweiterter Zustand.

use std::rc::Rc;

use lfx_api::{ItemKind, ItemsQuery};
use lfx_player::TrackKind;
use serde_json::{Value, json};

use crate::app::{App, post};
use crate::play::ResumeMode;

impl App {
    /// `play` mit `kind` = movie | episode | resume | id (`id`) und `mode` = ask | resume | restart.
    fn automation_play(self: &Rc<Self>, msg: &Value) -> Result<(), String> {
        let Some(active) = self.session.active() else {
            return Err("nicht angemeldet".into());
        };
        let client = active.client.clone();
        let kind = msg["kind"].as_str().unwrap_or("movie").to_owned();
        let id = msg["id"].as_str().map(str::to_owned);
        let skip = msg["skip"].as_u64().unwrap_or(0) as usize;
        let mode = match msg["mode"].as_str() {
            Some("resume") => ResumeMode::Resume,
            Some("restart") => ResumeMode::Restart,
            _ => ResumeMode::Ask,
        };
        self.rt.spawn(async move {
            let item = match (kind.as_str(), id) {
                (_, Some(id)) => client.item(&id, &["MediaSources"]).await.ok(),
                ("resume", _) => client
                    .resume(10, &[], &[])
                    .await
                    .ok()
                    .and_then(|r| r.items.into_iter().nth(skip)),
                (k, _) => {
                    let kinds = if k == "episode" {
                        [ItemKind::Episode]
                    } else {
                        [ItemKind::Movie]
                    };
                    let q = ItemsQuery::new()
                        .include_types(&kinds)
                        .raw("recursive", "true")
                        .raw("limit", "10");
                    client
                        .items(&q)
                        .await
                        .ok()
                        .and_then(|r| r.items.into_iter().nth(skip))
                }
            };
            match item {
                Some(it) => post(move |app| app.play_item_with(&it, mode)),
                None => tracing::warn!("Automation play: kein Titel gefunden"),
            }
        });
        Ok(())
    }

    /// Befehl `player` mit `op`: menu, menu_pick, menu_close, scrub, scrub_end, seek_to, skip, up_next_cancel, resume,
    /// fullscreen, volume, focus, pick_track, quality, cmd (Rohbefehl der Oberfläche).
    pub fn player_automation(self: &Rc<Self>, msg: &Value) -> Result<Value, String> {
        let arg_f = msg["value"].as_f64().unwrap_or(0.0);
        match msg["op"].as_str().unwrap_or("") {
            "play" => self.automation_play(msg)?,
            "menu" => {
                let name = msg["kind"].as_str().unwrap_or("");
                self.player_cmd(&format!("menu:{name}"), 0.0);
            }
            "menu_pick" => self.player_cmd("menu-pick", arg_f as f32),
            "menu_close" => self.player_cmd("menu-close", 0.0),
            "scrub" => self.player_cmd("scrub", arg_f as f32),
            "scrub_end" => self.player_cmd("scrub-end", 0.0),
            "seek_to" => {
                let p = self.st.borrow().play.player.clone();
                if let Some(p) = p {
                    p.seek_to(arg_f);
                }
            }
            "skip" => self.player_cmd("skip", 0.0),
            "next" => self.player_cmd("next", 0.0),
            "up_next_cancel" => self.player_cmd("up-next-cancel", 0.0),
            "resume" => self.player_cmd(
                "resume-pick",
                if msg["from_start"].as_bool() == Some(true) {
                    1.0
                } else {
                    0.0
                },
            ),
            "fullscreen" => self.player_cmd("fullscreen", 0.0),
            "volume" => self.player_cmd("volume", arg_f as f32),
            "mute" => self.player_cmd("mute", 0.0),
            "quality" => self.change_quality(arg_f as usize),
            "focus" => self.apply_action(lfx_nav::NavAction::Menu),
            "pick_track" => {
                let kind = match msg["kind"].as_str() {
                    Some("audio") => TrackKind::Audio,
                    _ => TrackKind::Subtitle,
                };
                if !self.choose_track_for_test(kind, arg_f as usize) {
                    return Err("Spur nicht vorhanden".into());
                }
            }
            "cmd" => self.player_cmd(msg["name"].as_str().unwrap_or(""), arg_f as f32),
            _ => return Err("unbekannte Player-Operation".into()),
        }
        self.refresh_view();
        Ok(json!({}))
    }

    /// Zusatzfelder des Player-Zustands.
    pub fn player_state_json(&self) -> Value {
        let ui = self.win();
        let pv = ui.get_pv();
        let st = self.st.borrow();
        let p = &st.play;
        let s = p.player.as_ref().map(lfx_player::Player::state);
        let tracks = |k: TrackKind| {
            s.as_ref()
                .map_or(0, |s| s.tracks.iter().filter(|t| t.kind == k).count())
        };
        use slint::Model;
        json!({
            "item_id": p.item.as_ref().map(|i| i.id.clone()),
            "item_title": p.item.as_ref().map(|i| i.name.clone()),
            "osd": pv.osd,
            "loading": pv.loading,
            "paused_view": pv.paused,
            "note": pv.note.to_string(),
            "error_code": pv.error_code.to_string(),
            "skip_label": pv.skip_label.to_string(),
            "up_next": {"visible": pv.up_next_visible, "secs": pv.up_next_secs, "title": pv.up_next_title.to_string()},
            "resume_open": pv.resume_open,
            "resume_text": pv.resume_text.to_string(),
            "menu": {"open": pv.menu_open, "title": pv.menu_title.to_string(),
                     "items": pv.menu_items.iter().map(|r| r.label.to_string()).collect::<Vec<_>>(),
                     "selected": pv.menu_items.iter().filter(|r| r.selected).map(|r| r.label.to_string()).collect::<Vec<_>>()},
            "trick": {"visible": pv.trick_visible, "has_image": pv.trick_has_image,
                      "clip": [pv.trick_clip_x, pv.trick_clip_y, pv.trick_clip_w, pv.trick_clip_h],
                      "tiles": p.trick.tiles.len(), "geom": p.trick.geom.is_some()},
            "chapters": p.chapters.len(),
            "chapter_marks": pv.chapter_marks.row_count(),
            "segments": p.segments.len(),
            "has_next": pv.has_next,
            "cover": p.cover.is_some(),
            "audio_tracks": tracks(TrackKind::Audio),
            "subtitle_tracks": tracks(TrackKind::Subtitle),
            "volume": pv.volume,
            "fullscreen": p.fullscreen,
            "hdr_status": p.hdr_status,
            "awake": p.awake.is_some(),
            "media_controls": p.media.is_some(),
            "sub_style": p.sub_style.to_json(),
            "speed": s.as_ref().map(|s| s.speed),
            "focus_slot": pv.focus_slot,
            "pos_text": pv.pos_text.to_string(),
            "dur_text": pv.dur_text.to_string(),
        })
    }
}

/// Hält das Fenster während eines Screenshots kurz im Vordergrund (andere Fenster würden sonst in die
/// Desktop-Aufnahme geraten, z. B. bei mehreren gleichzeitig laufenden Test-Apps).
#[cfg(windows)]
pub struct TopmostGuard(windows::Win32::Foundation::HWND);

#[cfg(windows)]
impl TopmostGuard {
    pub fn new(hwnd: isize) -> Self {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::{
            HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos,
        };
        let h = HWND(hwnd as *mut _);
        // SAFETY: eigenes Fenster, reine Z-Reihenfolge.
        unsafe {
            let _ = SetWindowPos(
                h,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
        Self(h)
    }
}

#[cfg(windows)]
impl Drop for TopmostGuard {
    fn drop(&mut self) {
        use windows::Win32::UI::WindowsAndMessaging::{
            HWND_NOTOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos,
        };
        // SAFETY: siehe `new`.
        unsafe {
            let _ = SetWindowPos(
                self.0,
                Some(HWND_NOTOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
    }
}
