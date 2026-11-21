//! Ende-zu-Ende-Test der Fernsteuerung und von SyncPlay gegen den Mock (Bereich W5-remote).
//!
//! Braucht Fenster, libmpv und `ffmpeg`, daher `#[ignore]`:
//! `cargo test -p lfx-app --test w5_remote -- --ignored --test-threads=1`
//! Screenshots landen in `LFX_SHOT_DIR` (Dateien `remote-*.png`), alle Inhalte sind synthetisch.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use lfx_mock_jellyfin::{DEMO_PASSWORD, DEMO_USER, MockHandle, media, w2_player};
use serde_json::{Value, json};

struct AppProc {
    child: Child,
    conn: BufReader<TcpStream>,
    out: TcpStream,
    _home: tempfile::TempDir,
}

impl Drop for AppProc {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn wait_until(timeout: Duration, mut f: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + timeout;
    while Instant::now() < end {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    f()
}

impl AppProc {
    fn start() -> Self {
        let home = tempfile::tempdir().expect("Wegwerf-Ordner");
        let child = Command::new(env!("CARGO_BIN_EXE_lenzflix"))
            .arg("--home")
            .arg(home.path())
            .args(["--maximized", "--report-interval-ms", "500"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("App starten");
        let info_path = home.path().join("data").join("automation.json");
        assert!(
            wait_until(Duration::from_secs(30), || info_path.is_file()),
            "Automation-Datei erscheint nicht"
        );
        std::thread::sleep(Duration::from_millis(200));
        let info: Value = serde_json::from_slice(&std::fs::read(&info_path).unwrap()).unwrap();
        let port = info["port"].as_u64().unwrap() as u16;
        let stream = TcpStream::connect(("127.0.0.1", port)).expect("verbinden");
        stream
            .set_read_timeout(Some(Duration::from_secs(90)))
            .unwrap();
        let out = stream.try_clone().unwrap();
        let mut me = Self {
            child,
            conn: BufReader::new(stream),
            out,
            _home: home,
        };
        let r = me.cmd(json!({"cmd": "auth", "key": info["key"]}));
        assert_eq!(r["ok"], true);
        me
    }

    fn cmd(&mut self, v: Value) -> Value {
        writeln!(self.out, "{v}").unwrap();
        let mut line = String::new();
        self.conn.read_line(&mut line).expect("Antwort");
        serde_json::from_str(&line).expect("JSON-Antwort")
    }

    fn ok(&mut self, v: Value) -> Value {
        let r = self.cmd(v);
        assert_eq!(r["ok"], true, "{r}");
        r
    }

    fn state(&mut self) -> Value {
        self.ok(json!({"cmd": "state"}))["state"].clone()
    }

    fn op(&mut self, op: &str, extra: Value) {
        let mut m = json!({"cmd": "player", "op": op});
        if let (Some(o), Some(e)) = (m.as_object_mut(), extra.as_object()) {
            o.extend(e.clone());
        }
        self.ok(m);
    }

    fn wait_state(&mut self, secs: u64, what: &str, mut f: impl FnMut(&Value) -> bool) -> Value {
        let end = Instant::now() + Duration::from_secs(secs);
        loop {
            let s = self.state();
            if f(&s) {
                return s;
            }
            assert!(
                Instant::now() < end,
                "Zeitüberschreitung: {what}; Zustand: {}",
                s["remote"]
            );
            std::thread::sleep(Duration::from_millis(120));
        }
    }

    fn wait_playing(&mut self, what: &str) -> Value {
        self.wait_state(40, what, |s| {
            s["route"] == "player"
                && s["player"]["loaded"] == true
                && s["player"]["frames"].as_u64() > Some(0)
        })
    }

    fn shot(&mut self, dir: &Path, name: &str) {
        let path = dir.join(format!("{name}.png"));
        self.ok(json!({"cmd": "screenshot", "path": path}));
    }
}

fn shot_dir() -> (PathBuf, Option<tempfile::TempDir>) {
    match std::env::var_os("LFX_SHOT_DIR") {
        Some(d) => {
            let p = PathBuf::from(d);
            std::fs::create_dir_all(&p).unwrap();
            (p, None)
        }
        None => {
            let t = tempfile::tempdir().unwrap();
            (t.path().to_path_buf(), Some(t))
        }
    }
}

fn general(mock: &MockHandle, name: &str, args: Value) {
    mock.ws_push(
        &json!({"MessageType": "GeneralCommand", "Data": {"Name": name, "Arguments": args}})
            .to_string(),
    );
}

fn playstate(mock: &MockHandle, command: &str, extra: Value) {
    let mut d = json!({"Command": command});
    if let (Some(o), Some(e)) = (d.as_object_mut(), extra.as_object()) {
        o.extend(e.clone());
    }
    mock.ws_push(&json!({"MessageType": "Playstate", "Data": d}).to_string());
}

#[test]
#[ignore = "braucht Fenster, libmpv und ffmpeg; lokal mit --ignored ausführen"]
fn remote_control_and_syncplay() {
    let video_dir = tempfile::tempdir().unwrap();
    let video = video_dir.path().join("rich.mkv");
    if let Err(e) = w2_player::generate_rich_video(&video, 60) {
        eprintln!("übersprungen: kein Testvideo ({e})");
        return;
    }
    media::set_video_file(Some(video));
    w2_player::set_segments(None);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (addr, mock) = rt.block_on(lfx_mock_jellyfin::start());
    let url = format!("http://{addr}");
    let (shots, _keep) = shot_dir();

    let mut app = AppProc::start();
    app.ok(json!({"cmd": "login", "server": url, "user": DEMO_USER, "password": DEMO_PASSWORD}));
    app.wait_state(30, "Startseite", |s| {
        s["route"] == "home" && s["hero"]["has_item"] == true
    });

    // ---- A: Verbindung und Fähigkeiten
    assert!(
        wait_until(Duration::from_secs(20), || {
            mock.ws_connections_open() >= 1 && !mock.capabilities_posts().is_empty()
        }),
        "WebSocket/Fähigkeiten fehlen"
    );
    let caps = mock.capabilities_posts().last().cloned().unwrap();
    assert_eq!(caps["SupportsMediaControl"], true);
    app.wait_state(10, "verbunden", |s| s["remote"]["connected"] == true);

    // ---- B: Nachricht anzeigen
    general(
        &mock,
        "DisplayMessage",
        json!({"Header": "Hinweis", "Text": "Der Film beginnt gleich", "TimeoutMs": "3000"}),
    );
    app.wait_state(10, "Nachricht", |s| {
        s["remote"]["toast"]
            .as_str()
            .is_some_and(|t| t.contains("Der Film beginnt gleich"))
    });
    app.shot(&shots, "remote-01-nachricht");

    // ---- C: Wiedergabe, Pause, Fortsetzen, Spulen
    app.op("play", json!({"kind": "movie", "mode": "restart"}));
    let s = app.wait_playing("Film läuft");
    let id = s["play_ui"]["item_id"].as_str().unwrap().to_owned();
    playstate(&mock, "Pause", json!({}));
    app.wait_state(10, "pausiert", |s| s["player"]["paused"] == true);
    playstate(&mock, "Unpause", json!({}));
    app.wait_state(10, "läuft", |s| s["player"]["paused"] == false);
    playstate(&mock, "Seek", json!({"SeekPositionTicks": 300_000_000}));
    app.wait_state(10, "gesprungen", |s| {
        s["player"]["position"].as_f64() > Some(29.0)
    });
    general(&mock, "SetVolume", json!({"Volume": "40"}));
    app.wait_state(10, "Lautstärke", |s| {
        s["remote"]["log"].to_string().contains("volume")
    });
    general(&mock, "GoHome", json!({}));
    app.wait_state(10, "Startseite", |s| s["route"] == "home");

    // ---- D: Play aus der Ferne mit Startposition
    mock.ws_push(
        &json!({"MessageType": "Play", "Data": {"ItemIds": [id], "PlayCommand": "PlayNow",
            "StartPositionTicks": 100_000_000}})
        .to_string(),
    );
    let s = app.wait_playing("Fernwiedergabe");
    assert_eq!(s["play_ui"]["item_id"], json!(id));
    app.wait_state(10, "Startposition", |s| {
        s["player"]["position"].as_f64() > Some(9.0)
    });
    app.op("cmd", json!({"name": "activity"}));
    std::thread::sleep(Duration::from_millis(350));
    app.shot(&shots, "remote-02-fernwiedergabe");

    general(
        &mock,
        "DisplayContent",
        json!({"ItemId": id, "ItemType": "Movie"}),
    );
    app.wait_state(10, "Detailseite", |s| s["route"] == "detail");
    app.ok(json!({"cmd": "route", "route": "home"}));
    app.wait_state(10, "Startseite", |s| s["route"] == "home");

    // ---- E: Fernsteuerung ausschalten -> Befehle wirken nicht mehr
    app.op("play", json!({"kind": "movie", "mode": "restart"}));
    app.wait_playing("Film läuft erneut");
    let before = mock.capabilities_posts().len();
    app.ok(json!({"cmd": "remote_allow", "value": false}));
    assert!(
        wait_until(Duration::from_secs(20), || mock.capabilities_posts().len()
            > before),
        "neue Fähigkeitsmeldung fehlt"
    );
    assert_eq!(
        mock.capabilities_posts().last().unwrap()["SupportsMediaControl"],
        false
    );
    app.wait_state(10, "wieder verbunden", |s| s["remote"]["connected"] == true);
    playstate(&mock, "Pause", json!({}));
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(
        app.state()["player"]["paused"],
        false,
        "Pause wurde trotz Sperre ausgeführt"
    );
    app.ok(json!({"cmd": "remote_allow", "value": true}));
    app.wait_state(10, "wieder erlaubt", |s| {
        s["remote"]["allow"] == true && s["remote"]["connected"] == true
    });

    // ---- F: SyncPlay
    mock.add_syncplay_group("Filmabend", &["anna"]);
    app.ok(json!({"cmd": "remote_refresh"}));
    app.wait_state(10, "Gruppenliste", |s| {
        s["remote"]["groups"] == json!(["Filmabend"])
    });
    app.op("cmd", json!({"name": "activity"}));
    app.op("menu", json!({"kind": "together"}));
    std::thread::sleep(Duration::from_millis(300));
    app.shot(&shots, "remote-03-menue-gruppen");
    app.op("menu_close", json!({}));
    app.ok(json!({"cmd": "remote_join", "index": 0}));
    let s = app.wait_state(10, "Gruppe beigetreten", |s| {
        s["remote"]["in_group"] == true
    });
    assert_eq!(s["remote"]["group"], "Filmabend");
    app.wait_state(5, "Hinweis", |s| {
        s["remote"]["toast"]
            .as_str()
            .is_some_and(|t| t.contains("beigetreten"))
    });
    // Eigene Pause geht an den Server und kommt als Gruppenbefehl zurück.
    app.op("cmd", json!({"name": "toggle"}));
    app.wait_state(10, "Gruppen-Pause", |s| s["player"]["paused"] == true);
    assert!(mock.syncplay_calls().iter().any(|(a, _)| a == "Pause"));
    app.op("cmd", json!({"name": "activity"}));
    app.op("menu", json!({"kind": "together"}));
    std::thread::sleep(Duration::from_millis(300));
    app.shot(&shots, "remote-04-gruppe-pausiert");
    app.op("menu_close", json!({}));
    app.op("cmd", json!({"name": "toggle"}));
    app.wait_state(10, "Gruppen-Fortsetzen", |s| s["player"]["paused"] == false);
    assert!(mock.syncplay_calls().iter().any(|(a, _)| a == "Unpause"));
    app.ok(json!({"cmd": "remote_leave"}));
    app.wait_state(10, "Gruppe verlassen", |s| s["remote"]["in_group"] == false);
    assert!(mock.syncplay_calls().iter().any(|(a, _)| a == "Leave"));
}
