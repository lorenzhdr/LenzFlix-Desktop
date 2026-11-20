//! W5-downloads: Ende-zu-Ende-Test des Offline-Modus gegen den Mock.
//!
//! Ablauf: Download starten (Dialog/Einreihen), Fortschritt sehen, fertig, Netz weg (Tor des Mocks aus),
//! Startseite zeigt die Offline-Bibliothek, lokal abspielen (Fortschritt in die Outbox), Netz wieder da,
//! Abgleich schickt den Fortschritt zum Server. Dazu Screenshots der Downloads-Seite in allen vier Designs.
//!
//! Braucht Fenster, libmpv und `ffmpeg`, daher `#[ignore]`:
//! `cargo test -p lfx-app --test w5_downloads -- --ignored --test-threads=1`
//! Screenshots landen in `LFX_SHOT_DIR` (Dateien `downloads-*.png`); alle Inhalte sind synthetisch.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use lfx_mock_jellyfin::{DEMO_PASSWORD, DEMO_USER, media, w2_player, w5_downloads};
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
            .env("LFX_SHOT_UI_ONLY", "1")
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

    fn wait_state(&mut self, secs: u64, what: &str, mut f: impl FnMut(&Value) -> bool) -> Value {
        let end = Instant::now() + Duration::from_secs(secs);
        loop {
            let s = self.state();
            if f(&s) {
                return s;
            }
            assert!(
                Instant::now() < end,
                "Zeitüberschreitung: {what}; Downloads: {}",
                s["downloads"]
            );
            std::thread::sleep(Duration::from_millis(120));
        }
    }

    fn shot(&mut self, dir: &Path, name: &str) {
        std::thread::sleep(Duration::from_millis(500));
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

fn rec<'a>(s: &'a Value, id: &str) -> Option<&'a Value> {
    s["downloads"]["records"]
        .as_array()?
        .iter()
        .find(|r| r["item_id"] == id)
}

#[test]
#[ignore = "braucht Fenster, libmpv und ffmpeg; lokal mit --ignored ausführen"]
fn download_offline_play_and_sync() {
    let video_dir = tempfile::tempdir().unwrap();
    let video = video_dir.path().join("clip.mkv");
    if let Err(e) = w2_player::generate_rich_video(&video, 20) {
        eprintln!("übersprungen: kein Testvideo ({e})");
        return;
    }
    media::set_video_file(Some(video));
    w5_downloads::serve_video_downloads(true);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (addr, mock) = rt.block_on(lfx_mock_jellyfin::start());
    let gate = rt.block_on(w5_downloads::Gate::start(addr));
    let url = format!("http://{}", gate.addr());
    let (shots, _keep) = shot_dir();
    let movie = lfx_mock_jellyfin::sample_movie_id(0);

    let mut app = AppProc::start();
    app.ok(json!({"cmd": "login", "server": url, "user": DEMO_USER, "password": DEMO_PASSWORD}));
    app.wait_state(30, "Startseite", |s| {
        s["route"] == "home" && s["hero"]["has_item"] == true
    });
    app.wait_state(10, "Manager", |s| s["downloads"]["manager"] == true);

    // ---- Dialog öffnen (Titel laden) und Bildschirmfoto
    app.ok(json!({"cmd": "dl_open", "id": movie}));
    app.wait_state(10, "Dialog", |s| s["downloads"]["dialog"] == true);
    app.shot(&shots, "downloads-dialog-flix");
    // Abbrechen per Taste
    app.ok(json!({"cmd": "nav", "action": "Back"}));
    app.wait_state(5, "Dialog zu", |s| s["downloads"]["dialog"] == false);

    // ---- Download starten, bei 60 KB anhalten lassen, Fortschritt sehen
    mock.hold_next_download_after(60_000);
    app.ok(json!({"cmd": "dl_enqueue", "id": movie, "quality": 0}));
    app.wait_state(30, "Download läuft", |s| {
        rec(s, &movie).is_some_and(|r| r["state"] == "downloading")
    });
    std::thread::sleep(Duration::from_millis(700));
    app.ok(json!({"cmd": "route", "route": "downloads"}));
    let s = app.wait_state(10, "Seite", |s| {
        s["route"] == "downloads"
            && s["downloads"]["rows"]
                .as_array()
                .is_some_and(|r| r.iter().any(|x| x["kind"] == 2))
    });
    let row = s["downloads"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .position(|x| x["kind"] == 2)
        .unwrap();
    // Pause über den linken Knopf der Zeile: der Teilstand bleibt stehen (Fortschritt sichtbar)
    app.ok(json!({"cmd": "dl_click", "kind": "btn", "a": row, "b": 0}));
    let s = app.wait_state(15, "Pause mit Teilstand", |s| {
        rec(s, &movie).is_some_and(|r| r["state"] == "paused" && r["done"].as_u64() > Some(0))
    });
    println!("Teilstand: {}", rec(&s, &movie).unwrap());
    app.shot(&shots, "downloads-paused-flix");
    app.ok(json!({"cmd": "dl_click", "kind": "btn", "a": row, "b": 0}));
    mock.release_download();
    app.wait_state(60, "fertig", |s| {
        rec(s, &movie).is_some_and(|r| r["state"] == "completed")
    });
    app.wait_state(10, "Seite zeigt fertigen Titel", |s| {
        s["downloads"]["rows"]
            .as_array()
            .is_some_and(|r| r.iter().any(|x| x["kind"] == 3))
    });

    // ---- Alle vier Designs
    for design in ["flix", "plus", "prime", "retro"] {
        app.ok(json!({"cmd": "design", "design": design}));
        if design == "retro" {
            app.ok(json!({"cmd": "retro_boot", "action": "skip"}));
        }
        app.shot(&shots, &format!("downloads-library-{design}"));
    }
    app.ok(json!({"cmd": "design", "design": "flix"}));
    // Rückfrage beim Löschen zeigen und abbrechen
    app.ok(json!({"cmd": "nav", "action": "Down", "count": 4}));
    app.ok(json!({"cmd": "nav", "action": "Right"}));
    app.ok(json!({"cmd": "nav", "action": "Activate"}));
    app.wait_state(5, "Rückfrage", |s| s["downloads"]["confirm"] == true);
    app.shot(&shots, "downloads-confirm-flix");
    app.ok(json!({"cmd": "nav", "action": "Back"}));
    app.wait_state(5, "Rückfrage zu", |s| s["downloads"]["confirm"] == false);

    // ---- Netz weg: Startseite zeigt die Offline-Bibliothek
    gate.set_online(false);
    app.ok(json!({"cmd": "dl_check"}));
    app.wait_state(30, "offline erkannt", |s| s["downloads"]["offline"] == true);
    app.ok(json!({"cmd": "route", "route": "home"}));
    app.wait_state(30, "Offline-Startseite", |s| {
        s["route"] == "home"
            && s["rows"]
                .as_array()
                .is_some_and(|r| r.iter().any(|x| x["title"] == "Heruntergeladene Filme"))
    });
    app.shot(&shots, "downloads-offline-home-flix");
    for design in ["plus", "prime", "retro"] {
        app.ok(json!({"cmd": "design", "design": design}));
        if design == "retro" {
            app.ok(json!({"cmd": "retro_boot", "action": "skip"}));
        }
        app.shot(&shots, &format!("downloads-offline-home-{design}"));
    }
    app.ok(json!({"cmd": "design", "design": "flix"}));

    // ---- Offline abspielen
    app.ok(json!({"cmd": "dl_play", "id": movie}));
    app.wait_state(40, "lokale Wiedergabe", |s| {
        s["route"] == "player"
            && s["player"]["loaded"] == true
            && s["player"]["frames"].as_u64() > Some(0)
    });
    app.ok(json!({"cmd": "player", "op": "seek_to", "value": 9.0}));
    std::thread::sleep(Duration::from_millis(1800));
    app.ok(json!({"cmd": "nav", "action": "Back"}));
    let s = app.wait_state(20, "Fortschritt lokal gemerkt", |s| {
        rec(s, &movie).is_some_and(|r| r["position_ticks"].as_i64() > Some(0))
            && s["downloads"]["outbox_pending"].as_u64() >= Some(1)
    });
    println!("offline: {}", s["downloads"]);

    // ---- Netz wieder da: Abgleich
    gate.set_online(true);
    app.ok(json!({"cmd": "dl_check"}));
    app.wait_state(40, "wieder online und abgeglichen", |s| {
        s["downloads"]["offline"] == false
            && s["downloads"]["syncs_done"].as_u64() >= Some(1)
            && s["downloads"]["outbox_pending"] == 0
    });
    let server = mock.server_user_data(&movie);
    assert!(
        server.position_ticks > 0 || server.played,
        "Server kennt den Fortschritt: {server:?}"
    );
    app.shot(&shots, "downloads-online-again-flix");
    drop(app);
    w5_downloads::serve_video_downloads(false);
}
