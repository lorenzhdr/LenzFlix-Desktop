//! W5-polish: Sichtvergleich aller Seiten in allen Designs gegen den Mock (kein externes Netz) plus Prüfungen
//! der Kopfleisten-Fokuszone und des Überspringen-Knopfs im Player.
//!
//! Braucht Fenster, libmpv und `ffmpeg`, daher `#[ignore]`. Lokal ausführen:
//!
//! `cargo test -p lfx-app --test w5_polish -- --ignored --test-threads=1`
//!
//! Screenshots landen in `LFX_SHOT_DIR` (sonst Wegwerf-Ordner), Dateiname `polish-<LFX_SHOT_TAG><Design>-<Seite>.png`.
//! Alle Inhalte sind synthetisch. Es wird der UI-Schnappschuss genommen (`LFX_SHOT_UI_ONLY`), damit parallel
//! laufende Fenster nicht stören.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use lfx_mock_jellyfin::{DEMO_PASSWORD, DEMO_USER, media, w2_home, w2_player};
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
    fn start(design: &str) -> Self {
        let home = tempfile::tempdir().expect("Wegwerf-Ordner");
        let child = Command::new(env!("CARGO_BIN_EXE_lenzflix"))
            .arg("--home")
            .arg(home.path())
            .args([
                "--maximized",
                "--report-interval-ms",
                "500",
                "--design",
                design,
            ])
            .env("LFX_SHOT_UI_ONLY", "1")
            .env("LFX_RETRO_NO_BOOT", "1")
            .env("LFX_AUDIO_NULL", "1")
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
        let r = self.cmd(v.clone());
        assert_eq!(r["ok"], true, "{v}: {r}");
        r
    }

    fn state(&mut self) -> Value {
        self.ok(json!({"cmd": "state"}))["state"].clone()
    }

    fn nav(&mut self, action: &str, count: u64) {
        self.ok(json!({"cmd": "nav", "action": action, "count": count}));
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
                s["route"]
            );
            std::thread::sleep(Duration::from_millis(150));
        }
    }

    fn route(&mut self, name: &str) {
        self.ok(json!({"cmd": "route", "route": name}));
        std::thread::sleep(Duration::from_millis(1800));
    }

    fn shot(&mut self, dir: &Path, tag: &str, design: &str, page: &str) {
        let w = self.state()["window"].clone();
        println!("{design} {page}: Fenster {w}");
        let path = dir.join(format!("polish-{tag}{design}-{page}.png"));
        let r = self.cmd(json!({"cmd": "screenshot", "path": path}));
        assert_eq!(r["ok"], true, "Screenshot {page}: {r}");
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

/// Kopfleiste: Symbole (Suche, TV, Datensparmodus, Profil) sind per Tastatur erreichbar und lösen aus.
fn check_header_focus(app: &mut AppProc, design: &str, shots: &Path, tag: &str) {
    app.route("series");
    // In die Kopfleiste springen (Hoch bis Zone 0), dann ganz nach rechts.
    app.nav("Up", 12);
    app.nav("Right", 9);
    let s = app.state();
    assert_eq!(
        s["focus"]["zone"], 0,
        "{design}: Fokus liegt nicht in der Kopfleiste: {}",
        s["focus"]
    );
    assert_eq!(
        s["focus"]["index"], 8,
        "{design}: Profil nicht als letztes Ziel erreichbar"
    );
    // Eine Position zurück: Datensparmodus (Panel öffnet), dann Suche.
    app.nav("Left", 3);
    let s = app.state();
    assert_eq!(s["focus"]["index"], 5, "{design}: Suche nicht erreichbar");
    std::thread::sleep(Duration::from_millis(400));
    app.shot(shots, tag, design, "17-kopfleiste-fokus-suche");
    app.nav("Activate", 1);
    app.wait_state(10, "Suche geöffnet", |s| s["route"] == "search");
}

#[test]
#[ignore = "braucht Fenster, libmpv und ffmpeg; lokal mit --ignored ausführen"]
fn polish_sweep_all_designs() {
    let _lock = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let tag = std::env::var("LFX_SHOT_TAG").unwrap_or_default();
    let only = std::env::var("LFX_ONLY_DESIGN").ok();
    let video_dir = tempfile::tempdir().unwrap();
    let video = video_dir.path().join("rich.mkv");
    let have_video = w2_player::generate_rich_video(&video, 60).is_ok();
    if have_video {
        media::set_video_file(Some(video));
        w2_player::set_segments(Some(vec![
            ("Intro".into(), 5.0, 15.0),
            ("Outro".into(), 40.0, 60.0),
        ]));
    }
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

    for design in ["flix", "plus", "prime", "retro"] {
        if only.as_deref().is_some_and(|d| d != design) {
            continue;
        }
        let mut app = AppProc::start(design);
        app.ok(
            json!({"cmd": "login", "server": url, "user": DEMO_USER, "password": DEMO_PASSWORD}),
        );
        app.wait_state(40, "Startseite", |s| {
            s["route"] == "home"
                && s["rows"].as_array().is_some_and(|r| r.len() >= 3)
                && s["hero"]["has_backdrop"] == true
        });
        std::thread::sleep(Duration::from_millis(2500));
        app.shot(&shots, &tag, design, "01-home");
        // Scroll
        app.nav("Down", 3);
        std::thread::sleep(Duration::from_millis(900));
        app.shot(&shots, &tag, design, "02-home-gescrollt");
        let id = app.state()["focus_item"]["id"].as_str().map(str::to_owned);
        app.nav("Up", 6);
        // Menüs der Kopfleiste
        app.ok(json!({"cmd": "home_menu", "menu": 1}));
        std::thread::sleep(Duration::from_millis(500));
        app.shot(&shots, &tag, design, "03-profilmenue");
        app.ok(json!({"cmd": "home_menu", "menu": 2}));
        std::thread::sleep(Duration::from_millis(500));
        app.shot(&shots, &tag, design, "04-datensparmodus");
        app.ok(json!({"cmd": "home_menu", "menu": 0}));

        for (name, page) in [
            ("series", "05-serien"),
            ("movies", "06-filme"),
            ("mylist", "07-meine-liste"),
            ("newpopular", "08-neu-und-beliebt"),
            ("search", "09-suche"),
            ("tvmode", "12-tv-modus"),
            ("settings", "13-einstellungen"),
        ] {
            app.route(name);
            app.shot(&shots, &tag, design, page);
        }
        app.ok(json!({"cmd": "route", "route": "home"}));
        std::thread::sleep(Duration::from_millis(800));

        if let Some(id) = &id {
            app.ok(json!({"cmd": "detail_open", "id": id, "mode": "page"}));
            std::thread::sleep(Duration::from_millis(3000));
            app.shot(&shots, &tag, design, "10-detail");
            app.ok(json!({"cmd": "route", "route": "home"}));
            std::thread::sleep(Duration::from_millis(800));
            app.ok(json!({"cmd": "detail_open", "id": id, "mode": "modal"}));
            std::thread::sleep(Duration::from_millis(2500));
            app.shot(&shots, &tag, design, "11-modal");
            app.ok(json!({"cmd": "detail_close"}));
        }
        // Musik
        let r = app.cmd(json!({"cmd": "media_view", "ctype": "music", "tab": 0}));
        if r["ok"] == true {
            std::thread::sleep(Duration::from_millis(2500));
            app.shot(&shots, &tag, design, "14-musik");
        }

        if have_video {
            app.route("home");
            app.ok(json!({"cmd": "player", "op": "play", "kind": "movie", "mode": "restart"}));
            app.wait_state(40, "Film läuft", |s| {
                s["route"] == "player" && s["player"]["position"].as_f64() > Some(0.5)
            });
            app.ok(json!({"cmd": "player", "op": "seek_to", "value": 6.0}));
            app.wait_state(15, "Überspringen-Knopf", |s| {
                s["play_ui"]["skip_label"] == "Intro überspringen"
            });
            app.nav("Info", 0);
            std::thread::sleep(Duration::from_millis(500));
            app.shot(&shots, &tag, design, "15-player-skip");
            app.ok(json!({"cmd": "player", "op": "menu", "kind": "audio"}));
            std::thread::sleep(Duration::from_millis(500));
            app.shot(&shots, &tag, design, "16-player-menue");
            app.ok(json!({"cmd": "player", "op": "menu_close"}));
            app.nav("Back", 1);
            app.wait_state(15, "zurück", |s| s["route"] == "home");
        }
        check_header_focus(&mut app, design, &shots, &tag);
        let _ = app.cmd(json!({"cmd": "quit"}));
    }
    media::set_video_file(None);
    w2_player::set_segments(None);
    rt.block_on(mock.shutdown());
}
