//! Ende-zu-Ende-Lauf der Konto-, Einstellungs- und Verwaltungsseiten (W4-account) gegen den Mock (kein externes Netz).
//!
//! Braucht ein Fenster (UI-only-Aufnahmen), daher `#[ignore]`. Lokal ausführen:
//!
//! `cargo test -p lfx-app --test account_e2e -- --ignored --test-threads=1`
//!
//! Screenshots landen im Ordner aus `LFX_SHOT_DIR` (sonst in einem Wegwerf-Ordner). Alle Inhalte sind synthetisch.
//! LAN-Suche und echter Browserstart gehören nicht zum Test (Netzwerk bzw. Fremdprogramm).

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
                "Zeitüberschreitung: {what}; Konto: {}",
                s["account"]
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

    fn section(&mut self, name: &str) {
        self.ok(json!({"cmd": "account_section", "name": name}));
    }

    fn row(&mut self, label: &str) {
        self.ok(json!({"cmd": "account_row", "label": label}));
    }

    fn adjust(&mut self, label: &str, d: i32) {
        self.ok(json!({"cmd": "account_row", "label": label, "adjust": d}));
    }

    fn field(&mut self, key: &str, value: &str) {
        self.ok(json!({"cmd": "account_field", "key": key, "value": value}));
    }

    fn wait_row(&mut self, what: &str, label: &str) -> Value {
        let l = label.to_owned();
        self.wait_state(15, what, move |s| find(s, &l).is_some())
    }

    fn shot(&mut self, dir: &Path, name: &str) {
        std::thread::sleep(Duration::from_millis(500));
        let path = dir.join(format!("{name}.png"));
        let r = self.cmd(json!({"cmd": "screenshot", "path": path}));
        assert_eq!(r["ok"], true, "Screenshot {name}: {r}");
    }

    fn wait_toast(&mut self, part: &str) {
        let p = part.to_owned();
        self.wait_state(10, &format!("Hinweis „{part}“"), move |s| {
            s["account"]["toast"]
                .as_str()
                .is_some_and(|t| t.contains(&p))
        });
    }
}

/// Erste Zeile, deren Beschriftung `label` enthält.
fn find(s: &Value, label: &str) -> Option<Value> {
    s["account"]["rows"]
        .as_array()?
        .iter()
        .find(|r| r["label"].as_str().is_some_and(|l| l.contains(label)))
        .cloned()
}

/// Beschriftung der ersten Listenzeile (Art 6).
fn first_item(s: &Value) -> Option<String> {
    s["account"]["rows"]
        .as_array()?
        .iter()
        .find(|r| r["kind"] == 6)
        .and_then(|r| r["label"].as_str().map(str::to_owned))
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

#[test]
#[ignore = "braucht ein Fenster; lokal mit --ignored ausführen"]
fn settings_and_admin_flow() {
    let _lock = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (addr, mock) = rt.block_on(lfx_mock_jellyfin::start());
    mock.set_demo_admin(true);
    let url = format!("http://{addr}");
    let (shots, _keep) = shot_dir();
    let mut app = AppProc::start();
    login(&mut app, &url);

    // ---- Einstellungen: Anzeige
    app.ok(json!({"cmd": "route", "route": "settings"}));
    let s = app.wait_state(10, "Einstellungen", |s| s["route"] == "settings");
    let names = s["account"]["sections"].to_string();
    assert!(names.contains("Verwaltung"), "{names}");
    app.wait_row("Anzeige", "Datensparmodus");
    app.shot(&shots, "account-01-anzeige");
    app.row("Couch-Modus");
    app.wait_state(5, "Couch an", |s| {
        find(s, "Couch").is_some_and(|r| r["on"] == true)
    });
    app.adjust("Design", 1);
    app.wait_state(5, "Design LENZ+", |s| s["design"] == "plus");
    app.shot(&shots, "account-02-anzeige-plus");
    app.adjust("Design", -1);
    app.wait_state(5, "Design LENZFLIX", |s| s["design"] == "flix");

    // ---- Wiedergabe: Server-Konfiguration wird gespeichert
    app.section("Wiedergabe");
    app.wait_row("Wiedergabe", "Bevorzugte Tonsprache");
    app.adjust("Bevorzugte Tonsprache", 1);
    std::thread::sleep(Duration::from_millis(900));
    let s = app.wait_state(10, "Tonsprache vom Server", |s| {
        s["account"]["loading"] == false
            && find(s, "Bevorzugte Tonsprache").is_some_and(|r| r["value"] == "Deutsch")
    });
    assert!(find(&s, "Maximale Bitrate").is_some());
    app.shot(&shots, "account-03-wiedergabe");
    app.section("Untertitel");
    app.wait_row("Untertitel", "Schriftgröße");
    app.adjust("Schriftgröße", 1);
    app.shot(&shots, "account-04-untertitel");
    app.section("Steuerung");
    app.wait_row("Steuerung", "Gamepad");

    // ---- Profil: Passwort und Profilbild
    app.section("Profil");
    app.wait_row("Profil", "Aktuelles Passwort");
    app.shot(&shots, "account-05-profil");
    app.field("pw_cur", DEMO_PASSWORD);
    app.field("pw_new", "neues-passwort");
    app.field("pw_rep", "anderes");
    app.row("Passwort ändern");
    app.wait_toast("stimmen nicht überein");
    app.field("pw_rep", "neues-passwort");
    app.row("Passwort ändern");
    app.wait_toast("Passwort geändert");
    let png = tempfile::Builder::new().suffix(".png").tempfile().unwrap();
    std::fs::write(png.path(), lfx_mock_jellyfin::make_png(8, 8)).unwrap();
    app.field("avatar_path", &png.path().to_string_lossy());
    app.row("Profilbild hochladen");
    app.wait_toast("Profilbild hochgeladen");
    assert!(mock.avatar().is_some(), "Profilbild beim Server angekommen");

    // ---- Quick Connect freigeben
    app.section("Quick Connect");
    app.wait_state(10, "Quick Connect aktiv", |s| {
        find(s, "Quick Connect auf dem Server").is_some_and(|r| r["value"] == "Aktiv")
    });
    let code = rt.block_on(async {
        let c =
            lfx_api::Client::new(&url, lfx_api::ClientInfo::new("Testgerät", "dev-e2e")).unwrap();
        c.quick_connect_initiate().await.unwrap().code
    });
    app.field("qc_code", "000000");
    app.row("Gerät freigeben");
    app.wait_toast("unbekannt");
    app.field("qc_code", &code);
    app.row("Gerät freigeben");
    app.wait_toast("Gerät freigegeben");
    app.shot(&shots, "account-06-quickconnect");

    // ---- Konten und Server
    app.section("Konten");
    app.wait_row("Server", "Server hinzufügen");
    app.shot(&shots, "account-07-server");
    let first_server = first_item(&app.state()).expect("Serverzeile");
    app.row(&first_server);
    app.wait_row("Serverdetail", "Server entfernen");
    app.shot(&shots, "account-08-server-detail");
    app.ok(json!({"cmd": "account_back"}));
    app.wait_row("zurück", "Server hinzufügen");

    // ---- Verwaltung
    app.ok(json!({"cmd": "route", "route": "admin"}));
    app.wait_state(10, "Verwaltung", |s| {
        s["route"] == "admin" && find(s, "Aktive Sitzungen").is_some()
    });
    app.shot(&shots, "account-09-admin-uebersicht");
    app.row("Test-Fernseher");
    app.wait_row("Sitzung", "Nachricht senden");
    app.field("msg_text", "Hallo vom Test");
    app.row("Nachricht senden");
    assert!(
        wait_until(Duration::from_secs(10), || mock
            .admin_events()
            .iter()
            .any(|e| e.starts_with("Nachricht:sess-tv:Hallo vom Test"))),
        "{:?}",
        mock.admin_events()
    );
    app.shot(&shots, "account-10-admin-sitzung");
    app.row("Wiedergabe stoppen");
    assert!(
        wait_until(Duration::from_secs(10), || mock
            .admin_events()
            .iter()
            .any(|e| e.contains("Stop"))),
        "{:?}",
        mock.admin_events()
    );
    app.ok(json!({"cmd": "account_back"}));

    app.section("Benutzer");
    app.wait_row("Benutzer", "Neuen Benutzer anlegen");
    app.shot(&shots, "account-11-admin-benutzer");
    app.row("gast");
    app.wait_row("Rechte", "Videos umwandeln");
    let before = find(&app.state(), "Videos umwandeln").unwrap()["on"].clone();
    app.adjust("Videos umwandeln", 1);
    app.wait_state(10, "Recht geändert", |s| {
        s["account"]["loading"] == false
            && find(s, "Videos umwandeln").is_some_and(|r| r["on"] != before)
    });
    app.shot(&shots, "account-12-admin-rechte");
    app.ok(json!({"cmd": "account_back"}));
    app.row("demo");
    app.wait_row("eigenes Konto", "Benutzer löschen");
    let own = app.state();
    assert_eq!(
        find(&own, "Benutzer löschen").unwrap()["enabled"],
        false,
        "eigenes Konto nicht löschbar"
    );
    app.ok(json!({"cmd": "account_back"}));
    app.row("Neuen Benutzer anlegen");
    app.wait_row("Formular", "Benutzer anlegen");
    app.field("new_user", "neuling");
    app.field("new_pass", "geheim1");
    app.row("Benutzer anlegen");
    app.wait_state(10, "neuer Benutzer in der Liste", |s| {
        find(s, "neuling").is_some()
    });

    app.section("Bibliotheken");
    app.wait_row("Bibliotheken", "Alle Bibliotheken scannen");
    app.shot(&shots, "account-13-admin-bibliotheken");
    app.row("Alle Bibliotheken scannen");
    assert!(wait_until(Duration::from_secs(10), || mock
        .admin_events()
        .contains(&"Bibliothek aktualisiert".to_owned())));
    let lib = first_item(&app.state()).expect("Bibliothek");
    app.row(&lib);
    assert!(
        wait_until(Duration::from_secs(10), || mock
            .admin_events()
            .iter()
            .any(|e| e.starts_with("Bibliothek gescannt:"))),
        "{:?}",
        mock.admin_events()
    );

    app.section("Geplante");
    let s = app.wait_state(10, "Aufgaben", |s| first_item(s).is_some());
    let task = first_item(&s).unwrap();
    app.shot(&shots, "account-14-admin-aufgaben");
    app.row(&task);
    assert!(wait_until(Duration::from_secs(10), || mock
        .admin_events()
        .iter()
        .any(|e| e.starts_with("Aufgabe gestartet:"))));

    app.section("Aktivität");
    app.wait_state(10, "Aktivität", |s| {
        s["account"]["loading"] == false && first_item(s).is_some()
    });
    app.shot(&shots, "account-15-admin-aktivitaet");
    app.section("Protokolle");
    let s = app.wait_state(10, "Protokolldateien", |s| first_item(s).is_some());
    let log = first_item(&s).unwrap();
    app.row(&log);
    app.wait_state(10, "Protokollzeilen", |s| {
        s["account"]["view"]
            .as_str()
            .is_some_and(|v| v.contains("Log"))
            && s["account"]["loading"] == false
    });
    app.shot(&shots, "account-16-admin-protokoll");
    app.ok(json!({"cmd": "account_back"}));

    app.section("Geräte");
    app.wait_state(10, "Geräte", |s| first_item(s).is_some());
    app.shot(&shots, "account-17-admin-geraete");

    app.section("Server");
    app.wait_row("Server", "im Browser");
    app.row("im Browser");
    let s = app.wait_state(5, "Browser-Adresse", |s| {
        !s["account"]["opened_url"].is_null()
    });
    assert!(
        s["account"]["opened_url"]
            .as_str()
            .unwrap()
            .ends_with("/web/#/dashboard")
    );
    app.row("Server neu starten");
    app.wait_state(5, "Rückfrage", |s| !s["account"]["confirm"].is_null());
    app.shot(&shots, "account-18-admin-neustart-frage");
    app.ok(json!({"cmd": "account_confirm", "yes": false}));
    assert!(!mock.admin_events().contains(&"Neustart".to_owned()));
    app.row("Server neu starten");
    app.ok(json!({"cmd": "account_confirm", "yes": true}));
    assert!(wait_until(Duration::from_secs(10), || mock
        .admin_events()
        .contains(&"Neustart".to_owned())));

    // zurück zu den Einstellungen
    app.ok(json!({"cmd": "account_section", "name": "Einstellungen", "activate": true}));
    app.wait_state(10, "zurück", |s| s["route"] == "settings");
    let _ = app.cmd(json!({"cmd": "quit"}));
    rt.block_on(mock.shutdown());
}

#[test]
#[ignore = "braucht ein Fenster; lokal mit --ignored ausführen"]
fn non_admin_has_no_admin_area() {
    let _lock = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (addr, mock) = rt.block_on(lfx_mock_jellyfin::start());
    mock.set_demo_admin(false);
    let url = format!("http://{addr}");
    let mut app = AppProc::start();
    login(&mut app, &url);
    app.ok(json!({"cmd": "route", "route": "settings"}));
    let s = app.wait_state(10, "Einstellungen", |s| s["route"] == "settings");
    let names = s["account"]["sections"].to_string();
    assert!(!names.contains("Verwaltung"), "{names}");
    app.ok(json!({"cmd": "route", "route": "admin"}));
    let s = app.wait_state(10, "Rückfall auf Einstellungen", |s| {
        s["route"] == "settings"
    });
    assert_eq!(s["account"]["admin"], false);
    let _ = app.cmd(json!({"cmd": "quit"}));
    rt.block_on(mock.shutdown());
}
