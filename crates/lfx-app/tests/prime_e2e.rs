//! Ende-zu-Ende-Lauf von LENZPRIME (W3-prime): Kopfleiste, Hero, Zeilen, Vorschau-Karte, Profilmenü, Titelseite gegen den Mock (kein externes Netz), mit Screenshots.
//!
//! Sie brauchen ein Fenster, libmpv (`cargo xtask fetch-mpv` oder `vendor/mpv/<ziel>`) und `ffmpeg` für das
//! synthetische Testvideo, daher `#[ignore]`. Lokal ausführen:
//!
//! `cargo test -p lfx-app --test prime_e2e -- --ignored --test-threads=1`
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
                "--report-interval-ms",
                "500",
                "--design",
                "prime",
            ])
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
        let r = self.cmd(json!({"cmd": "home_shot", "path": path}));
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
#[ignore = "braucht Fenster, libmpv und ffmpeg; lokal mit --ignored ausführen"]
fn prime_home_hover_profile_and_title() {
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
    let s = app.wait_state(30, "Startseite mit Zeilen und Hero", |s| {
        s["route"] == "home"
            && s["rows"].as_array().is_some_and(|r| r.len() >= 3)
            && s["hero"]["has_backdrop"] == true
    });
    assert_eq!(s["design"], "prime", "Design LENZPRIME aktiv");
    // Geometrie: Hero füllt die Breite ab y = 0, sechs Karten je Zeile
    let layout = s["home"]["layout"].clone();
    assert_eq!(layout["bb"][0], 0.0, "{layout}");
    assert_eq!(layout["bb"][1], 0.0, "{layout}");
    std::thread::sleep(Duration::from_millis(1500));
    app.shot(&shots, "prime-01-oben");

    // Scrollen: Kopfleiste wird zur Glasleiste
    for _ in 0..3 {
        app.nav("Down");
    }
    std::thread::sleep(Duration::from_millis(900));
    app.shot(&shots, "prime-02-gescrollt");
    for _ in 0..5 {
        app.nav("Down");
    }
    std::thread::sleep(Duration::from_millis(900));
    app.shot(&shots, "prime-03-weiter-unten");
    for _ in 0..10 {
        app.nav("Up");
    }
    std::thread::sleep(Duration::from_millis(600));

    // Vorschau-Karte über der ersten Karte der ersten Zeile
    app.cmd(json!({"cmd": "detail_hover", "row": 0, "idx": 1}));
    app.wait_state(10, "Vorschau-Karte offen", |s| {
        s["detail"]["hovercard"]["visible"] == true
    });
    std::thread::sleep(Duration::from_millis(900));
    app.shot(&shots, "prime-04-vorschau");
    app.cmd(json!({"cmd": "detail_unhover", "row": 0, "idx": 1}));
    std::thread::sleep(Duration::from_millis(500));

    // Profilmenü und Datensparmodus
    app.cmd(json!({"cmd": "home_menu", "menu": 1}));
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(app.state()["home"]["menu"], 1);
    app.shot(&shots, "prime-05-profil");
    app.cmd(json!({"cmd": "home_menu", "menu": 0}));

    // Kartenaktivierung führt zur Titelseite (kein Info-Fenster)
    app.nav("Down");
    app.nav("Down");
    app.nav("Activate");
    let s = app.wait_state(15, "Titelseite", |s| s["route"] == "detail");
    assert_eq!(s["detail"]["surface"], "page", "{}", s["detail"]);
    app.wait_state(15, "Titel geladen", |s| s["detail"]["has_backdrop"] == true);
    std::thread::sleep(Duration::from_millis(900));
    app.shot(&shots, "prime-06-titelseite");
    app.nav("Back");
    app.wait_state(10, "zurück zur Startseite", |s| s["route"] == "home");

    // Suche im LENZPRIME-Stil
    app.cmd(json!({"cmd": "route", "route": "search"}));
    std::thread::sleep(Duration::from_millis(900));
    app.shot(&shots, "prime-07-suche");

    let _ = app.cmd(json!({"cmd": "quit"}));
    rt.block_on(mock.shutdown());
}
