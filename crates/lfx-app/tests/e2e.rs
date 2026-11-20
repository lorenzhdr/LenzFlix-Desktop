//! Ende-zu-Ende-Tests der App gegen den Mock-Server (kein externes Netz).
//!
//! Sie brauchen ein Fenster, libmpv (`cargo xtask fetch-mpv` oder `vendor/mpv/<ziel>`) und `ffmpeg` für das
//! synthetische Testvideo, daher `#[ignore]`. Lokal ausführen:
//!
//! `cargo test -p lfx-app --test e2e -- --ignored --test-threads=1`
//!
//! Screenshots landen im Ordner aus `LFX_SHOT_DIR` (sonst in einem Wegwerf-Ordner). Alle Inhalte sind synthetisch.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use lfx_mock_jellyfin::{DEMO_PASSWORD, DEMO_USER, MockConfig, Recorded, media};
use serde_json::{Value, json};

/// Es darf immer nur ein Fenster gleichzeitig laufen.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

/// Laufende App; wird beim Verwerfen beendet.
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
        assert_eq!(r["ok"], true, "Anmeldung an der Automation");
        me
    }

    fn cmd(&mut self, v: Value) -> Value {
        writeln!(self.out, "{v}").unwrap();
        let mut line = String::new();
        self.conn.read_line(&mut line).expect("Antwort");
        serde_json::from_str(&line).expect("JSON-Antwort")
    }

    fn state(&mut self) -> Value {
        let r = self.cmd(json!({"cmd": "state"}));
        assert_eq!(r["ok"], true, "{r}");
        r["state"].clone()
    }

    fn nav(&mut self, action: &str) {
        let r = self.cmd(json!({"cmd": "nav", "action": action}));
        assert_eq!(r["ok"], true, "{r}");
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
                "Zeitüberschreitung: {what}; Zustand: {s}"
            );
            std::thread::sleep(Duration::from_millis(150));
        }
    }

    fn shot(&mut self, dir: &Path, name: &str) {
        let path = dir.join(format!("{name}.png"));
        let r = self.cmd(json!({"cmd": "screenshot", "path": path}));
        assert_eq!(r["ok"], true, "Screenshot {name}: {r}");
        println!(
            "Screenshot {name}: {}x{} ({})",
            r["width"], r["height"], r["source"]
        );
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

fn recorded_kinds(h: &lfx_mock_jellyfin::MockHandle) -> Vec<&'static str> {
    h.recorded()
        .iter()
        .map(|r| match r {
            Recorded::PlaybackInfo { .. } => "info",
            Recorded::Playing(_) => "playing",
            Recorded::Progress(_) => "progress",
            Recorded::Stopped(_) => "stopped",
        })
        .collect()
}

#[test]
#[ignore = "braucht Fenster, libmpv und ffmpeg; lokal mit --ignored ausführen"]
fn login_home_play_back() {
    let _lock = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let video_dir = tempfile::tempdir().unwrap();
    let video = video_dir.path().join("test.mp4");
    if let Err(e) = media::generate_test_video(&video, 40) {
        eprintln!("übersprungen: kein Testvideo ({e})");
        return;
    }
    media::set_video_file(Some(video));
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (addr, mock) = rt.block_on(lfx_mock_jellyfin::start());
    let url = format!("http://{addr}");
    let (shots, _keep) = shot_dir();

    let mut app = AppProc::start();
    let s = app.state();
    assert_eq!(s["route"], "add-server");
    app.shot(&shots, "01-server-hinzufuegen");

    // Anmelden mit Name und Passwort (Server wird dabei hinzugefügt).
    let r = app
        .cmd(json!({"cmd": "login", "server": url, "user": DEMO_USER, "password": DEMO_PASSWORD}));
    assert_eq!(r["ok"], true, "{r}");

    // Startseite: mindestens drei Zeilen mit Karten und ein Billboard.
    let s = app.wait_state(30, "Startseite mit Zeilen", |s| {
        s["route"] == "home"
            && s["rows"]
                .as_array()
                .is_some_and(|r| r.iter().filter(|x| x["count"].as_u64() > Some(0)).count() >= 3)
            && s["hero"]["has_item"] == true
    });
    println!("Zeilen: {}", s["rows"]);
    assert!(s["hero"]["pool"].as_u64().unwrap() > 0);
    std::thread::sleep(Duration::from_millis(800));
    app.shot(&shots, "02-startseite");

    // Fokus bewegen: der Fokus wandert vom Billboard in die Zeilen und seitwärts.
    let start_focus = app.state()["focus"].clone();
    app.nav("Down");
    app.nav("Right");
    let moved = app.state()["focus"].clone();
    assert_ne!(start_focus, moved, "Fokus hat sich nicht bewegt");
    app.shot(&shots, "03-fokus");

    // Auf einen Film fokussieren (Serien öffnen die Detailseite).
    let mut found = false;
    'search: for _ in 0..6 {
        for _ in 0..4 {
            if app.state()["focus_item"]["kind"] == "Movie" {
                found = true;
                break 'search;
            }
            app.nav("Right");
        }
        app.nav("Down");
    }
    assert!(found, "kein Film im Fokus gefunden");
    app.nav("Activate");
    // W2-detail: Ein Kartenklick öffnet das Detail-Modal; dort startet Abspielen (Fokus liegt darauf).
    app.wait_state(20, "Detail-Modal", |s| {
        s["detail"]["modal_open"] == true && s["detail"]["loading"] == false
    });
    app.nav("Activate");

    // Wiedergabe: Zeit steigt, Bilder werden gezeichnet.
    let s = app.wait_state(30, "Wiedergabe läuft", |s| {
        s["route"] == "player"
            && s["player"]["loaded"] == true
            && s["player"]["frames"].as_u64() > Some(0)
    });
    let t1 = s["player"]["position"].as_f64().unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    let s2 = app.state();
    let t2 = s2["player"]["position"].as_f64().unwrap();
    assert!(t2 > t1 + 0.5, "Position steigt nicht: {t1} -> {t2}");
    assert_eq!(
        s2["player"]["video"]["width"], 640,
        "Videogröße: {}",
        s2["player"]
    );
    assert!(s2["player"]["target"]["width"].as_u64().unwrap() > 0);
    app.nav("SeekForward"); // zeigt das OSD
    std::thread::sleep(Duration::from_millis(400));
    app.shot(&shots, "04-player-osd");

    // Der Mock hat Playing und Progress aufgezeichnet.
    assert!(
        wait_until(Duration::from_secs(10), || {
            let k = recorded_kinds(&mock);
            k.contains(&"playing") && k.contains(&"progress")
        }),
        "Playing/Progress fehlen: {:?}",
        recorded_kinds(&mock)
    );

    // Zurück: Stopped wird gemeldet, die Startseite ist wieder da.
    app.nav("Back");
    let s = app.wait_state(15, "zurück auf der Startseite", |s| s["route"] == "home");
    assert!(s["rows"].as_array().unwrap().len() >= 3);
    assert!(
        wait_until(Duration::from_secs(10), || recorded_kinds(&mock)
            .contains(&"stopped")),
        "Stopped fehlt: {:?}",
        recorded_kinds(&mock)
    );
    let kinds = recorded_kinds(&mock);
    let first_playing = kinds.iter().position(|k| *k == "playing").unwrap();
    let stopped = kinds.iter().position(|k| *k == "stopped").unwrap();
    assert!(first_playing < stopped, "Reihenfolge: {kinds:?}");
    std::thread::sleep(Duration::from_millis(800));
    app.shot(&shots, "05-zurueck-startseite");

    // Design wechseln (Einstellungen): Seite zeigt sich auch in LENZ+.
    app.cmd(json!({"cmd": "design", "design": "plus"}));
    std::thread::sleep(Duration::from_millis(500));
    app.shot(&shots, "06-design-plus");
    app.cmd(json!({"cmd": "route", "route": "settings"}));
    std::thread::sleep(Duration::from_millis(500));
    app.shot(&shots, "07-einstellungen");
    app.cmd(json!({"cmd": "route", "route": "about"}));
    std::thread::sleep(Duration::from_millis(500));
    app.shot(&shots, "08-ueber");
    media::set_video_file(None);
    let _ = app.cmd(json!({"cmd": "quit"}));
    rt.block_on(mock.shutdown());
}

#[test]
#[ignore = "braucht ein Fenster; lokal mit --ignored ausführen"]
fn quick_connect_shows_code_and_signs_in() {
    let _lock = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (addr, mock) = rt.block_on(lfx_mock_jellyfin::start_with(MockConfig {
        quick_connect_auto_after_polls: Some(2),
        ..MockConfig::default()
    }));
    let (shots, _keep) = shot_dir();
    let mut app = AppProc::start();
    let r = app.cmd(json!({"cmd": "quick_connect", "server": format!("http://{addr}")}));
    assert_eq!(r["ok"], true, "{r}");
    let s = app.wait_state(15, "Quick-Connect-Code", |s| {
        s["route"] == "login" && !s["qc_code"].as_str().unwrap_or("").is_empty()
    });
    println!(
        "Code angezeigt: {} Zeichen",
        s["qc_code"].as_str().unwrap().len()
    );
    app.shot(&shots, "09-quick-connect-code");
    app.wait_state(30, "angemeldet", |s| {
        s["route"] == "home" && s["session"] == true
    });
    let _ = app.cmd(json!({"cmd": "quit"}));
    rt.block_on(mock.shutdown());
}
