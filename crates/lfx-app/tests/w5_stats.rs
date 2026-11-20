//! Ende-zu-Ende-Lauf der Statistik-Seite (W5-stats) gegen den Mock (kein externes Netz).
//!
//! Braucht ein Fenster (UI-only-Aufnahmen), daher `#[ignore]`. Lokal ausführen:
//!
//! `cargo test -p lfx-app --test w5_stats -- --ignored --test-threads=1`
//!
//! Screenshots landen im Ordner aus `LFX_SHOT_DIR` (sonst in einem Wegwerf-Ordner). Alle Inhalte sind synthetisch.

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

    fn wait_state(&mut self, secs: u64, what: &str, mut f: impl FnMut(&Value) -> bool) -> Value {
        let end = Instant::now() + Duration::from_secs(secs);
        loop {
            let s = self.state();
            if f(&s) {
                return s;
            }
            assert!(
                Instant::now() < end,
                "Zeitüberschreitung: {what}; Statistik: {}",
                s["stats"]
            );
            std::thread::sleep(Duration::from_millis(120));
        }
    }

    /// Befehl, der gelingen muss.
    fn ok(&mut self, v: Value) -> Value {
        let r = self.cmd(v.clone());
        assert_eq!(r["ok"], true, "{v} -> {r}");
        r
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

fn login(app: &mut AppProc, url: &str) {
    let r = app
        .cmd(json!({"cmd": "login", "server": url, "user": DEMO_USER, "password": DEMO_PASSWORD}));
    assert_eq!(r["ok"], true, "{r}");
    app.wait_state(30, "Startseite", |s| {
        s["route"] == "home" && s["rows"].as_array().is_some_and(|r| r.len() >= 2)
    });
}

impl AppProc {
    fn shot(&mut self, dir: &Path, name: &str) {
        std::thread::sleep(Duration::from_millis(500));
        let path = dir.join(format!("{name}.png"));
        let r = self.cmd(json!({"cmd": "screenshot", "path": path}));
        assert_eq!(r["ok"], true, "Screenshot {name}: {r}");
    }
}

fn wait_status(app: &mut AppProc, want: &'static str) -> Value {
    app.wait_state(20, want, move |s| s["stats"]["status"] == want)
}

#[test]
#[ignore = "braucht ein Fenster; lokal mit --ignored ausführen"]
fn stats_page_flow() {
    let _lock = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (addr, mock) = rt.block_on(lfx_mock_jellyfin::start());
    mock.set_rich_stats(true);
    let url = format!("http://{addr}");
    let (shots, _keep) = shot_dir();
    let mut app = AppProc::start();
    login(&mut app, &url);

    // ---- alle vier Designs: Seite lädt, Diagramme haben Daten
    for design in ["flix", "plus", "prime", "retro"] {
        app.ok(json!({"cmd": "design", "design": design}));
        app.wait_state(5, "Design", |s| s["design"] == design);
        app.ok(json!({"cmd": "route", "route": "stats"}));
        let s = wait_status(&mut app, "ready");
        let st = &s["stats"];
        assert_eq!(s["route"], "stats");
        assert_eq!(st["range_title"], "Letzte 30 Tage", "{st}");
        assert_eq!(st["bars"], 30, "{st}");
        assert!(st["heat_cells"].as_u64().unwrap() > 20, "{st}");
        assert_eq!(st["kpis"].as_array().unwrap().len(), 4, "{st}");
        assert_eq!(st["methods"], 3, "{st}");
        assert_eq!(st["devices"], 3, "{st}");
        let movie = st["top_movies"][0].as_str().unwrap();
        assert_eq!(movie, "Die Nebelkuste", "Titel aufgelöst: {st}");
        assert_ne!(st["top_series"][0], "Unbekannter Titel", "{st}");
        app.shot(&shots, &format!("stats-{design}-oben"));
        if design == "flix" || design == "retro" {
            app.ok(json!({"cmd": "stats_scroll", "delta": -620}));
            app.shot(&shots, &format!("stats-{design}-mitte"));
            app.ok(json!({"cmd": "stats_scroll", "delta": -5000}));
            app.shot(&shots, &format!("stats-{design}-unten"));
        }
    }

    // ---- Zeiträume (Tastatur: Fokus nach rechts, Aktivieren; Maus-Weg per Befehl)
    app.ok(json!({"cmd": "design", "design": "flix"}));
    app.ok(json!({"cmd": "stats_scroll", "delta": 9000}));
    app.ok(json!({"cmd": "nav", "action": "Left", "count": 4}));
    app.ok(json!({"cmd": "nav", "action": "Right"}));
    app.ok(json!({"cmd": "nav", "action": "Activate"}));
    let s = app.wait_state(20, "7 Tage", |s| {
        s["stats"]["range_title"] == "Letzte 7 Tage"
    });
    assert_eq!(s["stats"]["range"], 0);
    assert_eq!(s["stats"]["bars"], 7);
    app.shot(&shots, "stats-flix-7tage");
    for (i, title) in [(2, "Dieses Jahr"), (3, "Gesamtzeit")] {
        app.ok(json!({"cmd": "stats_range", "index": i}));
        let s = app.wait_state(20, title, |s| s["stats"]["range_title"] == title);
        assert!(s["stats"]["bars"].as_u64().unwrap() > 10, "{}", s["stats"]);
        app.shot(&shots, &format!("stats-flix-{i}"));
    }

    // ---- Zurück zur Startseite
    app.ok(json!({"cmd": "nav", "action": "Back"}));
    app.wait_state(10, "Startseite", |s| s["route"] == "home");

    // ---- Dienst nicht eingerichtet: freundlicher Hinweis
    mock.set_branding_css(Some("/* ohne Dienst */"));
    app.ok(json!({"cmd": "route", "route": "stats"}));
    let s = wait_status(&mut app, "no-service");
    assert!(
        s["stats"]["message"]
            .as_str()
            .unwrap()
            .contains("nicht eingerichtet")
    );
    app.shot(&shots, "stats-flix-nicht-eingerichtet");

    // ---- Dienst nicht erreichbar: Fehler, danach Erneut versuchen
    mock.set_branding_css(Some(":root { --lfx-charts-url: \"http://127.0.0.1:1\"; }"));
    app.ok(json!({"cmd": "stats_retry"}));
    let s = wait_status(&mut app, "error");
    assert_eq!(s["stats"]["message"], "Keine Verbindung");
    app.shot(&shots, "stats-retro-fehler-vorher");
    mock.set_branding_css(Some(lfx_mock_jellyfin::DEFAULT_BRANDING));
    app.ok(json!({"cmd": "nav", "action": "Activate"}));
    wait_status(&mut app, "ready");

    // ---- Zeitraum merken, Profilmenü-Einstieg ist über `route` abgedeckt; Profilmenü selbst in home_e2e
}
