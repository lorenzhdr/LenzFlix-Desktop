//! W2-detail: Ende-zu-Ende-Test für Hovercard, Detail-Modal, Detailseite und Kontextmenü gegen den Mock.
//!
//! Braucht ein Fenster (kein Video, kein libmpv-Abspielen), daher `#[ignore]`:
//! `cargo test -p lfx-app --test e2e_detail -- --ignored --test-threads=1`
//! Screenshots: Ordner aus `LFX_SHOT_DIR` (sonst Wegwerf-Ordner). Alle Inhalte sind synthetisch.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use lfx_mock_jellyfin::catalog::{movie_id, series_id};
use lfx_mock_jellyfin::{DEMO_PASSWORD, DEMO_USER, Recorded, media};
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
            .args(["--maximized"])
            .env("LFX_SHOT_UI_ONLY", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(if std::env::var_os("LFX_TEST_STDERR").is_some() {
                Stdio::inherit()
            } else {
                Stdio::null()
            })
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
                "Zeitüberschreitung: {what}; Detail: {}",
                s["detail"]
            );
            std::thread::sleep(Duration::from_millis(150));
        }
    }

    fn shot(&mut self, dir: &Path, name: &str) {
        let path = dir.join(format!("detail-{name}.png"));
        let r = self.ok(json!({"cmd": "screenshot", "path": path}));
        println!("Screenshot {name}: {}x{}", r["width"], r["height"]);
    }

    fn open(&mut self, id: &str, mode: &str) {
        self.ok(json!({"cmd": "detail_open", "id": id, "mode": mode}));
    }

    /// Wartet, bis das Detail geladen ist und die Bilder da sind.
    fn wait_detail(&mut self, what: &str) -> Value {
        let s = self.wait_state(20, what, |s| {
            s["detail"]["loading"] == false
                && s["detail"]["title"].is_string()
                && s["detail"]["has_backdrop"] == true
        });
        std::thread::sleep(Duration::from_millis(700));
        s
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

fn hid(prefix: u128, n: u128) -> String {
    format!("{:032x}", (prefix << 16) + n)
}

#[test]
#[ignore = "braucht ein Fenster; lokal mit --ignored ausführen"]
fn hovercard_modal_page_menu() {
    // Testvideo (ffmpeg) für den Start mit Spurwahl; fehlt ffmpeg, entfällt nur diese Prüfung.
    let video_dir = tempfile::tempdir().unwrap();
    let video = video_dir.path().join("test.mp4");
    let have_video = media::generate_test_video(&video, 20).is_ok();
    if have_video {
        media::set_video_file(Some(video));
    }
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (addr, mock) = rt.block_on(lfx_mock_jellyfin::start());
    let url = format!("http://{addr}");
    let (shots, _keep) = shot_dir();
    let mut app = AppProc::start();
    app.ok(json!({"cmd": "login", "server": url, "user": DEMO_USER, "password": DEMO_PASSWORD}));
    app.wait_state(30, "Startseite", |s| {
        s["route"] == "home"
            && s["hero"]["has_item"] == true
            && s["rows"].as_array().is_some_and(|r| r.len() >= 3)
    });
    std::thread::sleep(Duration::from_millis(1200));
    app.shot(&shots, "01-startseite");

    // Hovercard: erst nach der Verzögerung offen, Daten sind da; beim Verlassen schließt sie.
    app.ok(json!({"cmd": "detail_hover", "row": 1, "idx": 1}));
    let s = app.state();
    assert_eq!(
        s["detail"]["hovercard"]["visible"], false,
        "nicht vor der Verzögerung"
    );
    let s = app.wait_state(5, "Hovercard offen", |s| {
        s["detail"]["hovercard"]["open"] == true
    });
    let title = s["detail"]["hovercard"]["title"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(!title.is_empty());
    std::thread::sleep(Duration::from_millis(900));
    app.shot(&shots, "02-hovercard");
    app.ok(json!({"cmd": "detail_unhover", "row": 1, "idx": 1}));
    app.wait_state(5, "Hovercard zu", |s| {
        s["detail"]["hovercard"]["visible"] == false
    });

    // Die Info-Taste öffnet das Modal statt der Hovercard.
    app.nav("Down", 1);
    app.nav("Info", 1);
    let s = app.wait_state(15, "Modal per Info-Taste", |s| {
        s["detail"]["modal_open"] == true && s["detail"]["loading"] == false
    });
    assert_eq!(s["detail"]["surface"], "modal");
    app.nav("Back", 1);
    app.wait_state(5, "Modal zu", |s| s["detail"]["modal_open"] == false);

    // Modal Film: Kopf, Beschreibung, Ähnliche, Info; Fokus per Tastatur nach unten.
    app.open(&movie_id(0), "modal");
    let s = app.wait_detail("Film-Modal");
    assert_eq!(s["detail"]["kind"], "Movie");
    assert!(s["detail"]["similar"].as_u64().unwrap() > 0);
    assert_eq!(s["detail"]["focus_sec"], 0);
    app.shot(&shots, "03-modal-film");
    app.nav("Down", 1);
    let s = app.state();
    assert!(
        s["detail"]["focus_sec"].as_i64().unwrap() >= 1,
        "{}",
        s["detail"]
    );
    app.nav("Down", 3);
    std::thread::sleep(Duration::from_millis(800));
    app.shot(&shots, "04-modal-film-unten");

    // Aktionen: Liste, Daumen, Gesehen mit sofortiger Anzeige, der Server bestätigt (Zustand bleibt).
    app.ok(json!({"cmd": "detail_click", "what": "fav"}));
    assert_eq!(app.state()["detail"]["fav"], true, "sofortige Anzeige");
    app.ok(json!({"cmd": "detail_click", "what": "like"}));
    app.ok(json!({"cmd": "detail_click", "what": "played"}));
    std::thread::sleep(Duration::from_millis(800));
    let s = app.state();
    assert_eq!(s["detail"]["fav"], true);
    assert_eq!(s["detail"]["like"], true);
    assert_eq!(s["detail"]["played"], true);
    app.nav("Back", 1);

    // Serie: Staffeln und Folgen, Staffelwechsel.
    app.open(&series_id(0), "modal");
    let s = app.wait_detail("Serien-Modal");
    assert_eq!(s["detail"]["kind"], "Series");
    assert_eq!(s["detail"]["seasons"], 2);
    assert_eq!(s["detail"]["episodes"], 3);
    std::thread::sleep(Duration::from_millis(500));
    app.nav("Down", 1);
    std::thread::sleep(Duration::from_millis(900));
    app.shot(&shots, "05-modal-serie-folgen");
    app.ok(json!({"cmd": "detail_season", "index": 1}));
    let s = app.wait_state(10, "zweite Staffel", |s| s["detail"]["season_idx"] == 1);
    assert_eq!(s["detail"]["episodes"], 3);
    app.nav("Back", 1);

    // Kontextmenü: Einträge wie im Web (ohne Teilen), Auswahl schaltet den Favoriten.
    app.ok(json!({"cmd": "detail_menu", "id": movie_id(1)}));
    let s = app.state();
    assert_eq!(s["detail"]["menu"]["open"], true);
    let entries = s["detail"]["menu"]["entries"].as_array().unwrap();
    assert!(
        entries
            .iter()
            .any(|e| e.as_str().unwrap().starts_with("Zu Meine Liste"))
    );
    assert!(entries.iter().any(|e| e == "Details"));
    assert!(
        !entries
            .iter()
            .any(|e| e.as_str().unwrap().contains("Teilen")),
        "Teilen entfällt"
    );
    app.shot(&shots, "06-kontextmenue");
    app.nav("Down", 1);
    app.nav("Activate", 1);
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(app.state()["detail"]["menu"]["open"], false);

    // Detailseite: Film mit Spurwahl, danach Serie, Folge und Person.
    app.open(&movie_id(2), "page");
    let s = app.wait_detail("Detailseite Film");
    assert_eq!(s["route"], "detail");
    assert_eq!(s["detail"]["surface"], "page");
    assert!(s["detail"]["audio"].as_str().unwrap().contains("Deutsch"));
    app.shot(&shots, "07-seite-film");
    app.ok(json!({"cmd": "detail_cycle", "kind": 1}));
    app.ok(json!({"cmd": "detail_cycle", "kind": 2}));
    let s = app.state();
    assert!(
        s["detail"]["audio"].as_str().unwrap().contains("Englisch"),
        "{}",
        s["detail"]
    );
    assert!(!s["detail"]["sub"].as_str().unwrap().is_empty());
    app.shot(&shots, "08-seite-film-spuren");
    // Abspielen übergibt Ton und Untertitel an den Player (die Startmeldung enthält die gewählte Tonspur).
    app.ok(json!({"cmd": "detail_click", "what": "play"}));
    if have_video {
        assert!(
            wait_until(Duration::from_secs(20), || mock.recorded().iter().any(
                |r| matches!(r, Recorded::Playing(v) if v["AudioStreamIndex"] == 2)
            )),
            "Startmeldung ohne gewählte Tonspur: {:?}",
            mock.recorded()
        );
    }
    app.nav("Back", 1);
    app.wait_state(15, "zurück", |s| {
        s["route"] == "detail" || s["route"] == "home"
    });

    app.open(&series_id(0), "page");
    app.wait_detail("Detailseite Serie");
    app.nav("Down", 2);
    std::thread::sleep(Duration::from_millis(900));
    app.shot(&shots, "09-seite-serie");
    // Folge 2 der Staffel 2: öffnet die Serie mit Staffel 2 und fokussierter Folge.
    app.open(&hid(0x40, 12), "page");
    let s = app.wait_detail("Detailseite Folge");
    assert_eq!(s["detail"]["season_idx"], 1);
    assert_eq!(
        s["detail"]["focus_sec"], 3,
        "Folge hat den Fokus: {}",
        s["detail"]
    );
    app.open(&hid(0x60, 1), "page");
    let s = app.wait_state(15, "Personenseite", |s| {
        s["detail"]["kind"] == "Person" && s["detail"]["loading"] == false
    });
    assert!(s["detail"]["similar"].as_u64().unwrap() > 0);
    std::thread::sleep(Duration::from_millis(1200));
    app.shot(&shots, "10-seite-person");
    media::set_video_file(None);
    app.ok(json!({"cmd": "quit"}));
    rt.block_on(mock.shutdown());
}
