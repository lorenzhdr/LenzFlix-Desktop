//! Ende-zu-Ende-Lauf von Design-Picker und Couch-Modus (W3-picker) gegen den Mock (kein externes Netz).
//!
//! Braucht ein Fenster, daher `#[ignore]`. Lokal ausführen:
//!
//! `cargo test -p lfx-app --test w3_picker -- --ignored --test-threads=1`
//!
//! Screenshots (nur Oberfläche) landen im Ordner aus `LFX_SHOT_DIR`. Alle Inhalte sind synthetisch.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use lfx_mock_jellyfin::{DEMO_PASSWORD, DEMO_USER};
use serde_json::{Value, json};

static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

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
            .env("LFX_NO_CURSOR_POLL", "1")
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

    fn ok(&mut self, v: Value) -> Value {
        let r = self.cmd(v);
        assert_eq!(r["ok"], true, "{r}");
        r
    }

    fn state(&mut self) -> Value {
        self.ok(json!({"cmd": "state"}))["state"].clone()
    }

    fn nav(&mut self, action: &str, source: &str) {
        self.ok(json!({"cmd": "nav", "action": action, "source": source}));
        std::thread::sleep(Duration::from_millis(60));
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
        std::thread::sleep(Duration::from_millis(500));
        let path = dir.join(format!("{name}.png"));
        let r = self.cmd(json!({"cmd": "screenshot", "path": path}));
        assert_eq!(r["ok"], true, "Screenshot {name}: {r}");
        println!("Screenshot {name}: {}x{}", r["width"], r["height"]);
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

fn start_logged_in(url: &str) -> AppProc {
    let mut app = AppProc::start();
    app.ok(json!({"cmd": "login", "server": url, "user": DEMO_USER, "password": DEMO_PASSWORD}));
    app.wait_state(30, "Startseite", |s| {
        s["route"] == "home" && s["rows"].as_array().is_some_and(|r| r.len() >= 2)
    });
    app
}

#[test]
#[ignore = "braucht Fenster; lokal mit --ignored ausführen"]
fn picker_switches_all_four_designs() {
    let _lock = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (addr, mock) = rt.block_on(lfx_mock_jellyfin::start());
    let url = format!("http://{addr}");
    let (shots, _keep) = shot_dir();
    let mut app = start_logged_in(&url);

    // Einstieg über das Profilmenü (Eintrag „Design wählen“, Index 5)
    app.ok(json!({"cmd": "home_menu", "menu": 1}));
    app.ok(json!({"cmd": "home_event", "name": "profile-item", "arg": 5}));
    let s = app.state();
    assert_eq!(s["couch"]["picker_open"], true, "Picker öffnet: {s}");
    app.shot(&shots, "picker-01-dialog");

    // Alle vier Designs per Tastatur wählen: sofortiger Wechsel, Dialog schließt, Wahl bleibt
    for (i, slug) in ["plus", "prime", "retro", "flix"].iter().enumerate() {
        if i > 0 {
            app.ok(json!({"cmd": "picker_open"}));
        }
        let cur = app.state()["couch"]["picker_index"].as_i64().unwrap();
        let want = ["flix", "plus", "prime", "retro"]
            .iter()
            .position(|d| d == slug)
            .unwrap() as i64;
        for _ in 0..(want - cur).unsigned_abs() {
            app.nav(if want > cur { "Right" } else { "Left" }, "keyboard");
        }
        assert_eq!(app.state()["couch"]["picker_index"], want);
        if *slug == "prime" {
            app.shot(&shots, "picker-02-fokus-prime");
        }
        app.nav("Activate", "keyboard");
        let s = app.wait_state(5, "Design gewechselt", |s| s["design"] == *slug);
        assert_eq!(s["couch"]["picker_open"], false, "Dialog schließt");
        assert_eq!(s["couch"]["shown_design"], *slug);
        std::thread::sleep(Duration::from_millis(400));
        app.shot(&shots, &format!("picker-03-wechsel-{slug}"));
    }

    // VIDEOTHEK LENZ ist nur ein Platz: Aktivieren ändert nichts, Dialog bleibt offen
    app.ok(json!({"cmd": "picker_open"}));
    app.ok(json!({"cmd": "picker_choose", "index": 4}));
    let s = app.state();
    assert_eq!(s["design"], "flix");
    assert_eq!(s["couch"]["picker_open"], true);
    app.nav("Back", "keyboard");
    assert_eq!(app.state()["couch"]["picker_open"], false);

    // Einstellungsseite: Picker und Couch-Schalter erreichbar
    app.ok(json!({"cmd": "route", "route": "settings"}));
    std::thread::sleep(Duration::from_millis(400));
    app.shot(&shots, "picker-04-einstellungen");

    let _ = app.cmd(json!({"cmd": "quit"}));
    rt.block_on(mock.shutdown());
}

#[test]
#[ignore = "braucht Fenster; lokal mit --ignored ausführen"]
fn picker_cover_mix_is_daily() {
    let _lock = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let mut app = AppProc::start();
    let a = app.ok(json!({"cmd": "picker_day", "day": "2026-11-20"}))["order"].clone();
    let b = app.ok(json!({"cmd": "picker_day", "day": "2026-11-21"}))["order"].clone();
    let a2 = app.ok(json!({"cmd": "picker_day", "day": "2026-11-20"}))["order"].clone();
    assert_eq!(a, a2, "gleicher Tag, gleiche Mischung");
    assert_ne!(a, b, "anderer Tag, andere Mischung");
    assert_eq!(a.as_array().unwrap().len(), 13);
    let _ = app.cmd(json!({"cmd": "quit"}));
}

#[test]
#[ignore = "braucht Fenster; lokal mit --ignored ausführen"]
fn couch_mode_follows_gamepad_and_mouse() {
    let _lock = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (addr, mock) = rt.block_on(lfx_mock_jellyfin::start());
    let url = format!("http://{addr}");
    let (shots, _keep) = shot_dir();
    let mut app = start_logged_in(&url);
    app.ok(json!({"cmd": "design", "design": "plus"}));
    std::thread::sleep(Duration::from_millis(300));
    app.shot(&shots, "picker-05-couch-aus-plus");

    // Automatisch: Gamepad-Eingabe schaltet den Couch-Look ein (immer LENZFLIX), Maus zurück
    assert_eq!(app.state()["couch"]["active"], false);
    app.nav("Down", "gamepad");
    let s = app.state();
    assert_eq!(s["couch"]["active"], true, "Gamepad -> Couch: {s}");
    assert_eq!(s["couch"]["shown_design"], "flix", "Couch zeigt LENZFLIX");
    assert_eq!(s["design"], "plus", "Wahl bleibt erhalten");
    app.nav("Down", "gamepad");
    app.nav("Right", "gamepad");
    app.shot(&shots, "picker-06-couch-start");
    app.ok(json!({"cmd": "route", "route": "settings"}));
    std::thread::sleep(Duration::from_millis(400));
    app.shot(&shots, "picker-07-couch-einstellungen");
    app.ok(json!({"cmd": "route", "route": "home"}));
    app.ok(json!({"cmd": "input", "source": "mouse"}));
    let s = app.state();
    assert_eq!(s["couch"]["active"], false, "Maus -> normal: {s}");
    assert_eq!(s["couch"]["shown_design"], "plus");

    // Manuell: „an“ gilt ohne Gamepad, „aus“ ignoriert das Gamepad
    app.ok(json!({"cmd": "couch", "mode": "on"}));
    assert_eq!(app.state()["couch"]["active"], true);
    app.ok(json!({"cmd": "input", "source": "mouse"}));
    assert_eq!(app.state()["couch"]["active"], true);
    app.ok(json!({"cmd": "couch", "mode": "off"}));
    app.nav("Down", "gamepad");
    assert_eq!(app.state()["couch"]["active"], false);
    assert_eq!(app.state()["couch"]["shown_design"], "plus");

    // Gamepad bedient den Picker und alle Designs bleiben per Gamepad navigierbar
    app.ok(json!({"cmd": "couch", "mode": "auto"}));
    app.ok(json!({"cmd": "picker_open"}));
    app.nav("Right", "gamepad");
    app.nav("Activate", "gamepad");
    let s = app.wait_state(5, "Design prime per Gamepad", |s| s["design"] == "prime");
    assert_eq!(s["couch"]["active"], true);
    app.nav("Back", "gamepad");

    let _ = app.cmd(json!({"cmd": "quit"}));
    rt.block_on(mock.shutdown());
}
