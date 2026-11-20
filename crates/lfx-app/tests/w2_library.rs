//! Ende-zu-Ende-Lauf der Bibliotheksseiten (Serien/Filme, Meine Liste, Neu und beliebt, Suche, Sammlungen) gegen
//! den Mock mit zusätzlichen erfundenen Titeln. Braucht ein Fenster, daher `#[ignore]`:
//!
//! `cargo test -p lfx-app --test w2_library -- --ignored --test-threads=1`
//!
//! Screenshots landen in `LFX_SHOT_DIR` (sonst in einem Wegwerf-Ordner). Alle Inhalte sind synthetisch.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use lfx_mock_jellyfin::{DEMO_PASSWORD, DEMO_USER, VIEW_BOXSETS};
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
                s["library"]
            );
            std::thread::sleep(Duration::from_millis(150));
        }
    }

    fn shot(&mut self, dir: &Path, name: &str) {
        std::thread::sleep(Duration::from_millis(600));
        let path = dir.join(format!("library-{name}.png"));
        let r = self.cmd(json!({"cmd": "lib_shot", "path": path}));
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

fn items(s: &Value) -> u64 {
    s["library"]["items"].as_u64().unwrap_or(0)
}

#[test]
#[ignore = "braucht ein Fenster; lokal mit --ignored ausführen"]
fn library_pages() {
    lfx_mock_jellyfin::set_extra_titles(120);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (addr, mock) = rt.block_on(lfx_mock_jellyfin::start());
    lfx_mock_jellyfin::set_extra_titles(0);
    let url = format!("http://{addr}");
    // Einige Favoriten (erfundene Ids der Zusatztitel: Präfix 0x15 Filme, 0x25 Serien)
    for n in 1..=7u128 {
        mock.set_favorite_item(&format!("{:032x}", (0x15u128 << 16) + n), true);
    }
    mock.set_favorite_item(&format!("{:032x}", (0x25u128 << 16) + 1), true);
    let (shots, _keep) = shot_dir();

    let mut app = AppProc::start();
    let r = app
        .cmd(json!({"cmd": "login", "server": url, "user": DEMO_USER, "password": DEMO_PASSWORD}));
    assert_eq!(r["ok"], true, "{r}");
    app.wait_state(30, "Startseite", |s| s["route"] == "home");
    // Schutz gegen eine fremde Binärdatei: der Zustand muss den Bibliotheksschlüssel kennen.
    assert!(
        app.state().get("library").is_some(),
        "Binärdatei ohne Bibliotheksseiten"
    );

    // ---- Meine Liste
    app.ok(json!({"cmd": "mylist"}));
    let s = app.wait_state(20, "Meine Liste", |s| {
        s["route"] == "mylist" && items(s) >= 8
    });
    assert_eq!(items(&s), 8, "8 Favoriten");
    assert_eq!(s["library"]["count_text"], "8 Titel");
    app.wait_state(20, "Bilder der Meine-Liste-Karten", |s| {
        s["library"]["images_set"].as_u64() >= Some(4)
    });
    app.shot(&shots, "mylist");
    app.ok(json!({"cmd": "mylist", "type": "series"}));
    let s = app.wait_state(20, "Meine Liste Serien", |s| items(s) == 1);
    assert_eq!(s["library"]["tools"][2]["active"], true);
    app.ok(json!({"cmd": "mylist", "type": "movies", "sort": "new"}));
    app.wait_state(20, "Meine Liste Filme", |s| items(s) == 7);
    app.shot(&shots, "mylist-filme");
    // Leerzustand: die einzige Serie wird entfernt
    mock.set_favorite_item(&format!("{:032x}", (0x25u128 << 16) + 1), false);
    app.ok(json!({"cmd": "mylist", "type": "series"}));
    let s = app.wait_state(20, "Leere Liste", |s| {
        s["library"]["empty_title"]
            .as_str()
            .is_some_and(|t| t.starts_with("Keine Serien"))
    });
    assert_eq!(items(&s), 0);
    app.shot(&shots, "mylist-leer");

    // ---- Filme als Raster mit Paging
    app.ok(json!({"cmd": "library", "kind": "movies", "view": "grid", "sort": "az"}));
    let s = app.wait_state(20, "Filme-Raster", |s| {
        s["route"] == "library" && items(s) >= 40
    });
    let first = s["library"]["first_titles"][0].as_str().unwrap().to_owned();
    assert_eq!(items(&s), 40, "erste Seite hat 40 Titel");
    let total = s["library"]["total"].as_u64().unwrap();
    assert!(total > 100, "Gesamtzahl {total}");
    app.wait_state(20, "Rasterbilder", |s| {
        s["library"]["images_set"].as_u64() >= Some(6)
    });
    app.shot(&shots, "movies-grid");
    // Endlos-Scroll
    app.ok(json!({"cmd": "lib_scroll", "rows": 5}));
    app.wait_state(20, "zweite Seite", |s| items(s) == 80);
    app.ok(json!({"cmd": "lib_scroll", "rows": 12}));
    app.wait_state(20, "dritte Seite", |s| items(s) > 80);
    // Sortierung Z–A: erster Titel ändert sich
    app.ok(json!({"cmd": "library", "kind": "movies", "view": "grid", "sort": "za"}));
    let s = app.wait_state(20, "Z–A", |s| {
        items(s) >= 40 && s["library"]["first_titles"][0] != json!(first)
    });
    assert_ne!(s["library"]["first_titles"][0], json!(first));
    // Genre-Filter (Drama = zweites Genre der sortierten Liste)
    let drama = format!("{:032x}", 0x7000_0001u32);
    app.ok(
        json!({"cmd": "library", "kind": "movies", "view": "grid", "sort": "az", "genre": drama}),
    );
    let s = app.wait_state(20, "Genre Drama", |s| {
        s["route"] == "library" && items(s) > 0 && s["library"]["total"].as_u64() < Some(total)
    });
    assert_eq!(s["library"]["genres_of_items"], json!(["Drama"]));
    app.shot(&shots, "movies-grid-genre");
    // Auswahlmenü öffnen (Genres, drei Spalten)
    app.wait_state(20, "Genres geladen", |s| {
        s["library"]["genres"].as_u64() >= Some(5)
    });
    app.ok(json!({"cmd": "lib_menu", "which": "genre"}));
    app.wait_state(5, "Menü offen", |s| s["library"]["menu_open"] == true);
    app.shot(&shots, "genre-menu");
    app.nav("Back", 1);
    app.wait_state(5, "Menü zu", |s| {
        s["library"]["menu_open"] == false && s["route"] == "library"
    });

    // ---- Serien als Raster, Sortierung „neu“
    app.ok(json!({"cmd": "library", "kind": "series", "view": "grid", "sort": "new"}));
    app.wait_state(20, "Serien-Raster", |s| items(s) >= 40);
    app.shot(&shots, "series-grid");

    // ---- Filme als Zeilen mit Billboard
    app.ok(json!({"cmd": "library", "kind": "movies", "view": "rows"}));
    let s = app.wait_state(30, "Filme-Zeilen", |s| {
        s["route"] == "library"
            && s["library"]["rows"]
                .as_array()
                .is_some_and(|r| r.len() >= 3)
            && s["library"]["pool"].as_u64() > Some(0)
    });
    println!(
        "Zeilen: {} Hero: {}",
        s["library"]["rows"], s["library"]["hero"]
    );
    std::thread::sleep(Duration::from_millis(1200));
    app.shot(&shots, "movies-rows");
    app.nav("Down", 2);
    app.nav("Right", 2);
    app.shot(&shots, "movies-rows-fokus");

    // ---- Neu und beliebt
    app.ok(json!({"cmd": "route", "route": "newpopular"}));
    let s = app.wait_state(30, "Neu und beliebt", |s| {
        s["route"] == "newpopular" && s["library"]["np_entries"].as_u64() >= Some(10)
    });
    assert_eq!(s["library"]["np_entries"], 20);
    std::thread::sleep(Duration::from_millis(1200));
    app.shot(&shots, "newpopular");
    app.ok(json!({"cmd": "np_tab", "index": 2}));
    std::thread::sleep(Duration::from_millis(800));
    app.shot(&shots, "newpopular-top-serien");

    // ---- Suche
    app.ok(json!({"cmd": "route", "route": "search"}));
    app.wait_state(20, "Suche (Chips)", |s| {
        s["route"] == "search"
            && s["library"]["search"]["chips"]
                .as_array()
                .is_some_and(|c| c.len() >= 3)
    });
    app.shot(&shots, "search-start");
    app.ok(json!({"cmd": "search_text", "text": "stille"}));
    let s = app.wait_state(20, "Suchergebnisse", |s| {
        s["library"]["search"]["sections"]
            .as_array()
            .is_some_and(|c| !c.is_empty())
    });
    println!("Abschnitte: {}", s["library"]["search"]["sections"]);
    std::thread::sleep(Duration::from_millis(1200));
    app.shot(&shots, "search-results");
    app.ok(json!({"cmd": "search_text", "text": "xqzv"}));
    app.wait_state(20, "keine Treffer", |s| {
        s["library"]["empty_title"]
            .as_str()
            .is_some_and(|t| t.starts_with("Keine Treffer"))
    });
    app.shot(&shots, "search-empty");

    // ---- Sammlung (Boxsets) mit Alphabetleiste
    app.ok(json!({"cmd": "collection", "id": VIEW_BOXSETS, "name": "Sammlungen"}));
    let s = app.wait_state(20, "Sammlung", |s| {
        s["route"] == "collection" && items(s) == 8
    });
    assert_eq!(s["library"]["count_text"], "8 Titel");
    app.shot(&shots, "collection");

    // ---- Karte aktivieren führt zur Detailseite
    app.ok(json!({"cmd": "mylist"}));
    app.wait_state(20, "Meine Liste", |s| items(s) > 0);
    app.nav("Activate", 1);
    app.wait_state(10, "Detailroute", |s| s["route"] == "detail");
}
