//! W4-account: Aktionen und Ladevorgänge der Konto-, Einstellungs- und Verwaltungsseiten.
//!
//! Netzwerk läuft auf der Tokio-Laufzeit; Ergebnisse kommen über [`post`] in den UI-Thread zurück. Jede Antwort
//! prüft den Seitenzähler `epoch`, damit veraltete Antworten (Bereichswechsel) verworfen werden.

use std::future::Future;
use std::rc::Rc;
use std::time::Duration;

use lfx_api::{ApiError, PasswordChange, PlaystateCommand};
use lfx_ui::ToastKind;

use crate::account::{AccountCtl, Act, View};
use crate::account_logic::{self as lg, AdmSec, LocalKind, POLICIES, SetSec};
use crate::app::{App, post};
use crate::cli::DesignId;
use crate::route::Route;

/// Fehlertext für die Oberfläche.
pub fn err_text(e: &ApiError) -> String {
    match e {
        ApiError::Forbidden => "Dafür fehlen die Rechte.".to_owned(),
        ApiError::Unauthorized => "Die Anmeldung ist abgelaufen.".to_owned(),
        ApiError::NotFound => "Nicht gefunden.".to_owned(),
        ApiError::Network(_) | ApiError::Timeout => "Der Server ist nicht erreichbar.".to_owned(),
        other => other.to_string(),
    }
}

fn qc_err(e: &ApiError) -> String {
    match e {
        ApiError::NotFound => "Der Code ist unbekannt oder abgelaufen.".to_owned(),
        ApiError::QuickConnectDisabled => {
            "Quick Connect ist auf dem Server ausgeschaltet.".to_owned()
        }
        other => err_text(other),
    }
}

/// Eigenschaften einer ausgelösten Serveraktion.
#[derive(Clone, Copy)]
struct Op {
    ok: &'static str,
    reload: bool,
    after: Option<fn(&mut AccountCtl)>,
    err: fn(&ApiError) -> String,
}

impl Default for Op {
    fn default() -> Self {
        Self {
            ok: "",
            reload: false,
            after: None,
            err: err_text,
        }
    }
}

impl App {
    // ---------------------------------------------------------------- Ändern (Schalter, Auswahl)

    /// Schalter oder Auswahl der Zeile `i` um `dir` ändern.
    pub fn acc_adjust(self: &Rc<Self>, i: usize, dir: i32) {
        let Some(act) = self.st.borrow().account.rows.get(i).map(|r| r.act.clone()) else {
            return;
        };
        {
            let mut st = self.st.borrow_mut();
            st.account.zone = 1;
            st.account.focus = i;
        }
        match act {
            Act::Design => {
                let cur = self.st.borrow().design.index();
                let n = lg::cycle(cur as usize, 4, dir) as i32;
                self.choose_design(DesignId::from_index(n));
            }
            Act::DataSaver => self.ds_toggle_main(),
            Act::Local(key) => {
                if let Some(def) = lg::local_def(key) {
                    let stored = self.local_get(key);
                    let v = lg::local_step(def, stored.as_ref(), dir);
                    self.save_setting(key, v);
                    if key == lg::KEY_REMOTE {
                        self.remote_settings_changed(); // W5-remote
                    }
                    // Zugehörige Anzeige (Vorschau) mitführen
                    let _ = matches!(def.kind, LocalKind::Toggle(_));
                }
            }
            Act::CfgToggle(t) => {
                let new = {
                    let mut st = self.st.borrow_mut();
                    let Some(cfg) = st.account.cfg.as_mut() else {
                        return;
                    };
                    let on = !t.get(cfg);
                    t.set(cfg, on);
                    cfg.clone()
                };
                self.acc_save_cfg(new);
            }
            Act::CfgChoice(c) => {
                let new = {
                    let mut st = self.st.borrow_mut();
                    let Some(cfg) = st.account.cfg.as_mut() else {
                        return;
                    };
                    c.step(cfg, dir);
                    cfg.clone()
                };
                self.acc_save_cfg(new);
            }
            Act::Policy(uid, idx) => self.acc_toggle_policy(&uid, idx),
            _ => {}
        }
        self.acc_rebuild();
    }

    fn acc_save_cfg(self: &Rc<Self>, cfg: lfx_api::UserConfiguration) {
        let Some(a) = self.session.active() else {
            return;
        };
        self.acc_run(
            async move { a.client.update_user_configuration(&cfg).await },
            Op {
                reload: true,
                ..Op::default()
            },
        );
    }

    fn acc_toggle_policy(self: &Rc<Self>, uid: &str, idx: usize) {
        let Some(def) = POLICIES.get(idx) else { return };
        let is_self = self.session.active().is_some_and(|a| {
            a.user_id
                .replace('-', "")
                .eq_ignore_ascii_case(&uid.replace('-', ""))
        });
        if lg::policy_protected(idx, is_self) {
            return;
        }
        let policy = {
            let mut st = self.st.borrow_mut();
            let Some(u) = st
                .account
                .users
                .as_mut()
                .and_then(|v| v.iter_mut().find(|u| u.id == uid))
            else {
                return;
            };
            let p = u.policy.get_or_insert_with(Default::default);
            let on = !(def.get)(p);
            (def.set)(p, on);
            p.clone()
        };
        let Some(a) = self.session.active() else {
            return;
        };
        let uid = uid.to_owned();
        self.acc_run(
            async move { a.client.update_user_policy(&uid, &policy).await },
            Op {
                reload: true,
                ..Op::default()
            },
        );
    }

    // ---------------------------------------------------------------- Aktionen

    /// Führt eine Aktion aus, fragt bei Bedarf vorher nach.
    pub fn acc_exec(self: &Rc<Self>, act: Act) {
        if act.confirm().is_some() {
            self.acc_ask(act);
        } else {
            self.acc_do(act);
        }
    }

    /// Führt eine Aktion ohne Rückfrage aus.
    pub fn acc_do(self: &Rc<Self>, act: Act) {
        let active = self.session.active();
        let client = active.as_ref().map(|a| a.client.clone());
        let field = |k: &str| {
            self.st
                .borrow()
                .account
                .fields
                .get(k)
                .cloned()
                .unwrap_or_default()
        };
        match act {
            Act::None | Act::Design | Act::DataSaver | Act::Local(_) => {}
            Act::CfgToggle(_) | Act::CfgChoice(_) | Act::Policy(..) | Act::Field(_) => {}
            Act::Back => self.acc_pop_view(),
            Act::Refresh => self.acc_reload(),
            Act::ChangePassword => {
                let (cur, new, rep) = (field("pw_cur"), field("pw_new"), field("pw_rep"));
                if let Err(m) = lg::password_check(&cur, &new, &rep) {
                    return self.toast(m, ToastKind::Error);
                }
                let Some(client) = client else { return };
                self.acc_run(
                    async move {
                        client
                            .change_password(&PasswordChange {
                                current_pw: cur,
                                new_pw: new,
                            })
                            .await
                    },
                    Op {
                        ok: "Passwort geändert.",
                        after: Some(|c| {
                            c.fields.remove("pw_cur");
                            c.fields.remove("pw_new");
                            c.fields.remove("pw_rep");
                        }),
                        ..Op::default()
                    },
                );
            }
            Act::UploadAvatar => {
                let path = field("avatar_path");
                let path = path.trim().trim_matches('"').to_owned();
                if path.is_empty() {
                    return self
                        .toast("Bitte den Pfad einer Bilddatei eingeben.", ToastKind::Error);
                }
                let Some(client) = client else { return };
                self.acc_run(
                    async move {
                        let bytes = tokio::task::spawn_blocking(move || {
                            let meta = std::fs::metadata(&path)?;
                            if meta.len() > 8 * 1024 * 1024 {
                                return Err(std::io::Error::other(
                                    "Die Datei ist größer als 8 MB.",
                                ));
                            }
                            std::fs::read(&path)
                        })
                        .await
                        .map_err(|e| ApiError::Io(std::io::Error::other(e.to_string())))?
                        .map_err(ApiError::Io)?;
                        client.upload_profile_image(&bytes).await
                    },
                    Op {
                        ok: "Profilbild hochgeladen.",
                        reload: true,
                        ..Op::default()
                    },
                );
            }
            Act::RemoveAvatar => {
                let Some(client) = client else { return };
                self.acc_run(
                    async move { client.delete_profile_image().await },
                    Op {
                        ok: "Profilbild entfernt.",
                        reload: true,
                        ..Op::default()
                    },
                );
            }
            Act::QcAuthorize => {
                let code = lg::clean_code(&field("qc_code"));
                if code.is_empty() {
                    return self.toast(
                        "Bitte den Code des anderen Geräts eingeben.",
                        ToastKind::Error,
                    );
                }
                let Some(client) = client else { return };
                self.acc_run(
                    async move {
                        match client.quick_connect_authorize(&code).await {
                            Ok(true) => Ok(()),
                            Ok(false) => Err(ApiError::NotFound),
                            Err(e) => Err(e),
                        }
                    },
                    Op {
                        ok: "Gerät freigegeben.",
                        after: Some(|c| {
                            c.fields.remove("qc_code");
                        }),
                        err: qc_err,
                        ..Op::default()
                    },
                );
            }
            Act::ServerOpen(id) => self.acc_open_view(View::Server(id)),
            Act::ServerAdd => self.go(Route::AddServer),
            Act::ServerDiscover => self.acc_discover(),
            Act::ServerAddFound(addr) => {
                let session = self.session.clone();
                self.acc_busy(true);
                self.rt.spawn(async move {
                    let r = session.add_server(&addr).await;
                    post(move |app| {
                        app.acc_busy(false);
                        match r {
                            Ok(_) => app.toast("Server hinzugefügt.", ToastKind::Success),
                            Err(e) => app.toast(&e.to_string(), ToastKind::Error),
                        }
                        app.acc_rebuild();
                    });
                });
            }
            Act::ServerWho(id) => {
                self.reset_home();
                self.show_profiles(id);
            }
            Act::ServerRemove(id) => {
                let was_active = active.as_ref().is_some_and(|a| a.server_id == id);
                match self.session.remove_server(&id) {
                    Ok(()) => self.toast("Server entfernt.", ToastKind::Success),
                    Err(e) => self.toast(&e.to_string(), ToastKind::Error),
                }
                if was_active || self.session.active().is_none() {
                    self.reset_home();
                    match self.session.servers().first().map(|s| s.id.clone()) {
                        Some(next) => self.show_profiles(next),
                        None => self.go_reset(Route::AddServer),
                    }
                } else {
                    self.acc_pop_view();
                }
            }
            Act::UserSwitch => self.profile_item(0),
            Act::RemoveAccount => {
                if let Some(a) = active {
                    match self.session.remove_user(&a.server_id, &a.user_id) {
                        Ok(()) => self.toast("Konto entfernt.", ToastKind::Success),
                        Err(e) => self.toast(&e.to_string(), ToastKind::Error),
                    }
                    self.reset_home();
                    self.show_profiles(a.server_id);
                }
            }
            Act::SignOut => self.sign_out(),
            Act::SessionOpen(id) => self.acc_open_view(View::Session(id)),
            Act::SessionMsg(id) => {
                let text = field("msg_text");
                if text.trim().is_empty() {
                    return self.toast("Bitte eine Nachricht eingeben.", ToastKind::Error);
                }
                let Some(client) = client else { return };
                self.acc_run(
                    async move { client.session_message(&id, "LENZFLIX", &text, 8000).await },
                    Op {
                        ok: "Nachricht gesendet.",
                        after: Some(|c| {
                            c.fields.remove("msg_text");
                        }),
                        ..Op::default()
                    },
                );
            }
            Act::SessionPause(id, paused) => {
                let Some(client) = client else { return };
                let cmd = if paused {
                    PlaystateCommand::Unpause
                } else {
                    PlaystateCommand::Pause
                };
                self.acc_run(
                    async move { client.session_playstate(&id, cmd, None).await },
                    Op {
                        reload: true,
                        ..Op::default()
                    },
                );
            }
            Act::SessionStop(id) => {
                let Some(client) = client else { return };
                self.acc_run(
                    async move {
                        client
                            .session_playstate(&id, PlaystateCommand::Stop, None)
                            .await
                    },
                    Op {
                        ok: "Wiedergabe gestoppt.",
                        reload: true,
                        ..Op::default()
                    },
                );
            }
            Act::UserOpen(id) => self.acc_open_view(View::User(id)),
            Act::UserNew => self.acc_open_view(View::NewUser),
            Act::UserCreate => {
                let name = field("new_user");
                let pass = field("new_pass");
                if name.trim().is_empty() {
                    return self.toast("Bitte einen Benutzernamen eingeben.", ToastKind::Error);
                }
                let Some(client) = client else { return };
                self.acc_run(
                    async move { client.create_user(name.trim(), &pass).await.map(|_| ()) },
                    Op {
                        ok: "Benutzer angelegt.",
                        reload: true,
                        after: Some(|c| {
                            c.fields.remove("new_user");
                            c.fields.remove("new_pass");
                            c.view = View::Root;
                        }),
                        ..Op::default()
                    },
                );
            }
            Act::UserDelete(id) => {
                let Some(client) = client else { return };
                self.acc_run(
                    async move { client.delete_user(&id).await },
                    Op {
                        ok: "Benutzer gelöscht.",
                        reload: true,
                        after: Some(|c| c.view = View::Root),
                        ..Op::default()
                    },
                );
            }
            Act::LibScanAll => {
                let Some(client) = client else { return };
                self.acc_run(
                    async move { client.refresh_library().await },
                    Op {
                        ok: "Scan aller Bibliotheken gestartet.",
                        reload: true,
                        ..Op::default()
                    },
                );
            }
            Act::LibScan(id) => {
                let Some(client) = client else { return };
                self.acc_run(
                    async move { client.refresh_item(&id).await },
                    Op {
                        ok: "Scan der Bibliothek gestartet.",
                        reload: true,
                        ..Op::default()
                    },
                );
            }
            Act::TaskToggle(id, running) => {
                let Some(client) = client else { return };
                self.acc_run(
                    async move {
                        if running {
                            client.stop_task(&id).await
                        } else {
                            client.start_task(&id).await
                        }
                    },
                    Op {
                        ok: if running {
                            "Aufgabe wird gestoppt."
                        } else {
                            "Aufgabe gestartet."
                        },
                        reload: true,
                        ..Op::default()
                    },
                );
            }
            Act::LogOpen(name) => self.acc_open_view(View::Log(name)),
            Act::DeviceDelete(id) => {
                let Some(client) = client else { return };
                self.acc_run(
                    async move { client.delete_device(&id).await },
                    Op {
                        ok: "Gerät abgemeldet.",
                        reload: true,
                        ..Op::default()
                    },
                );
            }
            Act::Restart => {
                let Some(client) = client else { return };
                self.acc_run(
                    async move { client.restart_server().await },
                    Op {
                        ok: "Neustart angefordert.",
                        ..Op::default()
                    },
                );
            }
            Act::Shutdown => {
                let Some(client) = client else { return };
                self.acc_run(
                    async move { client.shutdown_server().await },
                    Op {
                        ok: "Herunterfahren angefordert.",
                        ..Op::default()
                    },
                );
            }
            Act::OpenBrowser => {
                let Some(a) = active else { return };
                let Some(base) = self
                    .session
                    .servers()
                    .into_iter()
                    .find(|s| s.id == a.server_id)
                    .map(|s| s.url)
                else {
                    return;
                };
                let url = lg::dashboard_url(&base);
                let ok = lg::open_in_browser(&url);
                self.st.borrow_mut().account.opened_url = ok.then_some(url);
                if ok {
                    self.toast("Verwaltung wird im Browser geöffnet.", ToastKind::Info);
                } else {
                    self.toast("Der Browser ließ sich nicht öffnen.", ToastKind::Error);
                }
            }
        }
    }

    fn acc_busy(self: &Rc<Self>, on: bool) {
        self.st.borrow_mut().account.discovering = on;
        self.acc_rebuild();
    }

    fn acc_discover(self: &Rc<Self>) {
        if self.st.borrow().account.discovering {
            return;
        }
        let epoch = self.st.borrow().account.epoch;
        self.acc_busy(true);
        let session = self.session.clone();
        self.rt.spawn(async move {
            let r = session.discover_servers(Duration::from_secs(3)).await;
            post(move |app| {
                if app.st.borrow().account.epoch != epoch {
                    return;
                }
                app.st.borrow_mut().account.discovering = false;
                match r {
                    Ok(list) => {
                        let none = list.is_empty();
                        app.st.borrow_mut().account.discovered = list;
                        if none {
                            app.toast("Kein Server im Netzwerk gefunden.", ToastKind::Info);
                        }
                    }
                    Err(e) => app.toast(&e.to_string(), ToastKind::Error),
                }
                app.acc_rebuild();
            });
        });
    }

    /// Startet eine Serveraktion; Ergebnis als Hinweis, danach ggf. neu laden.
    fn acc_run<Fut>(self: &Rc<Self>, fut: Fut, op: Op)
    where
        Fut: Future<Output = Result<(), ApiError>> + Send + 'static,
    {
        let epoch = self.st.borrow().account.epoch;
        self.rt.spawn(async move {
            let r = fut.await;
            post(move |app| {
                match &r {
                    Ok(()) => {
                        if !op.ok.is_empty() {
                            app.toast(op.ok, ToastKind::Success);
                        }
                    }
                    Err(e) => app.toast(&(op.err)(e), ToastKind::Error),
                }
                if app.st.borrow().account.epoch != epoch || !app.on_account_page() {
                    return;
                }
                if r.is_ok()
                    && let Some(f) = op.after
                {
                    f(&mut app.st.borrow_mut().account);
                }
                if op.reload || r.is_err() {
                    app.acc_reload();
                } else {
                    app.acc_rebuild();
                }
            });
        });
    }

    // ---------------------------------------------------------------- Laden

    /// Verwirft die Daten des Bereichs und lädt sie neu.
    pub fn acc_reload(self: &Rc<Self>) {
        {
            let mut st = self.st.borrow_mut();
            let a = &mut st.account;
            a.epoch += 1;
            a.pending.clear();
            a.invalidate_section();
            if !a.admin_mode {
                a.me = None;
                a.cfg = None;
            }
        }
        self.acc_ensure_data();
        self.acc_rebuild();
    }

    /// Lädt, was der aktuelle Bereich braucht und noch fehlt.
    pub fn acc_ensure_data(self: &Rc<Self>) {
        if self.session.active().is_none() {
            return;
        }
        if let Some(sec) = self.acc_set_sec() {
            match sec {
                SetSec::Home | SetSec::Playback | SetSec::Subtitles | SetSec::Profile => {
                    self.acc_load_me();
                }
                SetSec::QuickConnect => self.acc_load_qc(),
                _ => {}
            }
        } else if let Some(sec) = self.acc_adm_sec() {
            let view = self.st.borrow().account.view.clone();
            match sec {
                AdmSec::Overview => {
                    self.acc_load(
                        "info",
                        |c, v| c.info = Some(v),
                        |cl| async move { cl.system_info().await },
                    );
                    self.acc_load(
                        "sessions",
                        |c, v| c.sessions = Some(v),
                        |cl| async move { cl.sessions(Some(960)).await },
                    );
                }
                AdmSec::Users => {
                    self.acc_load(
                        "users",
                        |c, v| c.users = Some(v),
                        |cl| async move { cl.users().await },
                    );
                }
                AdmSec::Libraries => {
                    self.acc_load(
                        "folders",
                        |c, v| c.folders = Some(v),
                        |cl| async move { cl.virtual_folders().await },
                    );
                }
                AdmSec::Tasks => {
                    self.acc_load(
                        "tasks",
                        |c, v| c.tasks = Some(v),
                        |cl| async move { cl.scheduled_tasks(false).await },
                    );
                }
                AdmSec::Activity => {
                    self.acc_load(
                        "activity",
                        |c, v: lfx_api::QueryResult<lfx_api::ActivityLogEntry>| {
                            c.activity = Some(v.items)
                        },
                        |cl| async move { cl.activity_log(50, 0, None).await },
                    );
                }
                AdmSec::Logs => {
                    if let View::Log(name) = view {
                        let n = name.clone();
                        self.acc_load(
                            "log",
                            |c, text: String| {
                                let lines: Vec<String> = text.lines().map(str::to_owned).collect();
                                let skip = lines.len().saturating_sub(300);
                                c.log_lines = Some(lines.into_iter().skip(skip).collect());
                            },
                            move |cl| async move { cl.log_file(&n).await },
                        );
                    } else {
                        self.acc_load(
                            "logs",
                            |c, v| c.logs = Some(v),
                            |cl| async move { cl.logs().await },
                        );
                    }
                }
                AdmSec::Devices => {
                    self.acc_load(
                        "devices",
                        |c, v: lfx_api::QueryResult<lfx_api::DeviceInfo>| c.devices = Some(v.items),
                        |cl| async move { cl.devices().await },
                    );
                }
                AdmSec::Server | AdmSec::Back => {}
            }
        }
    }

    fn acc_load_me(self: &Rc<Self>) {
        self.acc_load(
            "me",
            |c, u: lfx_api::UserDto| {
                c.cfg = Some(u.configuration.clone().unwrap_or_default());
                c.me = Some(u);
            },
            |cl| async move { cl.me().await },
        );
    }

    fn acc_load_qc(self: &Rc<Self>) {
        self.acc_load(
            "qc",
            |c, v: bool| c.qc_enabled = Some(v),
            |cl| async move { cl.quick_connect_enabled().await },
        );
    }

    /// Lädt einmal je Schlüssel (nur wenn noch nicht vorhanden oder schon unterwegs).
    fn acc_load<T, F, Fut>(
        self: &Rc<Self>,
        key: &'static str,
        apply: fn(&mut AccountCtl, T),
        make: F,
    ) where
        T: Send + 'static,
        F: FnOnce(lfx_api::Client) -> Fut,
        Fut: Future<Output = Result<T, ApiError>> + Send + 'static,
    {
        let Some(a) = self.session.active() else {
            return;
        };
        let epoch = {
            let mut st = self.st.borrow_mut();
            let c = &mut st.account;
            let present = match key {
                "me" => c.me.is_some(),
                "qc" => c.qc_enabled.is_some(),
                "info" => c.info.is_some(),
                "sessions" => c.sessions.is_some(),
                "users" => c.users.is_some(),
                "folders" => c.folders.is_some(),
                "tasks" => c.tasks.is_some(),
                "activity" => c.activity.is_some(),
                "logs" => c.logs.is_some(),
                "log" => c.log_lines.is_some(),
                "devices" => c.devices.is_some(),
                _ => false,
            };
            if present || !c.pending.insert(key) {
                return;
            }
            c.status.clear();
            c.status_error = false;
            c.epoch
        };
        let fut = make(a.client);
        self.rt.spawn(async move {
            let r = fut.await;
            post(move |app| {
                {
                    let mut st = app.st.borrow_mut();
                    if st.account.epoch != epoch {
                        return;
                    }
                    st.account.pending.remove(key);
                    match r {
                        Ok(v) => apply(&mut st.account, v),
                        Err(e) => {
                            st.account.status = err_text(&e);
                            st.account.status_error = true;
                        }
                    }
                }
                if app.on_account_page() {
                    app.acc_rebuild();
                }
            });
        });
        self.acc_push();
    }

    /// Alle 5 s: aktive Sitzungen der Übersicht auffrischen.
    pub fn acc_tick(self: &Rc<Self>) {
        let ok = {
            let st = self.st.borrow();
            let a = &st.account;
            self.on_account_page()
                && a.admin_mode
                && AdmSec::ALL.get(a.section) == Some(&AdmSec::Overview)
                && a.editing.is_none()
                && a.confirm.is_none()
                && a.pending.is_empty()
        };
        if !ok {
            return;
        }
        // Sitzungen leise nachladen (ohne die Anzeige zu leeren)
        let Some(a) = self.session.active() else {
            return;
        };
        let epoch = self.st.borrow().account.epoch;
        self.st.borrow_mut().account.pending.insert("tick");
        self.rt.spawn(async move {
            let r = a.client.sessions(Some(960)).await;
            post(move |app| {
                {
                    let mut st = app.st.borrow_mut();
                    if st.account.epoch != epoch {
                        return;
                    }
                    st.account.pending.remove("tick");
                    if let Ok(v) = r {
                        st.account.sessions = Some(v);
                    }
                }
                if app.on_account_page() {
                    app.acc_rebuild();
                }
            });
        });
    }
}
