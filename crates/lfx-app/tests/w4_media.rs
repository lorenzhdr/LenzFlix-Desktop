//! Ende-zu-Ende-Lauf der Medienseiten (W4-media: Musik, Buecher, Fotos, Live-TV) gegen den Mock mit
//! `set_media_views(true)`. Live-TV ist nur gegen den Mock pruefbar. Braucht ein Fenster, daher `#[ignore]`:
//!
//! `cargo test -p lfx-app --test w4_media -- --ignored --test-threads=1`
//!
//! Screenshots landen in `LFX_SHOT_DIR`. Alle Inhalte sind synthetisch.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use lfx_mock_jellyfin::{DEMO_PASSWORD, DEMO_USER};
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
            .env("LFX_AUDIO_NULL", "1")
            .env("LFX_SLIDE_MS", "400")
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

    fn ok(&mut self, v: Value) {
        let r = self.cmd(v);
        assert_eq!(r["ok"], true, "{r}");
    }

    fn state(&mut self) -> Value {
        let r = self.cmd(json!({"cmd": "state"}));
        assert_eq!(r["ok"], true, "{r}");
        r["state"].clone()
    }

    fn nav(&mut self, action: &str, count: u32) {
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
                s["media"]
            );
            std::thread::sleep(Duration::from_millis(150));
        }
    }

    fn shot(&mut self, dir: &Path, name: &str) {
        std::thread::sleep(Duration::from_millis(600));
        let path = dir.join(format!("media-{name}.png"));
        let r = self.cmd(json!({"cmd": "media_shot", "path": path}));
        assert_eq!(r["ok"], true, "Screenshot {name}: {r}");
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

fn m<'a>(s: &'a Value, k: &str) -> &'a Value {
    &s["media"][k]
}

fn n(s: &Value, k: &str) -> u64 {
    m(s, k).as_u64().unwrap_or(0)
}

fn loaded(s: &Value) -> bool {
    !m(s, "loading").as_bool().unwrap_or(true)
        && (n(s, "cards") + n(s, "tracks") + n(s, "guide_rows") > 0 || m(s, "view") == "Book")
}

impl AppProc {
    fn ev(&mut self, kind: &str, a: i32, b: i32) {
        self.ok(json!({"cmd": "media_event", "kind": kind, "a": a, "b": b}));
    }

    fn open_view(&mut self, ctype: &str, route: &str, min_items: u64) -> Value {
        self.ok(json!({"cmd": "media_view", "ctype": ctype}));
        let route = route.to_owned();
        self.wait_state(20, "Medienseite", move |s| {
            m(s, "route") == &json!(route)
                && loaded(s)
                && (n(s, "cards") + n(s, "tracks") + n(s, "guide_rows") >= min_items)
        })
    }

    fn tab(&mut self, i: i32, min_items: u64) -> Value {
        self.ev("chip", i, 0);
        let want = i as u64;
        self.wait_state(20, "Reiter", move |s| {
            n(s, "tab") == want
                && loaded(s)
                && (n(s, "cards") + n(s, "tracks") + n(s, "guide_rows") >= min_items)
        })
    }
}

#[test]
#[ignore = "braucht ein Fenster und libmpv; lokal mit --ignored ausführen"]
fn media_pages() {
    // kurze Titel, damit das Ende eines Titels schnell erreicht wird
    // SAFETY: einziger Test dieser Datei; gesetzt vor dem Start des Mocks und der Kindprozesse.
    unsafe { std::env::set_var("LFX_MOCK_TRACK_SECS", "2") };
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (addr, mock) = rt.block_on(lfx_mock_jellyfin::start());
    mock.set_media_views(true);
    let url = format!("http://{addr}");
    let (shots, _keep) = shot_dir();

    let mut app = AppProc::start();
    let r = app
        .cmd(json!({"cmd": "login", "server": url, "user": DEMO_USER, "password": DEMO_PASSWORD}));
    assert_eq!(r["ok"], true, "{r}");
    app.wait_state(30, "Startseite", |s| s["route"] == "home");
    assert!(
        app.state().get("media").is_some(),
        "Binärdatei ohne Medienseiten"
    );

    // ---- Musik: Alben
    let s = app.open_view("music", "music", 3);
    assert_eq!(n(&s, "cards"), 3, "3 Alben");
    std::thread::sleep(Duration::from_millis(1200));
    app.shot(&shots, "music-albums");
    // Tastatur: nach rechts im Inhalt, hoch in den Kopf, wieder hinunter
    app.nav("Right", 1);
    let s = app.state();
    assert_eq!((n(&s, "zone"), n(&s, "index")), (1, 1));
    app.nav("Left", 1);
    app.nav("Up", 1);
    assert_eq!(n(&app.state(), "zone"), 0);
    app.nav("Down", 1);
    assert_eq!(n(&app.state(), "zone"), 1);
    // ---- Künstler
    let s = app.tab(1, 2);
    assert_eq!(n(&s, "cards"), 2);
    std::thread::sleep(Duration::from_millis(1000));
    app.shot(&shots, "music-artists");
    // ---- Genres
    let s = app.tab(3, 1);
    assert!(n(&s, "cards") >= 1);
    // ---- Titel: Wiedergabe, Ende eines Titels startet den nächsten
    let s = app.tab(2, 10);
    assert_eq!(n(&s, "tracks"), 10, "10 Titel");
    app.ev("track", 0, 0);
    let s = app.wait_state(20, "Wiedergabe", |s| {
        m(s, "audio")["active"] == true && m(s, "audio")["mpv_loaded"] == true
    });
    let first_id = m(&s, "audio")["id"].clone();
    assert_eq!(m(&s, "audio")["queue"], 10);
    app.wait_state(20, "Position", |s| {
        m(s, "audio")["position"].as_f64().unwrap_or(0.0) > 0.4
    });
    app.shot(&shots, "music-tracks-bar");
    let s = app.wait_state(30, "nächster Titel", |s| {
        m(s, "audio")["id"] != first_id && m(s, "audio")["active"] == true
    });
    assert_ne!(m(&s, "audio")["id"], first_id);
    // Medientaste: Pause und weiter
    app.ok(json!({"cmd": "media_action", "action": "PlayPause"}));
    app.wait_state(10, "Pause", |s| m(s, "audio")["paused"] == true);
    app.ok(json!({"cmd": "media_action", "action": "PlayPause"}));
    app.wait_state(10, "weiter", |s| m(s, "audio")["paused"] == false);
    // Zufall, Wiederholen, Vollbild-Player
    app.ev("bar", 3, 0);
    app.ev("bar", 4, 0);
    let s = app.state();
    assert_eq!(m(&s, "audio")["shuffle"], true);
    assert_eq!(m(&s, "audio")["repeat"], 1);
    app.ev("np-open", 1, 0);
    app.wait_state(10, "Vollbild", |s| m(s, "audio")["full"] == true);
    app.shot(&shots, "music-fullplayer");
    app.nav("Back", 1);
    app.wait_state(10, "Vollbild zu", |s| m(s, "audio")["full"] == false);
    // Seitenwechsel: die Musik läuft weiter, die Leiste bleibt
    app.ok(json!({"cmd": "route", "route": "home"}));
    app.wait_state(20, "Startseite", |s| s["route"] == "home");
    assert_eq!(m(&app.state(), "audio")["active"], true);
    app.shot(&shots, "music-bar-home");

    // ---- Album und Instant Mix
    app.ok(json!({"cmd": "media_view", "ctype": "music"}));
    app.wait_state(20, "Musik", |s| {
        m(s, "route") == "music" && n(s, "cards") == 3
    });
    app.ev("card", 0, 0);
    let s = app.wait_state(20, "Album", |s| {
        m(s, "route") == &json!("album") && n(s, "tracks") >= 3
    });
    assert!(n(&s, "tracks") >= 3);
    std::thread::sleep(Duration::from_millis(800));
    app.shot(&shots, "music-album");
    app.ev("chip", 2, 0); // Instant Mix (Chips: Abspielen, Zufall, Instant Mix)
    app.wait_state(20, "Mix", |s| m(s, "audio")["queue"].as_u64() >= Some(5));

    // ---- Wiedergabelisten anlegen, füllen, ansehen
    app.ev("back", 0, 0);
    app.wait_state(20, "zurück", |s| m(s, "route") == &json!("music"));
    let s = app.tab(4, 1);
    assert_eq!(n(&s, "cards"), 1, "nur die Karte „Neue Wiedergabeliste“");
    app.ev("card", 0, 0);
    app.wait_state(10, "Namensabfrage", |s| m(s, "prompt") == &json!(true));
    app.ok(json!({"cmd": "media_text", "text": "Testliste"}));
    app.ev("prompt-ok", 0, 0);
    app.wait_state(20, "Liste angelegt", |s| n(s, "cards") == 2);
    assert_eq!(mock.playlists()[0].0, "Testliste");
    // Titel hinzufügen: Reiter Titel, Kontextmenü, „Zur Wiedergabeliste …“
    app.tab(2, 10);
    app.ev("track-menu", 1, 0);
    let s = app.wait_state(10, "Titelmenü", |s| m(s, "menu").is_array());
    assert!(
        m(&s, "menu")
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e == "Als Nächstes spielen")
    );
    app.ev("menu-pick", 3, 0);
    app.wait_state(10, "Listenwahl", |s| {
        m(s, "menu")
            .as_array()
            .is_some_and(|a| a.iter().any(|e| e == "Testliste"))
    });
    app.ev("menu-pick", 0, 0);
    let ok = wait_until(Duration::from_secs(10), || mock.playlists()[0].1.len() == 1);
    assert!(ok, "Titel in der Liste");
    // Liste öffnen
    app.tab(4, 2);
    app.ev("card", 1, 0);
    app.wait_state(20, "Liste", |s| {
        m(s, "route") == &json!("playlist") && n(s, "tracks") == 1
    });
    app.shot(&shots, "music-playlist");
    app.ok(json!({"cmd": "media_action", "action": "Stop"}));
    app.wait_state(10, "Stopp", |s| m(s, "audio")["active"] == false);

    // ---- Bücher
    let s = app.open_view("books", "books", 3);
    assert_eq!(n(&s, "cards"), 3);
    std::thread::sleep(Duration::from_millis(1000));
    app.shot(&shots, "books-list");
    app.ev("card", 0, 0);
    app.wait_state(20, "Buch", |s| {
        m(s, "route") == &json!("book") && m(s, "view") == &json!("Book")
    });
    std::thread::sleep(Duration::from_millis(800));
    app.shot(&shots, "books-detail");
    app.ev("book-open", 0, 0);
    let s = app.wait_state(20, "Buch geladen", |s| {
        m(s, "opened_files")
            .as_array()
            .is_some_and(|a| !a.is_empty())
    });
    let name = m(&s, "opened_files")[0].as_str().unwrap().to_owned();
    assert!(
        name.starts_with("buch-") && name.ends_with(".epub"),
        "{name}"
    );

    // ---- Fotos
    app.ok(json!({"cmd": "route", "route": "home"}));
    app.wait_state(20, "Startseite", |s| s["route"] == "home");
    let s = app.open_view("homevideos", "photos", 3);
    assert_eq!(n(&s, "cards"), 3, "1 Album und 2 Fotos");
    assert_eq!(m(&s, "kinds")[0], json!("PhotoAlbum"));
    std::thread::sleep(Duration::from_millis(1000));
    app.shot(&shots, "photos-grid");
    app.ev("card", 0, 0);
    app.wait_state(20, "Fotoalbum", |s| {
        m(s, "route") == &json!("photos") && n(s, "cards") == 3 && m(s, "title") != &json!("Fotos")
    });
    app.ev("card", 0, 0);
    app.wait_state(20, "Betrachter", |s| {
        m(s, "viewer").is_object() && m(s, "viewer_has_image") == &json!(true)
    });
    app.shot(&shots, "photos-viewer");
    app.ev("viewer-zoom", 3, 0);
    let s = app.state();
    assert!(m(&s, "viewer")["zoom"].as_f64().unwrap() > 1.5);
    app.nav("Right", 1);
    assert!(m(&app.state(), "viewer")["zoom"].as_f64().unwrap() > 1.0);
    app.shot(&shots, "photos-zoom");
    app.ev("viewer-zoom", -20, 0);
    app.nav("Right", 1);
    app.wait_state(10, "nächstes Foto", |s| m(s, "viewer")["index"] == 1);
    app.ev("viewer", 4, 0); // Diashow
    app.wait_state(10, "Diashow", |s| {
        m(s, "viewer")["slide"] == true && m(s, "viewer")["index"] == 2
    });
    app.ev("viewer", 4, 0);
    app.nav("Back", 1);
    app.wait_state(10, "Betrachter zu", |s| m(s, "viewer").is_null());

    // ---- Live-TV (nur gegen den Mock prüfbar)
    app.ok(json!({"cmd": "route", "route": "home"}));
    app.wait_state(20, "Startseite", |s| s["route"] == "home");
    let s = app.open_view("livetv", "livetv", 4);
    assert_eq!(n(&s, "cards"), 4, "4 Sender");
    std::thread::sleep(Duration::from_millis(1000));
    app.shot(&shots, "livetv-channels");
    let s = app.tab(1, 4);
    assert_eq!(n(&s, "guide_rows"), 4);
    assert!(m(&s, "guide_cells")[0].as_u64().unwrap() >= 3);
    std::thread::sleep(Duration::from_millis(500));
    app.shot(&shots, "livetv-guide");
    // Sendung wählen: Menü, Aufnehmen
    app.nav("Right", 1);
    app.nav("Activate", 1);
    let s = app.wait_state(10, "Sendungsmenü", |s| m(s, "menu").is_array());
    assert!(
        m(&s, "menu")
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e == "Aufnehmen")
    );
    app.shot(&shots, "livetv-guide-menu");
    app.ev("menu-pick", 1, 0);
    assert!(
        wait_until(Duration::from_secs(10), || mock.timers().len() == 1),
        "Timer angelegt"
    );
    // Timer-Reiter, Timer löschen
    let s = app.tab(3, 1);
    assert_eq!(n(&s, "timers"), 1);
    app.shot(&shots, "livetv-timers");
    app.ev("track-menu", 0, 0);
    app.wait_state(10, "Timermenü", |s| m(s, "menu").is_array());
    app.ev("menu-pick", 0, 0);
    assert!(
        wait_until(Duration::from_secs(10), || mock.timers().is_empty()),
        "Timer gelöscht"
    );
    // Aufnahmen
    let s = app.tab(2, 2);
    assert_eq!(n(&s, "cards"), 2);
    app.shot(&shots, "livetv-recordings");
    // Live-Wiedergabe: Sender öffnen
    app.tab(0, 4);
    app.ev("card", 0, 0);
    app.wait_state(20, "Player", |s| s["route"] == "player");
    assert!(
        wait_until(Duration::from_secs(10), || !mock
            .open_live_streams()
            .is_empty()),
        "Live-Stream geöffnet"
    );
}

#[test]
#[ignore = "braucht ein Fenster und libmpv; lokal mit --ignored ausführen"]
fn media_designs() {
    // SAFETY: gesetzt vor dem Start von Mock und Kindprozessen (--test-threads=1).
    unsafe { std::env::set_var("LFX_MOCK_TRACK_SECS", "30") };
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (addr, mock) = rt.block_on(lfx_mock_jellyfin::start());
    mock.set_media_views(true);
    let url = format!("http://{addr}");
    let (shots, _keep) = shot_dir();
    let mut app = AppProc::start();
    let r = app
        .cmd(json!({"cmd": "login", "server": url, "user": DEMO_USER, "password": DEMO_PASSWORD}));
    assert_eq!(r["ok"], true, "{r}");
    app.wait_state(30, "Startseite", |s| s["route"] == "home");
    for design in ["retro", "plus", "prime"] {
        app.ok(json!({"cmd": "design", "design": design}));
        app.open_view("music", "music", 3);
        app.tab(2, 10);
        app.ev("track", 2, 0);
        app.wait_state(20, "Wiedergabe", |s| m(s, "audio")["active"] == true);
        std::thread::sleep(Duration::from_millis(800));
        app.shot(&shots, &format!("{design}-music-tracks"));
        app.ok(json!({"cmd": "media_action", "action": "Stop"}));
        app.ok(json!({"cmd": "route", "route": "home"}));
        app.wait_state(20, "Startseite", |s| s["route"] == "home");
    }
    app.ok(json!({"cmd": "design", "design": "retro"}));
    app.open_view("livetv", "livetv", 4);
    app.tab(1, 4);
    std::thread::sleep(Duration::from_millis(500));
    app.shot(&shots, "retro-livetv-guide");
}
