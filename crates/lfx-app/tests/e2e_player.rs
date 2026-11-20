//! Ende-zu-Ende-Test des Players gegen den Mock (Bereich W2-player): OSD, Menüs, Spuren, Kapitel, Trickplay,
//! Überspringen, „Nächste Folge“, Fortsetzen-Abfrage, Fehler, Vollbild.
//!
//! Braucht Fenster, libmpv und `ffmpeg`, daher `#[ignore]`:
//! `cargo test -p lfx-app --test e2e_player -- --ignored --test-threads=1`
//! Screenshots landen in `LFX_SHOT_DIR` (Dateien `player-*.png`), alle Inhalte sind synthetisch.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use lfx_mock_jellyfin::{DEMO_PASSWORD, DEMO_USER, PlaybackMode, media, w2_player};
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

    fn state(&mut self) -> Value {
        let r = self.cmd(json!({"cmd": "state"}));
        assert_eq!(r["ok"], true, "{r}");
        r["state"].clone()
    }

    fn nav(&mut self, action: &str) {
        let r = self.cmd(json!({"cmd": "nav", "action": action}));
        assert_eq!(r["ok"], true, "{r}");
    }

    /// Befehl `player` mit `op` und optionalen Feldern.
    fn op(&mut self, op: &str, extra: Value) -> Value {
        let mut m = json!({"cmd": "player", "op": op});
        if let (Some(o), Some(e)) = (m.as_object_mut(), extra.as_object()) {
            o.extend(e.clone());
        }
        let r = self.cmd(m);
        assert_eq!(r["ok"], true, "Operation {op}: {r}");
        r
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
            std::thread::sleep(Duration::from_millis(120));
        }
    }

    fn shot(&mut self, dir: &Path, name: &str) {
        let path = dir.join(format!("{name}.png"));
        let r = self.cmd(json!({"cmd": "screenshot", "path": path}));
        assert_eq!(r["ok"], true, "Screenshot {name}: {r}");
        println!("Screenshot {name}: {}x{}", r["width"], r["height"]);
    }

    /// Wiedergabe läuft (geladen, Bilder gezeichnet).
    fn wait_playing(&mut self, what: &str) -> Value {
        self.wait_state(40, what, |s| {
            s["route"] == "player"
                && s["player"]["loaded"] == true
                && s["player"]["frames"].as_u64() > Some(0)
        })
    }

    fn show_osd(&mut self) {
        self.op("cmd", json!({"name": "activity"}));
        std::thread::sleep(Duration::from_millis(350));
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
fn player_osd_menus_skip_upnext_resume() {
    let video_dir = tempfile::tempdir().unwrap();
    let video = video_dir.path().join("rich.mkv");
    if let Err(e) = w2_player::generate_rich_video(&video, 60) {
        eprintln!("übersprungen: kein Testvideo ({e})");
        return;
    }
    media::set_video_file(Some(video));
    // Intro 5–15 s, Abspann ab 40 s (Videolänge 60 s).
    w2_player::set_segments(Some(vec![
        ("Intro".into(), 5.0, 15.0),
        ("Outro".into(), 40.0, 60.0),
    ]));
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

    // ---- A: Film starten, OSD, Spuren, Kapitel
    app.op("play", json!({"kind": "movie", "mode": "restart"}));
    let s = app.wait_playing("Film läuft");
    let ui = &s["play_ui"];
    println!("play_ui: {ui}");
    assert_eq!(ui["audio_tracks"], 2, "zwei Tonspuren");
    assert!(ui["subtitle_tracks"].as_u64() >= Some(1), "Untertitel da");
    assert!(ui["chapters"].as_u64() >= Some(2), "Kapitel vom Server");
    assert_eq!(ui["segments"], 2);
    let hs = app.wait_state(5, "HDR-Plan", |s| s["play_ui"]["hdr_status"] != "");
    assert_eq!(hs["play_ui"]["hdr_status"], "Aus (SDR-Inhalt)");
    assert_eq!(ui["awake"], true, "Bildschirm wird wachgehalten");
    println!("Medientasten verbunden: {}", ui["media_controls"]);
    app.op("seek_to", json!({"value": 2.0}));
    std::thread::sleep(Duration::from_millis(700));
    app.show_osd();
    app.shot(&shots, "player-01-osd");

    // OSD blendet nach Inaktivität aus.
    std::thread::sleep(Duration::from_millis(3600));
    let s = app.state();
    assert_eq!(s["play_ui"]["osd"], false, "OSD sollte ausgeblendet sein");
    app.shot(&shots, "player-02-osd-aus");

    // ---- B: Menüs
    app.show_osd();
    app.op("menu", json!({"kind": "audio"}));
    let s = app.state();
    assert_eq!(s["play_ui"]["menu"]["open"], true);
    assert_eq!(s["play_ui"]["menu"]["items"].as_array().unwrap().len(), 2);
    std::thread::sleep(Duration::from_millis(250));
    app.shot(&shots, "player-03-menue-ton");
    let before = s["play_ui"]["menu"]["selected"].clone();
    app.op("menu_pick", json!({"value": 1}));
    std::thread::sleep(Duration::from_millis(400));
    app.op("menu", json!({"kind": "audio"}));
    let after = app.state()["play_ui"]["menu"]["selected"].clone();
    assert_ne!(before, after, "Tonspur wurde gewechselt");
    app.op("menu_close", json!({}));

    // Untertitel an, an eine Stelle mit Untertitel springen.
    app.op("seek_to", json!({"value": 7.0}));
    app.op("menu", json!({"kind": "subtitles"}));
    let s = app.state();
    let items = s["play_ui"]["menu"]["items"].as_array().unwrap().clone();
    assert!(
        items.len() >= 4,
        "Aus + Spur + Größe + Hintergrund: {items:?}"
    );
    assert_eq!(items[0], "Aus");
    std::thread::sleep(Duration::from_millis(250));
    app.shot(&shots, "player-04-menue-untertitel");
    app.op("menu_pick", json!({"value": 1}));
    std::thread::sleep(Duration::from_millis(300));
    // Größe weiterschalten (Menü bleibt offen), dann schließen.
    app.op("menu", json!({"kind": "subtitles"}));
    let n = app.state()["play_ui"]["menu"]["items"]
        .as_array()
        .unwrap()
        .len();
    app.op("menu_pick", json!({"value": n - 2}));
    let style = app.state()["play_ui"]["sub_style"].clone();
    assert_eq!(style["size"], 125, "Untertitelgröße weitergeschaltet");
    app.op("menu_close", json!({}));

    // Geschwindigkeit und Info
    app.op("menu", json!({"kind": "speed"}));
    app.shot(&shots, "player-05-menue-tempo");
    app.op("menu_pick", json!({"value": 3}));
    assert_eq!(app.state()["play_ui"]["speed"], 1.25);
    app.op("menu", json!({"kind": "info"}));
    app.shot(&shots, "player-06-info");
    app.op("menu_close", json!({}));
    app.op("menu", json!({"kind": "quality"}));
    app.shot(&shots, "player-07-menue-qualitaet");
    app.op("menu_close", json!({}));

    // ---- C: Trickplay beim Spulen
    app.show_osd();
    app.op("scrub", json!({"value": 0.5}));
    app.wait_state(10, "Trickplay-Bild", |s| {
        s["play_ui"]["trick"]["has_image"] == true
    });
    app.op("scrub", json!({"value": 0.5}));
    std::thread::sleep(Duration::from_millis(300));
    app.shot(&shots, "player-08-trickplay");
    app.op("scrub_end", json!({}));

    // ---- D: Intro überspringen
    app.op("seek_to", json!({"value": 6.0}));
    app.wait_state(10, "Überspringen-Knopf", |s| {
        s["play_ui"]["skip_label"] == "Intro überspringen"
    });
    app.show_osd();
    app.shot(&shots, "player-09-intro-ueberspringen");
    app.nav("Activate"); // Enter bei sichtbarem Knopf überspringt
    let s = app.wait_state(10, "Intro übersprungen", |s| {
        s["player"]["position"].as_f64() > Some(15.3)
    });
    let _ = s;
    app.wait_state(5, "Knopf weg", |s| s["play_ui"]["skip_label"] == "");

    // ---- E: Folge starten, Abspann -> „Nächste Folge“
    app.nav("Back");
    app.wait_state(15, "zurück", |s| s["route"] == "home");
    app.op("play", json!({"kind": "episode", "mode": "restart"}));
    let s = app.wait_playing("Folge läuft");
    let first_id = s["play_ui"]["item_id"].clone();
    println!(
        "Folge: {} next={}",
        s["play_ui"]["item_title"], s["play_ui"]["has_next"]
    );
    app.wait_state(10, "Nächste Folge bekannt", |s| {
        s["play_ui"]["has_next"] == true
    });
    app.op("seek_to", json!({"value": 41.0}));
    app.wait_state(10, "Nächste-Folge-Karte", |s| {
        s["play_ui"]["up_next"]["visible"] == true
    });
    app.show_osd();
    app.shot(&shots, "player-10-naechste-folge");
    // Countdown läuft ab -> nächste Folge startet selbst.
    app.wait_state(25, "Folgewiedergabe", |s| {
        s["play_ui"]["item_id"] != first_id && s["player"]["loaded"] == true
    });
    app.wait_playing("nächste Folge läuft");
    app.shot(&shots, "player-11-folge-gewechselt");
    app.nav("Back");
    app.wait_state(15, "zurück", |s| s["route"] == "home");

    // ---- F: Fortsetzen-Abfrage
    app.op("play", json!({"kind": "resume", "mode": "ask"}));
    let s = app.wait_state(15, "Fortsetzen-Abfrage", |s| {
        s["play_ui"]["resume_open"] == true
    });
    println!("Abfrage: {}", s["play_ui"]["resume_text"]);
    assert!(
        s["play_ui"]["resume_text"]
            .as_str()
            .unwrap()
            .starts_with("Fortsetzen bei")
    );
    std::thread::sleep(Duration::from_millis(500));
    app.shot(&shots, "player-12-fortsetzen");
    app.nav("Down"); // Fokus auf „Von vorne“
    app.shot(&shots, "player-13-fortsetzen-vorne");
    // Ladezustand mit Startbild: Server verzögert, dann Auswahl.
    mock.set_delay(Duration::from_millis(4000));
    app.nav("Up");
    app.nav("Activate");
    std::thread::sleep(Duration::from_millis(1200));
    let s = app.state();
    assert_eq!(s["play_ui"]["loading"], true);
    app.shot(&shots, "player-14-ladebild");
    mock.set_delay(Duration::ZERO);
    app.wait_playing("Fortsetzen läuft");
    let pos = app.state()["player"]["position"].as_f64().unwrap();
    assert!(
        pos > 10.0,
        "Fortsetzen springt an die gemerkte Stelle: {pos}"
    );

    // ---- G: Vollbild
    app.op("fullscreen", json!({}));
    std::thread::sleep(Duration::from_millis(900));
    let s = app.state();
    assert_eq!(s["play_ui"]["fullscreen"], true);
    println!("Fenster im Vollbild: {}", s["window"]);
    app.show_osd();
    app.shot(&shots, "player-15-vollbild");
    app.op("fullscreen", json!({}));
    std::thread::sleep(Duration::from_millis(700));
    assert_eq!(app.state()["play_ui"]["fullscreen"], false);
    app.nav("Back");
    app.wait_state(15, "zurück", |s| s["route"] == "home");

    // ---- H: Fehler (keine abspielbare Quelle ohne Umwandlung)
    mock.set_playback_mode(PlaybackMode::ForceTranscode);
    app.op("play", json!({"kind": "movie", "mode": "restart"}));
    let s = app.wait_state(20, "Fehleranzeige", |s| {
        !s["play_ui"]["note"].as_str().unwrap_or("").is_empty()
    });
    println!(
        "Fehler: {} / {}",
        s["play_ui"]["error_code"], s["play_ui"]["note"]
    );
    assert!(!s["play_ui"]["error_code"].as_str().unwrap().is_empty());
    app.shot(&shots, "player-16-fehler");
    app.nav("Back");
    app.wait_state(15, "zurück", |s| s["route"] == "home");
    mock.set_playback_mode(PlaybackMode::Direct);

    media::set_video_file(None);
    w2_player::set_segments(None);
    let _ = app.cmd(json!({"cmd": "quit"}));
    rt.block_on(mock.shutdown());
}
