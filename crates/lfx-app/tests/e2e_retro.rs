//! W3-retro: Ende-zu-Ende-Lauf des Designs LENZ//84 (Boot, Startseite, Terminal, Overdrive, Regen) gegen den Mock.
//!
//! Sie brauchen ein Fenster, libmpv (`cargo xtask fetch-mpv` oder `vendor/mpv/<ziel>`) und `ffmpeg` für das
//! synthetische Testvideo, daher `#[ignore]`. Lokal ausführen:
//!
//! `cargo test -p lfx-app --test e2e_retro -- --ignored --test-threads=1`
//!
//! Screenshots landen im Ordner aus `LFX_SHOT_DIR` (sonst in einem Wegwerf-Ordner). Alle Inhalte sind synthetisch.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use lfx_mock_jellyfin::{DEMO_PASSWORD, DEMO_USER, w2_home};
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
            .args([
                "--maximized",
                "--design",
                "retro",
                "--report-interval-ms",
                "500",
            ])
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

#[test]
#[ignore = "braucht ein Fenster; lokal mit --ignored ausführen"]
fn retro_boot_home_terminal_and_effects() {
    let _lock = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    w2_home::set_options(w2_home::W2Options {
        fresh_dates: true,
        varied_covers: true,
        trailers: false,
        charts_full: true,
        resume_demo: true,
    });
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (addr, mock) = rt.block_on(lfx_mock_jellyfin::start());
    let url = format!("http://{addr}");
    let (shots, _keep) = shot_dir();

    let mut app = AppProc::start();
    let r = app
        .cmd(json!({"cmd": "login", "server": url, "user": DEMO_USER, "password": DEMO_PASSWORD}));
    assert_eq!(r["ok"], true, "{r}");

    // Boot-Sequenz läuft und lässt sich überspringen.
    let s = app.wait_state(20, "Boot-Bildschirm", |s| s["retro"]["boot"] == true);
    assert_eq!(s["design"], "retro");
    assert_eq!(s["retro"]["sound"], false, "Klang ist standardmäßig aus");
    std::thread::sleep(Duration::from_millis(1700));
    app.shot(&shots, "retro-01-boot");
    assert!(app.state()["retro"]["boot_lines"].as_u64() >= Some(3));
    app.cmd(json!({"cmd": "retro_boot", "action": "skip"}));
    app.wait_state(10, "Boot vorbei", |s| s["retro"]["boot"] == false);

    // Startseite im Retro-Look.
    app.wait_state(30, "Startseite mit Zeilen und Hero", |s| {
        s["route"] == "home"
            && s["rows"].as_array().is_some_and(|r| r.len() >= 3)
            && s["hero"]["has_backdrop"] == true
    });
    std::thread::sleep(Duration::from_millis(1200));
    let s = app.state();
    assert_ne!(
        s["retro"]["hero_file"], "",
        "DOS-Dateiname des Heroes: {}",
        s["retro"]
    );
    assert!(s["retro"]["clock"].as_str().is_some_and(|c| c.len() == 5));
    app.shot(&shots, "retro-02-start");
    for _ in 0..3 {
        app.nav("Down");
    }
    std::thread::sleep(Duration::from_millis(900));
    app.shot(&shots, "retro-03-zeilen");
    for _ in 0..3 {
        app.nav("Up");
    }

    // Terminal: Hilfe, Schalter.
    app.cmd(json!({"cmd": "retro_term", "action": "open"}));
    app.wait_state(5, "Terminal offen", |s| s["retro"]["term_open"] == true);
    app.cmd(json!({"cmd": "retro_term", "action": "line", "text": "hilfe"}));
    app.wait_state(10, "Hilfetext im Terminal", |s| {
        s["retro"]["term_lines"]
            .as_array()
            .is_some_and(|l| l.len() >= 4)
    });
    std::thread::sleep(Duration::from_millis(500));
    app.shot(&shots, "retro-04-terminal");
    app.cmd(json!({"cmd": "retro_term", "action": "line", "text": "crt aus"}));
    app.wait_state(10, "CRT aus", |s| s["retro"]["crt"] == false);
    app.cmd(json!({"cmd": "retro_term", "action": "line", "text": "crt an"}));
    app.wait_state(10, "CRT an", |s| s["retro"]["crt"] == true);
    app.cmd(json!({"cmd": "retro_term", "action": "close"}));
    app.wait_state(5, "Terminal zu", |s| s["retro"]["term_open"] == false);

    // Cheat-Code: Pfeile, B, A (die Tasten bleiben für die Seite erhalten).
    for k in [
        "\u{f700}", "\u{f700}", "\u{f701}", "\u{f701}", "\u{f702}", "\u{f703}", "\u{f702}",
        "\u{f703}", "b", "a",
    ] {
        app.cmd(json!({"cmd": "retro_key", "text": k}));
    }
    app.wait_state(5, "Overdrive", |s| s["retro"]["overdrive"] == true);
    std::thread::sleep(Duration::from_millis(900));
    app.shot(&shots, "retro-05-overdrive");

    // Code-Regen (Bildschirmschoner) und Beenden durch eine Taste.
    app.cmd(json!({"cmd": "retro_rain", "on": true}));
    app.wait_state(5, "Regen", |s| s["retro"]["rain"] == true);
    std::thread::sleep(Duration::from_millis(2500));
    assert!(app.state()["retro"]["rain_cols"].as_u64() >= Some(5));
    app.shot(&shots, "retro-06-regen");
    app.cmd(json!({"cmd": "retro_key", "text": "x"}));
    app.wait_state(5, "Regen weg", |s| s["retro"]["rain"] == false);

    // Meine Liste und Serien im Retro-Look.
    app.cmd(json!({"cmd": "route", "route": "mylist"}));
    app.wait_state(15, "Meine Liste", |s| s["route"] == "mylist");
    std::thread::sleep(Duration::from_millis(1200));
    app.shot(&shots, "retro-07-meine-liste");
    app.cmd(json!({"cmd": "route", "route": "series"}));
    app.wait_state(15, "Serien", |s| s["route"] == "library");
    std::thread::sleep(Duration::from_millis(1500));
    app.shot(&shots, "retro-08-serien");

    // Im TV-Modus gibt es keine Retro-Effekte.
    app.cmd(json!({"cmd": "route", "route": "tvmode"}));
    app.wait_state(15, "TV-Modus", |s| s["route"] == "tvmode");
    assert_eq!(
        app.state()["retro"]["active"],
        false,
        "keine Effekte im TV-Modus"
    );
    std::thread::sleep(Duration::from_millis(1500));
    app.shot(&shots, "retro-09-tv");

    let _ = app.cmd(json!({"cmd": "quit"}));
    rt.block_on(mock.shutdown());
}
