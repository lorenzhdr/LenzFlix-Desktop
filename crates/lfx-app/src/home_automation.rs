//! Dev-Automation der Startseite (W2-home, nur mit Feature `dev-automation`).
//!
//! Befehle: `home_event` (`name`, `arg`: wie ein Ereignis der Oberfläche), `home_menu` (`menu` 0..3),
//! `home_trailer` (`immediate`: ohne Wartezeit; startet den Trailer des Billboard-Titels), `home_stop_trailer`.

use std::rc::Rc;
use std::sync::mpsc::Sender;

use serde_json::{Value, json};

use crate::app::App;

impl App {
    /// Zustand der Startseite für `state` (keine Adressen, keine Tokens).
    pub fn home_state_json(&self) -> Value {
        let ui = self.win();
        let h = ui.get_home_ui();
        let st = self.st.borrow();
        let x = &st.home.x;
        let frames = st.play.pipe.as_ref().map_or(0, |p| {
            p.stats.frames.load(std::sync::atomic::Ordering::Relaxed)
        });
        json!({
            "menu": x.menu,
            "ds_on": h.ds_on,
            "ambient": x.ambient,
            "hero_index": st.home.hero_idx,
            "hero_count": st.home.pool.len(),
            "trend_available": x.trend.available,
            "badges": st.home.rows.iter().map(|r| {
                use slint::Model;
                (0..r.model.row_count())
                    .filter(|i| !r.model.row_data(*i).is_some_and(|c| c.badge.is_empty()))
                    .count()
            }).collect::<Vec<_>>(),
            "top10_rows": st.home.rows.iter().filter(|r| r.kind == lfx_core::images::CardKind::Top10).count(),
            "trailer": {
                "active": x.trailer.active,
                "visible": x.trailer.visible,
                "muted": x.trailer.muted,
                "frames": frames,
                "rect": x.trailer.rect,
            },
            "layout": x.layout.as_ref().map(|l| json!({
                "bb": [l.bb_x, l.bb_y, l.bb_w, l.bb_h], "rows_top": l.rows_top, "row_h": l.row_h,
            })),
        })
    }

    /// Befehle `home_*`.
    pub fn home_command(self: &Rc<Self>, cmd: &str, msg: &Value, reply: Sender<Value>) {
        let ok = || {
            let _ = reply.send(json!({"ok": true}));
        };
        match cmd {
            "home_event" => {
                let name = msg["name"].as_str().unwrap_or("");
                self.on_home_event(name, msg["arg"].as_i64().unwrap_or(0) as i32);
                ok();
            }
            "home_menu" => {
                let m = msg["menu"].as_i64().unwrap_or(0) as i32;
                self.close_menu();
                if m != 0 {
                    self.open_menu(m);
                }
                ok();
            }
            "home_trailer" => {
                self.st.borrow_mut().home.x.trailer.immediate =
                    msg["immediate"].as_bool().unwrap_or(true);
                self.schedule_trailer();
                ok();
            }
            "home_shot" => {
                // Fenster nach vorn holen (parallele Läufe teilen den Desktop), dann Bildschirmfoto.
                #[cfg(windows)]
                {
                    use windows::Win32::Foundation::HWND;
                    use windows::Win32::UI::WindowsAndMessaging::{
                        BringWindowToTop, SetForegroundWindow,
                    };
                    let h = HWND(self.shell.hwnd_raw() as *mut core::ffi::c_void);
                    // SAFETY: gültiges Fensterhandle der eigenen Shell, nur Anzeigereihenfolge.
                    unsafe {
                        let _ = BringWindowToTop(h);
                        let _ = SetForegroundWindow(h);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(250));
                }
                let Some(path) = msg["path"].as_str() else {
                    let _ = reply.send(json!({"ok": false, "error": "path fehlt"}));
                    return;
                };
                match self.screenshot(std::path::Path::new(path)) {
                    Ok(info) => {
                        let mut v = json!({"ok": true});
                        if let (Some(o), Some(e)) = (v.as_object_mut(), info.as_object()) {
                            o.extend(e.clone());
                        }
                        let _ = reply.send(v);
                    }
                    Err(e) => {
                        let _ = reply.send(json!({"ok": false, "error": e}));
                    }
                }
            }
            "home_stop_trailer" => {
                self.stop_trailer();
                ok();
            }
            _ => {
                let _ = reply.send(json!({"ok": false, "error": "unbekannter Befehl"}));
            }
        }
    }
}
