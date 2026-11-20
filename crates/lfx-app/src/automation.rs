//! Entwickler-Steuerung (Feature `dev-automation`): ein Zeilen-JSON-Protokoll über TCP, nur 127.0.0.1.
//!
//! Beim Start wird ein zufälliger Port gewählt; Port und Einmal-Schlüssel stehen in
//! `<Datenordner>/automation.json`. Jede Verbindung beginnt mit `{"cmd":"auth","key":"…"}`.
//! Befehle: `state`, `nav` (`action`), `route` (`route`), `login` (`server`, `user`, `password`),
//! `quick_connect`, `design`, `screenshot` (`path`), `quit`. Antworten sind je eine JSON-Zeile mit `ok`.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::rc::Rc;
use std::sync::mpsc::{Sender, channel};
use std::time::Duration;

use lfx_nav::NavAction;
use serde_json::{Value, json};
use slint::ComponentHandle;

use crate::app::{App, post};
use crate::cli::DesignId;
use crate::route::Route;

/// Name der Datei mit Port und Schlüssel.
pub const INFO_FILE: &str = "automation.json";

fn parse_action(name: &str) -> Option<NavAction> {
    Some(match name {
        "Up" => NavAction::Up,
        "Down" => NavAction::Down,
        "Left" => NavAction::Left,
        "Right" => NavAction::Right,
        "Activate" => NavAction::Activate,
        "Back" => NavAction::Back,
        "Menu" => NavAction::Menu,
        "Info" => NavAction::Info,
        "Home" => NavAction::Home,
        "Search" => NavAction::Search,
        "PageUp" => NavAction::PageUp,
        "PageDown" => NavAction::PageDown,
        "PlayPause" => NavAction::PlayPause,
        "Play" => NavAction::Play,
        "Pause" => NavAction::Pause,
        "Stop" => NavAction::Stop,
        "SeekForward" => NavAction::SeekForward,
        "SeekBackward" => NavAction::SeekBackward,
        "VolumeUp" => NavAction::VolumeUp,
        "VolumeDown" => NavAction::VolumeDown,
        "Mute" => NavAction::Mute,
        _ => return None,
    })
}

fn random_key() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut out = String::new();
    for _ in 0..4 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos()),
        );
        out.push_str(&format!("{:016x}", h.finish()));
    }
    out
}

/// Startet den Listener; schreibt Port und Schlüssel nach `<data>/automation.json`.
pub fn start(data_dir: &Path) -> std::io::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let key = random_key();
    std::fs::create_dir_all(data_dir)?;
    std::fs::write(
        data_dir.join(INFO_FILE),
        json!({ "port": port, "key": key }).to_string(),
    )?;
    tracing::info!(port, "Automation lauscht");
    std::thread::Builder::new()
        .name("lfx-automation".into())
        .spawn(move || {
            for conn in listener.incoming().flatten() {
                let key = key.clone();
                let _ = std::thread::Builder::new()
                    .name("lfx-automation-conn".into())
                    .spawn(move || serve(conn, &key));
            }
        })?;
    Ok(())
}

fn serve(stream: TcpStream, key: &str) {
    let Ok(mut out) = stream.try_clone() else {
        return;
    };
    let reader = BufReader::new(stream);
    let mut authed = false;
    for line in reader.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            let _ = writeln!(out, "{}", json!({"ok": false, "error": "kein JSON"}));
            continue;
        };
        let cmd = msg["cmd"].as_str().unwrap_or("").to_owned();
        if !authed {
            if cmd == "auth" && msg["key"].as_str() == Some(key) {
                authed = true;
                let _ = writeln!(out, "{}", json!({"ok": true}));
                continue;
            }
            let _ = writeln!(out, "{}", json!({"ok": false, "error": "nicht angemeldet"}));
            break;
        }
        let (tx, rx) = channel::<Value>();
        let queued = slint::invoke_from_event_loop(move || {
            if let Some(app) = crate::app::current() {
                app.automation_command(&cmd, &msg, tx);
            } else {
                let _ = tx.send(json!({"ok": false, "error": "App nicht bereit"}));
            }
        });
        let reply = if queued.is_err() {
            json!({"ok": false, "error": "Ereignisschleife beendet"})
        } else {
            rx.recv_timeout(Duration::from_secs(60))
                .unwrap_or_else(|_| json!({"ok": false, "error": "Zeitüberschreitung"}))
        };
        if writeln!(out, "{reply}").is_err() {
            break;
        }
    }
}

impl App {
    /// Führt einen Befehl im UI-Thread aus; die Antwort kann auch später (aus einem anderen Thread) kommen.
    pub fn automation_command(self: &Rc<Self>, cmd: &str, msg: &Value, reply: Sender<Value>) {
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
        match cmd {
            "state" => ok(json!({"state": self.state_json()})),
            "nav" => match msg["action"].as_str().and_then(parse_action) {
                Some(a) => {
                    let n = msg["count"].as_u64().unwrap_or(1).clamp(1, 50);
                    for _ in 0..n {
                        self.apply_action(a);
                    }
                    ok(json!({}));
                }
                None => fail("unbekannte Aktion"),
            },
            "route" => match msg["route"].as_str().and_then(Route::from_name) {
                Some(r) => {
                    self.go(r);
                    ok(json!({}));
                }
                None => fail("unbekannte Route"),
            },
            "design" => match msg["design"].as_str().and_then(DesignId::parse) {
                Some(d) => {
                    self.apply_design(d);
                    ok(json!({}));
                }
                None => fail("unbekanntes Design"),
            },
            "login" => {
                let url = msg["server"].as_str().map(str::to_owned);
                let user = msg["user"].as_str().unwrap_or("").to_owned();
                let password = msg["password"].as_str().unwrap_or("").to_owned();
                let session = self.session.clone();
                let server_known = self.st.borrow().server_id.clone();
                self.rt.spawn(async move {
                    let server_id = match (url, server_known) {
                        (Some(u), _) => session.add_server(&u).await.map(|e| e.id),
                        (None, Some(id)) => Ok(id),
                        (None, None) => session.servers().first().map(|s| s.id.clone()).ok_or(
                            lfx_session::SessionError::InvalidServer("kein Server".into()),
                        ),
                    };
                    let result = match server_id {
                        Ok(id) => session
                            .login_password(&id, &user, &password)
                            .await
                            .map(|_| id),
                        Err(e) => Err(e),
                    };
                    post(move |app| match result {
                        Ok(id) => {
                            app.st.borrow_mut().server_id = Some(id);
                            app.login_ok();
                            let _ = reply.send(json!({"ok": true}));
                        }
                        Err(e) => {
                            let _ = reply.send(json!({"ok": false, "error": e.to_string()}));
                        }
                    });
                });
            }
            "quick_connect" => {
                if let Some(u) = msg["server"].as_str() {
                    // Server zuerst hinzufügen, dann Quick Connect starten.
                    let session = self.session.clone();
                    let url = u.to_owned();
                    self.rt.spawn(async move {
                        let r = session.add_server(&url).await;
                        post(move |app| match r {
                            Ok(e) => {
                                app.st.borrow_mut().server_id = Some(e.id.clone());
                                app.show_login(e.id, None);
                                app.start_quick_connect();
                                let _ = reply.send(json!({"ok": true}));
                            }
                            Err(e) => {
                                let _ = reply.send(json!({"ok": false, "error": e.to_string()}));
                            }
                        });
                    });
                } else {
                    self.start_quick_connect();
                    ok(json!({}));
                }
            }
            "player" => match self.player_automation(msg) {
                Ok(v) => ok(v),
                Err(e) => fail(&e),
            }, // W2-player
            "screenshot" => {
                let Some(path) = msg["path"].as_str() else {
                    return fail("path fehlt");
                };
                match self.screenshot(Path::new(path)) {
                    Ok(info) => ok(info),
                    Err(e) => fail(&e),
                }
            }
            "quit" => {
                ok(json!({}));
                self.shell.close();
            }
            // W2-tvui
            "tv" => match self.tv_automation(msg) {
                Ok(v) => ok(v),
                Err(e) => fail(&e),
            },
            c if c.starts_with("detail_") => self.detail_automation(c, msg, reply), // W2-detail
            // W2-library
            _ => match self.lib_automation(cmd, msg) {
                Some(Ok(v)) => ok(v),
                Some(Err(e)) => fail(&e),
                None => fail("unbekannter Befehl"),
            },
        }
    }

    /// Bildschirmfoto des Fensters; unter Windows aus dem Desktop (mit Video), sonst der UI-Schnappschuss.
    pub fn screenshot(&self, path: &Path) -> Result<Value, String> {
        // W2-detail: LFX_SHOT_UI_ONLY=1 nimmt den UI-Schnappschuss statt des Desktops (parallele Fenster stören sonst)
        #[cfg(windows)]
        let _top = crate::play_auto::TopmostGuard::new(self.shell.hwnd_raw()); // W2-player
        #[cfg(windows)]
        if std::env::var_os("LFX_SHOT_UI_ONLY").is_none()
            && let Some((w, h, rgba)) = crate::capture::capture_hwnd(self.shell.hwnd_raw())
        {
            crate::capture::write_png(path, w, h, &rgba)?;
            return Ok(json!({"width": w, "height": h, "source": "desktop"}));
        }
        let snap = self
            .win()
            .window()
            .take_snapshot()
            .map_err(|e| format!("Schnappschuss nicht möglich: {e}"))?;
        crate::capture::write_png(path, snap.width(), snap.height(), snap.as_bytes())?;
        Ok(json!({"width": snap.width(), "height": snap.height(), "source": "ui-only"}))
    }

    /// Zustand für Tests und Fehlersuche (keine Tokens, keine Adressen).
    pub fn state_json(&self) -> Value {
        let ui = self.win();
        let st = self.st.borrow();
        let focus = st
            .graph
            .focused()
            .map(|t| json!({"zone": t.zone.0, "index": t.index}));
        let item = drop_borrow_item(&st);
        let hero = ui.get_hero();
        let ((pw, ph), dpr) = self.shell.window_info();
        let player = st.play.player.as_ref().map(|p| {
            let s = p.state();
            let (frames, vw, vh) = st.play.pipe.as_ref().map_or((0, 0, 0), |pipe| {
                use std::sync::atomic::Ordering::Relaxed;
                (
                    pipe.stats.frames.load(Relaxed),
                    pipe.stats.width.load(Relaxed),
                    pipe.stats.height.load(Relaxed),
                )
            });
            json!({
                "loaded": s.loaded, "position": s.position, "duration": s.duration,
                "paused": s.paused, "eof": s.eof, "buffering": s.buffering,
                "hwdec": s.hwdec, "error": s.error.as_ref().map(|e| e.text.clone()),
                "video": {"width": s.video.width, "height": s.video.height},
                "frames": frames, "target": {"width": vw, "height": vh},
                "opened": st.play.opened,
            })
        });
        json!({
            "route": st.router.current().name(),
            "depth": st.router.depth(),
            "session": self.session.active().is_some(),
            "design": st.design.slug(),
            "focus": focus,
            "focus_item": item,
            "rows": st.home.rows.iter().map(|r| json!({"title": r.title, "count": r.items.len()})).collect::<Vec<_>>(),
            "hero": {"title": hero.title.to_string(), "has_item": hero.has_item,
                     "has_backdrop": hero.has_backdrop, "has_logo": hero.has_logo,
                     "pool": st.home.pool.len()},
            "home_loading": ui.get_home_loading(),
            "home_error": ui.get_home_error().to_string(),
            "qc_code": ui.get_qc_code().to_string(),
            "login_error": ui.get_login_error().to_string(),
            "add_error": ui.get_add_error().to_string(),
            "users": st.users.iter().map(|u| u.name.clone()).collect::<Vec<_>>(),
            "player": player,
            "tv": self.tv_state_json(), // W2-tvui
            "library": self.lib_state_json(), // W2-library
            "play_ui": self.player_state_json(), // W2-player
            "window": {"width": pw, "height": ph, "dpr": dpr},
            "detail": self.detail_state_json(), // W2-detail
        })
    }
}

fn drop_borrow_item(st: &crate::app::State) -> Value {
    let Some(t) = st.graph.focused() else {
        return Value::Null;
    };
    if !matches!(st.router.current(), Route::Home) {
        return Value::Null;
    }
    let Some(r) = crate::focus::zone_row(t.zone) else {
        return Value::Null;
    };
    st.home
        .rows
        .get(r)
        .and_then(|row| row.items.get(t.index))
        .map_or(
            Value::Null,
            |it| json!({"id": it.id, "kind": format!("{:?}", it.kind), "name": it.name}),
        )
}
