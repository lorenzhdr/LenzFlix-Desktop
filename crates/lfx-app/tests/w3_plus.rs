//! Ende-zu-Ende-Lauf des Designs LENZ+ (W3-plus) gegen den Mock (kein externes Netz), mit Screenshots.
//!
//! Sie brauchen ein Fenster, libmpv (`cargo xtask fetch-mpv` oder `vendor/mpv/<ziel>`) und `ffmpeg` für das
//! synthetische Testvideo, daher `#[ignore]`. Lokal ausführen:
//!
//! `cargo test -p lfx-app --test w3_plus -- --ignored --test-threads=1`
//!
//! Screenshots landen im Ordner aus `LFX_SHOT_DIR` (sonst in einem Wegwerf-Ordner). Alle Inhalte sind synthetisch.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use lfx_mock_jellyfin::{DEMO_PASSWORD, DEMO_USER, media, w2_home};
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
                "plus",
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
fn plus_home_title_and_play() {
    let _lock = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let video_dir = tempfile::tempdir().unwrap();
    let video = video_dir.path().join("film.mp4");
    let have_video = media::generate_test_video(&video, 40).is_ok();
    if have_video {
        media::set_video_file(Some(video));
    }
    w2_home::set_options(w2_home::W2Options {
        fresh_dates: true,
        varied_covers: true,
        trailers: false,
        charts_full: true,
        resume_demo: true,
    });
    lfx_mock_jellyfin::set_extra_titles(40); // Sammlungen für die Kacheln
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (addr, mock) = rt.block_on(lfx_mock_jellyfin::start());
    let url = format!("http://{addr}");
    let (shots, _keep) = shot_dir();

    let mut app = AppProc::start();
    let r = app
        .cmd(json!({"cmd": "login", "server": url, "user": DEMO_USER, "password": DEMO_PASSWORD}));
    assert_eq!(r["ok"], true, "{r}");
    let s = app.wait_state(30, "Startseite mit Hero und Zeilen", |s| {
        s["route"] == "home"
            && s["design"] == "plus"
            && s["rows"].as_array().is_some_and(|r| r.len() >= 3)
            && s["hero"]["has_backdrop"] == true
            && s["plus"]["brands"].as_u64() >= Some(1)
    });
    println!("Zeilen: {} Kacheln: {}", s["rows"], s["plus"]["brands"]);
    std::thread::sleep(Duration::from_millis(1500));
    app.shot(&shots, "plus-01-start");

    // Fokus: Hero -> Kacheln -> Zeilen
    assert_eq!(
        app.state()["focus"]["zone"],
        1,
        "Fokus startet auf dem Hero"
    );
    app.nav("Down");
    assert_eq!(app.state()["focus"]["zone"], 2, "danach die Kacheln");
    std::thread::sleep(Duration::from_millis(500));
    app.shot(&shots, "plus-02-kacheln-fokus");
    app.nav("Down");
    app.nav("Down");
    std::thread::sleep(Duration::from_millis(900));
    app.shot(&shots, "plus-03-gescrollt");

    // Menü und Kopfleiste
    for _ in 0..8 {
        app.nav("Up");
    }
    app.cmd(json!({"cmd": "home_menu", "menu": 1}));
    std::thread::sleep(Duration::from_millis(400));
    app.shot(&shots, "plus-04-profilmenue");
    app.cmd(json!({"cmd": "home_menu", "menu": 0}));

    // Hero öffnet die Titelseite
    app.cmd(json!({"cmd": "home_event", "name": "hero-info"}));
    let s = app.wait_state(20, "Titelseite", |s| {
        s["route"] == "detail" && s["detail"]["title"].is_string()
    });
    println!("Titelseite: {}", s["detail"]["title"]);
    std::thread::sleep(Duration::from_millis(1500));
    app.shot(&shots, "plus-05-titelseite");

    // Suche, Meine Liste und Serien mit der LENZ+-Kopfleiste
    for (name, shot) in [
        ("search", "plus-06-suche"),
        ("mylist", "plus-07-meine-liste"),
        ("series", "plus-08-serien"),
    ] {
        let r = app.cmd(json!({"cmd": "route", "route": name}));
        assert_eq!(r["ok"], true, "{r}");
        std::thread::sleep(Duration::from_millis(1800));
        app.shot(&shots, shot);
    }

    // Abspielen von der Titelseite
    if have_video {
        app.cmd(json!({"cmd": "route", "route": "home"}));
        app.wait_state(20, "Startseite", |s| {
            s["route"] == "home" && s["hero"]["has_item"] == true
        });
        app.cmd(json!({"cmd": "home_event", "name": "hero-info"}));
        app.wait_state(20, "Titelseite", |s| {
            s["route"] == "detail" && s["detail"]["title"].is_string()
        });
        std::thread::sleep(Duration::from_millis(800));
        app.cmd(json!({"cmd": "detail_click", "what": "play"}));
        app.wait_state(30, "Player", |s| s["route"] == "player");
    }
    media::set_video_file(None);
    let _ = app.cmd(json!({"cmd": "quit"}));
    rt.block_on(mock.shutdown());
}
