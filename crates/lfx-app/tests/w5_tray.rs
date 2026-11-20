//! Ende-zu-Ende-Lauf von Infobereich-Symbol, Einzelinstanz und Update-Oberfläche (W5-tray), ohne externes Netz.
//!
//! Der Update-Server ist ein winziger lokaler HTTP-Server in diesem Test (Manifest plus Testpaket, per Minisign mit
//! einem Wegwerf-Schlüssel signiert); die App bekommt Adresse und Schlüssel über Umgebungsvariablen, die es nur im
//! Feature `dev-automation` gibt. Die Installation startet dabei nie. Alle Inhalte sind synthetisch.
//!
//! Braucht ein Fenster, daher `#[ignore]`: `cargo test -p lfx-app --test w5_tray -- --ignored --test-threads=1`.
//! Screenshots landen in `LFX_SHOT_DIR` (sonst Wegwerf-Ordner), Namen `tray-*.png`.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use lfx_mock_jellyfin::{DEMO_PASSWORD, DEMO_USER};
use serde_json::{Value, json};

static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

// ---------------------------------------------------------------- Update-Server

struct UpdateServer {
    url: String,
    key: String,
    tamper: Arc<AtomicBool>,
    slow: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
}

impl Drop for UpdateServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

const VERSION_NEW: &str = "9.9.9";

fn package_bytes() -> Vec<u8> {
    (0..3_000_000u32).map(|i| (i % 251) as u8).collect()
}

fn start_update_server() -> UpdateServer {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let kp = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
    let data = package_bytes();
    let sig = minisign::sign(
        Some(&kp.pk),
        &kp.sk,
        std::io::Cursor::new(&data),
        Some("Testpaket"),
        Some("synthetisch"),
    )
    .unwrap()
    .into_string();
    let platform = lfx_update::current_platform().expect("unterstützte Plattform");
    let manifest = json!({
        "version": VERSION_NEW,
        "notes": "Neu: Testhinweis eins\nNeu: Testhinweis zwei\nBehoben: Testhinweis drei",
        "pub_date": "2026-12-01T10:00:00Z",
        "assets": { platform: {
            "url": format!("http://127.0.0.1:{port}/LENZFLIX_{VERSION_NEW}_test-setup.exe"),
            "size": data.len(),
            "sha256": lfx_update::verify::sha256_hex(&data),
            "signature": sig,
        }},
    })
    .to_string();
    let tamper = Arc::new(AtomicBool::new(false));
    let slow = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(AtomicBool::new(false));
    let (t, s, st) = (tamper.clone(), slow.clone(), stop.clone());
    std::thread::spawn(move || {
        while !st.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((conn, _)) => {
                    let (manifest, data, t, s) =
                        (manifest.clone(), data.clone(), t.clone(), s.clone());
                    std::thread::spawn(move || serve(conn, &manifest, &data, &t, &s));
                }
                Err(_) => std::thread::sleep(Duration::from_millis(20)),
            }
        }
    });
    UpdateServer {
        url: format!("http://127.0.0.1:{port}/latest.json"),
        key: kp.pk.to_base64(),
        tamper,
        slow,
        stop,
    }
}

fn serve(mut conn: TcpStream, manifest: &str, data: &[u8], tamper: &AtomicBool, slow: &AtomicBool) {
    conn.set_nonblocking(false).ok();
    let mut line = String::new();
    let mut reader = BufReader::new(conn.try_clone().unwrap());
    if reader.read_line(&mut line).is_err() {
        return;
    }
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h).unwrap_or(0) <= 2 {
            break;
        }
    }
    let (body, kind): (Vec<u8>, &str) = if line.contains("latest.json") {
        (manifest.as_bytes().to_vec(), "application/json")
    } else {
        let mut d = data.to_vec();
        if tamper.load(Ordering::SeqCst) {
            d[1000] ^= 0xff;
        }
        (d, "application/octet-stream")
    };
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    if conn.write_all(head.as_bytes()).is_err() {
        return;
    }
    for chunk in body.chunks(65_536) {
        if conn.write_all(chunk).is_err() {
            return;
        }
        if slow.load(Ordering::SeqCst) && line.contains("setup.exe") {
            std::thread::sleep(Duration::from_millis(60));
        }
    }
    let _ = conn.flush();
    let mut sink = [0u8; 16];
    let _ = conn.read(&mut sink);
}

// ---------------------------------------------------------------- App-Prozess

struct AppProc {
    child: Child,
    conn: BufReader<TcpStream>,
    out: TcpStream,
    home: PathBuf,
    _keep: Option<tempfile::TempDir>,
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
    fn start(update: Option<&UpdateServer>, design: &str) -> Self {
        let tmp = tempfile::tempdir().expect("Wegwerf-Ordner");
        let home = tmp.path().to_path_buf();
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_lenzflix"));
        cmd.arg("--home")
            .arg(&home)
            .args([
                "--maximized",
                "--report-interval-ms",
                "500",
                "--design",
                design,
            ])
            .env("LFX_SHOT_UI_ONLY", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Some(u) = update {
            cmd.env("LFX_UPDATE_URL", &u.url)
                .env("LFX_UPDATE_KEY", &u.key);
        }
        let child = cmd.spawn().expect("App starten");
        Self::attach(child, home, Some(tmp))
    }

    fn attach(child: Child, home: PathBuf, keep: Option<tempfile::TempDir>) -> Self {
        let info_path = home.join("data").join("automation.json");
        assert!(
            wait_until(Duration::from_secs(30), || info_path.is_file()),
            "Automation-Datei erscheint nicht"
        );
        std::thread::sleep(Duration::from_millis(200));
        let info: Value = serde_json::from_slice(&std::fs::read(&info_path).unwrap()).unwrap();
        let port = info["port"].as_u64().unwrap() as u16;
        let stream = TcpStream::connect(("127.0.0.1", port)).expect("verbinden");
        stream
            .set_read_timeout(Some(Duration::from_secs(60)))
            .unwrap();
        let out = stream.try_clone().unwrap();
        let mut me = Self {
            child,
            conn: BufReader::new(stream),
            out,
            home,
            _keep: keep,
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

    /// Befehl, nach dem sich die App beendet: die Antwort kann verloren gehen.
    fn cmd_final(&mut self, v: Value) {
        let _ = writeln!(self.out, "{v}");
    }

    fn ok(&mut self, v: Value) -> Value {
        let r = self.cmd(v.clone());
        assert_eq!(r["ok"], true, "{v} -> {r}");
        r
    }

    /// Zustand von Symbol und Update (`tray_state` liefert beide).
    fn tu(&mut self) -> Value {
        self.ok(json!({"cmd": "tray_state"}))
    }

    fn wait_update(&mut self, secs: u64, what: &str, mut f: impl FnMut(&Value) -> bool) -> Value {
        let end = Instant::now() + Duration::from_secs(secs);
        loop {
            let s = self.tu();
            if f(&s["update"]) {
                return s["update"].clone();
            }
            assert!(
                Instant::now() < end,
                "Zeitüberschreitung: {what}; Update: {}",
                s["update"]
            );
            std::thread::sleep(Duration::from_millis(150));
        }
    }

    fn shot(&mut self, dir: &Path, name: &str) {
        std::thread::sleep(Duration::from_millis(500));
        let path = dir.join(format!("{name}.png"));
        self.ok(json!({"cmd": "screenshot", "path": path}));
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

// ---------------------------------------------------------------- Tests

#[test]
#[ignore = "braucht ein Fenster; lokal mit --ignored ausführen"]
fn update_dialog_flow() {
    let _lock = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let server = start_update_server();
    let (shots, _keep) = shot_dir();
    let mut app = AppProc::start(Some(&server), "flix");

    // Startprüfung nach wenigen Sekunden: Dialog mit Versionshinweisen.
    let u = app.wait_update(40, "Update gefunden", |u| {
        u["phase"] == "available" && u["dialog"] == true
    });
    assert_eq!(u["version"], VERSION_NEW);
    assert_eq!(u["buttons"], json!(["install", "later", "skip"]));
    assert!(u["notes"].as_str().unwrap().contains("Testhinweis zwei"));
    app.shot(&shots, "tray-01-update-flix");
    // Alle Designs: der Dialog nutzt nur Theme-Werte.
    for (i, d) in ["plus", "prime", "retro"].iter().enumerate() {
        app.ok(json!({"cmd": "design", "design": d}));
        app.shot(&shots, &format!("tray-02-update-{d}"));
        let _ = i;
    }
    app.ok(json!({"cmd": "design", "design": "flix"}));

    // Später: Dialog zu, Erinnerung gesetzt, Datei geschrieben.
    app.ok(json!({"cmd": "update_press", "button": "later"}));
    let u = app.wait_update(5, "Später", |u| u["dialog"] == false);
    assert!(u["remind_after"].as_u64().unwrap() > 0);
    assert!(app.home.join("config").join("update.json").is_file());

    // Manuelle Prüfung zeigt den Dialog trotz „später“ erneut; Überspringen merkt die Version.
    app.ok(json!({"cmd": "update_check", "manual": true}));
    app.wait_update(15, "erneut gefunden", |u| u["dialog"] == true);
    app.ok(json!({"cmd": "update_press", "button": "skip"}));
    let u = app.wait_update(5, "Überspringen", |u| u["dialog"] == false);
    assert_eq!(u["skipped"], VERSION_NEW);
    // Automatische Prüfung meldet die übersprungene Version nicht mehr.
    app.ok(json!({"cmd": "update_check", "manual": false}));
    let u = app.wait_update(15, "still gefunden", |u| u["phase"] == "available");
    assert_eq!(u["dialog"], false);

    // Manuell erneut, Download mit Fortschritt (langsamer Server), dann geprüft und „installiert“ (nur vermerkt).
    server.slow.store(true, Ordering::SeqCst);
    app.ok(json!({"cmd": "update_check", "manual": true}));
    app.wait_update(15, "Dialog", |u| u["dialog"] == true);
    app.ok(json!({"cmd": "update_press", "button": "install"}));
    let u = app.wait_update(20, "Fortschritt", |u| {
        u["phase"] == "downloading" && u["progress"].as_f64().is_some_and(|p| p > 0.1)
    });
    assert_eq!(u["buttons"], json!(["cancel"]));
    app.shot(&shots, "tray-03-download");
    // Abbrechen geht zurück zum Hinweis.
    app.ok(json!({"cmd": "update_press", "button": "cancel"}));
    app.wait_update(5, "abgebrochen", |u| {
        u["phase"] == "available" && u["dialog"] == true
    });
    server.slow.store(false, Ordering::SeqCst);
    app.ok(json!({"cmd": "update_press", "button": "install"}));
    let u = app.wait_update(30, "installiert", |u| {
        u["phase"] == "installing" && !u["installed"].is_null()
    });
    assert!(u["installed"].as_str().unwrap().contains(VERSION_NEW));
    app.shot(&shots, "tray-04-installiert");

    // Manipuliertes Paket wird verworfen, Fehlerdialog mit „Erneut versuchen“.
    server.tamper.store(true, Ordering::SeqCst);
    app.ok(json!({"cmd": "update_reset"}));
    app.ok(json!({"cmd": "update_check", "manual": true}));
    app.wait_update(15, "Dialog", |u| u["dialog"] == true);
    app.ok(json!({"cmd": "update_press", "button": "install"}));
    let u = app.wait_update(30, "Fehler", |u| u["phase"] == "error");
    assert!(
        u["error"]
            .as_str()
            .unwrap()
            .contains("nicht vertrauenswürdig"),
        "{u}"
    );
    assert_eq!(u["buttons"], json!(["install", "close"]));
    app.shot(&shots, "tray-05-fehler");
    app.ok(json!({"cmd": "update_press", "button": "close"}));
    app.wait_update(5, "geschlossen", |u| u["dialog"] == false);
}

#[test]
#[ignore = "braucht ein Fenster; lokal mit --ignored ausführen"]
fn tray_settings_and_single_instance() {
    let _lock = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (addr, _mock) = rt.block_on(lfx_mock_jellyfin::start());
    let server = start_update_server();
    let (shots, _keep) = shot_dir();
    let mut app = AppProc::start(Some(&server), "flix");

    let r = app.cmd(json!({
        "cmd": "login", "server": format!("http://{addr}"), "user": DEMO_USER, "password": DEMO_PASSWORD
    }));
    assert_eq!(r["ok"], true, "{r}");
    assert!(wait_until(Duration::from_secs(30), || {
        let s = app.cmd(json!({"cmd": "state"}));
        s["state"]["route"] == "home"
    }));
    // Startprüfung abwarten und den Dialog wegklicken, damit er die Einstellungen nicht verdeckt.
    app.wait_update(40, "Update gefunden", |u| u["dialog"] == true);
    app.ok(json!({"cmd": "update_press", "button": "later"}));

    app.ok(json!({"cmd": "route", "route": "settings"}));
    assert!(wait_until(Duration::from_secs(10), || {
        let s = app.cmd(json!({"cmd": "account_state"}));
        s["account"]["rows"].to_string().contains("Update-Status")
    }));
    app.shot(&shots, "tray-06-einstellungen-flix");
    for d in ["plus", "prime", "retro"] {
        app.ok(json!({"cmd": "design", "design": d}));
        app.ok(json!({"cmd": "route", "route": "settings"}));
        app.shot(&shots, &format!("tray-07-einstellungen-{d}"));
    }
    app.ok(json!({"cmd": "design", "design": "flix"}));
    app.ok(json!({"cmd": "route", "route": "settings"}));

    // Standard aus: Schließen würde beenden, das Symbol fehlt.
    let t = app.tu();
    assert_eq!(t["tray"]["close_to_tray"], false);
    assert_eq!(t["tray"]["icon"], false);

    // Einstellung an (über die Einstellungszeile): Symbol da, Datei geschrieben.
    app.ok(json!({"cmd": "account_row", "label": "im Infobereich weiterlaufen"}));
    let t = app.tu();
    assert_eq!(t["tray"]["close_to_tray"], true);
    assert_eq!(t["tray"]["icon"], true, "{t}");
    assert!(app.home.join("config").join("window.json").is_file());

    // Schließen versteckt nur das Fenster.
    assert_eq!(t["tray"]["window_visible"], true);
    app.ok(json!({"cmd": "tray_close_request"}));
    assert_eq!(app.tu()["tray"]["window_visible"], false);

    // Zweiter Start mit demselben Ordner: weckt das Fenster und beendet sich selbst.
    let second = Command::new(env!("CARGO_BIN_EXE_lenzflix"))
        .arg("--home")
        .arg(&app.home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("zweiter Start");
    let mut second = second;
    let status = {
        let end = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(s) = second.try_wait().unwrap() {
                break s;
            }
            assert!(Instant::now() < end, "zweiter Start beendet sich nicht");
            std::thread::sleep(Duration::from_millis(100));
        }
    };
    assert!(status.success(), "zweiter Start: {status}");
    assert!(
        wait_until(
            Duration::from_secs(10),
            || app.tu()["tray"]["window_visible"] == true
        ),
        "Fenster wird nicht gezeigt"
    );

    // Menüeinträge: Öffnen und Beenden-Pfad (Pause ohne Wiedergabe bleibt wirkungslos).
    app.ok(json!({"cmd": "tray_close_request"}));
    assert_eq!(app.tu()["tray"]["window_visible"], false);
    app.ok(json!({"cmd": "tray_action", "name": "open"}));
    let t = app.tu();
    assert_eq!(t["tray"]["window_visible"], true);
    assert_eq!(t["tray"]["last_action"], "lfx-tray-open");
    app.ok(json!({"cmd": "tray_action", "name": "pause"}));
    assert_eq!(app.tu()["tray"]["pause_label"], "Wiedergabe pausieren");
    app.shot(&shots, "tray-08-einstellungen-an");

    // Wieder aus: Symbol weg, Schließen beendet die App.
    app.ok(json!({"cmd": "tray_pref", "on": false}));
    let t = app.tu();
    assert_eq!(t["tray"]["icon"], false);
    assert_eq!(t["tray"]["close_to_tray"], false);
    app.cmd_final(json!({"cmd": "tray_close_request"}));
    assert!(
        wait_until(Duration::from_secs(15), || app
            .child
            .try_wait()
            .ok()
            .flatten()
            .is_some()),
        "App beendet sich nicht"
    );
}
