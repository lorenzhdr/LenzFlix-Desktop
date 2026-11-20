//! Ende-zu-Ende-Test der TV-Modus-Seite (W2-tvui) gegen den Mock-Server (kein externes Netz).
//!
//! Sie brauchen ein Fenster, libmpv (`cargo xtask fetch-mpv` oder `vendor/mpv/<ziel>`) und `ffmpeg` für das
//! synthetische Testvideo, daher `#[ignore]`. Lokal ausführen:
//!
//! `cargo test -p lfx-app --test e2e_tvui -- --ignored --test-threads=1`
//!
//! Screenshots landen im Ordner aus `LFX_SHOT_DIR` (sonst in einem Wegwerf-Ordner). Alle Inhalte sind synthetisch.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use lfx_mock_jellyfin::{DEMO_PASSWORD, DEMO_USER, Recorded, media, w2_tvui};
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
            // Feste Sendezeit (UTC) für deterministische Bilder: 20:15 MEZ.
            .env("LFX_TV_NOW", "2026-11-20T19:15")
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

fn tv(app: &mut AppProc, op: &str, value: i64) {
    let r = app.cmd(json!({"cmd": "tv", "op": op, "value": value}));
    assert_eq!(r["ok"], true, "{r}");
}

#[test]
#[ignore = "braucht Fenster, libmpv und ffmpeg; lokal mit --ignored ausführen"]
fn tv_mode_watch_zap_epg_info_error() {
    let _lock = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let video_dir = tempfile::tempdir().unwrap();
    let video = video_dir.path().join("long.mp4");
    // Der TV-Modus startet mitten in der Sendung: das Testvideo muss länger als jede Sendung sein.
    if let Err(e) = w2_tvui::generate_long_test_video(&video, 9000) {
        eprintln!("übersprungen: kein Testvideo ({e})");
        return;
    }
    media::set_video_file(Some(video));
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (addr, mock) = rt.block_on(lfx_mock_jellyfin::start());
    let url = format!("http://{addr}");
    let (shots, _keep) = shot_dir();

    let mut app = AppProc::start();
    let r = app
        .cmd(json!({"cmd": "login", "server": url, "user": DEMO_USER, "password": DEMO_PASSWORD}));
    assert_eq!(r["ok"], true, "{r}");
    app.wait_state(30, "Startseite", |s| {
        s["route"] == "home" && s["hero"]["has_item"] == true
    });

    tv(&mut app, "front", 1);
    // Einstieg über die Route (die Navigationsleiste führt dieselbe Route an).
    app.cmd(json!({"cmd": "route", "route": "tvmode"}));
    let s = app.wait_state(30, "TV läuft", |s| {
        s["route"] == "tvmode"
            && s["tv"]["controller"] == true
            && s["tv"]["phase"] == "Playing"
            && s["tv"]["player"]["frames"].as_u64() > Some(0)
    });
    println!("TV: {}", s["tv"]);
    assert_eq!(s["tv"]["list_rows"], 18);
    let p1 = s["tv"]["player"]["position"].as_f64().unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    let p2 = app.state()["tv"]["player"]["position"].as_f64().unwrap();
    assert!(p2 > p1 + 0.5, "Position steigt nicht: {p1} -> {p2}");
    let rect = app.state()["tv"]["rect"].clone();
    assert!(rect[2].as_f64().unwrap() > 300.0, "Videofenster: {rect}");
    tv(&mut app, "controls", 1);
    std::thread::sleep(Duration::from_millis(600));
    app.shot(&shots, "tvui-01-jetzt");

    // Zappen: Sender +1, Einblendung sichtbar.
    let ch = app.state()["tv"]["channel"].as_i64().unwrap();
    app.nav("Up");
    let s = app.wait_state(20, "Sender gewechselt und spielt", |s| {
        s["tv"]["channel"].as_i64() == Some(ch % 18 + 1) && s["tv"]["phase"] == "Playing"
    });
    println!("nach Zappen: {}", s["tv"]);
    app.shot(&shots, "tvui-02-zappen");

    // Ziffernwahl: 1 und 2 = Sender 12.
    tv(&mut app, "digit", 1);
    tv(&mut app, "digit", 2);
    app.wait_state(20, "Sender 12", |s| s["tv"]["channel"] == 12);

    // Programm-Raster und Info-Karte.
    tv(&mut app, "tab", 1);
    std::thread::sleep(Duration::from_millis(800));
    app.shot(&shots, "tvui-03-programm");
    tv(&mut app, "tab", 0);
    tv(&mut app, "tune", 1);
    app.wait_state(20, "Sender 1", |s| {
        s["tv"]["channel"] == 1 && s["tv"]["phase"] == "Playing"
    });
    tv(&mut app, "tab", 1);
    tv(&mut app, "info", 0);
    app.wait_state(10, "Info-Karte", |s| s["tv"]["info_card"] == true);
    std::thread::sleep(Duration::from_millis(600));
    app.shot(&shots, "tvui-04-infokarte");
    tv(&mut app, "closeinfo", 0);
    tv(&mut app, "tab", 0);

    // Sendepause / Füllprogramm: einen Sender mit Zustandskarte suchen.
    let mut found_card = false;
    for no in 1..=18 {
        tv(&mut app, "tune", no);
        std::thread::sleep(Duration::from_millis(500));
        let s = app.state();
        if matches!(
            s["tv"]["card"].as_str(),
            Some("pause" | "closedown" | "empty" | "next")
        ) {
            println!("Karte auf Sender {no}: {}", s["tv"]["card"]);
            std::thread::sleep(Duration::from_millis(500));
            app.shot(&shots, "tvui-05-sendepause");
            found_card = true;
            break;
        }
    }
    println!("Zustandskarte gefunden: {found_card}");

    // Fehler: PlaybackInfo scheitert -> Fehlerkarte mit Code; danach Erneut versuchen.
    w2_tvui::set_fail_playback(true);
    for no in 1..=18 {
        tv(&mut app, "tune", no);
        let s = app.state();
        if s["tv"]["entry_item"].is_string() {
            break;
        }
    }
    let s = app.wait_state(30, "Fehlerkarte", |s| s["tv"]["card"] == "error");
    println!("Fehler: {}", s["tv"]);
    assert!(
        s["tv"]["card_code"].is_string(),
        "Fehlercode fehlt: {}",
        s["tv"]
    );
    std::thread::sleep(Duration::from_millis(500));
    app.shot(&shots, "tvui-06-fehler");
    w2_tvui::set_fail_playback(false);
    tv(&mut app, "retry", 0);
    app.wait_state(30, "wieder Empfang", |s| s["tv"]["phase"] == "Playing");

    // Vollbild.
    tv(&mut app, "fs", 1);
    std::thread::sleep(Duration::from_millis(1500));
    app.shot(&shots, "tvui-07-vollbild");
    tv(&mut app, "fs", 0);

    // Keine Spuren: der TV-Modus meldet nichts an den Server.
    let kinds = recorded_kinds(&mock);
    assert!(
        !kinds
            .iter()
            .any(|k| matches!(*k, "playing" | "progress" | "stopped")),
        "TV-Modus hat Fortschritt gemeldet: {kinds:?}"
    );

    // Verlassen: Wiedergabe gestoppt, Player freigegeben; danach läuft ein Film normal.
    app.nav("Back");
    let s = app.wait_state(15, "zurück auf der Startseite", |s| s["route"] == "home");
    assert_eq!(s["tv"]["active"], false);
    assert!(s["tv"]["player"].is_null(), "TV-Player nicht freigegeben");
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
    assert!(found, "kein Film im Fokus");
    app.nav("Activate");
    app.wait_state(30, "Film läuft", |s| {
        s["route"] == "player"
            && s["player"]["loaded"] == true
            && s["player"]["frames"].as_u64() > Some(0)
    });
    app.nav("Back");
    app.wait_state(15, "zurück", |s| s["route"] == "home");
    media::set_video_file(None);
    let _ = app.cmd(json!({"cmd": "quit"}));
    rt.block_on(mock.shutdown());
}
