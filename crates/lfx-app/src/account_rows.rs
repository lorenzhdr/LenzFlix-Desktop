//! W4-account: Zeilenaufbau der Konto-, Einstellungs- und Verwaltungsseiten (aus Zustand, Speicher, Sitzung).

use std::rc::Rc;

use lfx_session::ActiveSession;
use serde_json::Value;

use crate::account::{AccountCtl, Act, Row, View};
use crate::account_logic::{
    self as lg, AdmSec, CfgChoice, CfgToggle, LocalKind, POLICIES, SetSec, fmt_bytes, fmt_date,
    session_lines, task_last, task_state,
};
use crate::app::App;

/// Namen der Designs in der Reihenfolge von `DesignId`.
pub const DESIGN_NAMES: [&str; 4] = ["LENZFLIX", "LENZ+", "LENZPRIME", "LENZ//84"];

fn nid(s: &str) -> String {
    s.chars()
        .filter(|c| *c != '-')
        .collect::<String>()
        .to_lowercase()
}

/// Vom Zustand unabhängige Angaben für den Aufbau.
struct Ctx {
    ds_on: bool,
    design: usize,
    active: Option<ActiveSession>,
}

impl App {
    /// Gespeicherte lokale Einstellung des aktiven Benutzers.
    pub fn local_get(&self, key: &str) -> Option<Value> {
        let a = self.session.active()?;
        self.store
            .get_setting(&a.server_id, &a.user_id, key)
            .ok()
            .flatten()
    }

    fn local_row(&self, key: &'static str) -> Row {
        let def = lg::local_def(key).expect("bekannter Schlüssel");
        let stored = self.local_get(key);
        match def.kind {
            LocalKind::Toggle(_) => Row::toggle(
                def.label,
                lg::local_bool(def, stored.as_ref()),
                Act::Local(key),
            ),
            LocalKind::Choice(..) => Row::choice(
                def.label,
                lg::local_label(def, stored.as_ref()),
                Act::Local(key),
            ),
        }
    }

    /// Alle Zeilen der aktuellen Ansicht.
    pub fn acc_build_rows(self: &Rc<Self>) -> Vec<Row> {
        self.load_home_settings();
        let ctx = {
            let st = self.st.borrow();
            Ctx {
                ds_on: st.home.x.ds.on,
                design: st.design.index() as usize,
                active: self.session.active(),
            }
        };
        let st = self.st.borrow();
        let ctl = &st.account;
        if ctl.admin_mode {
            self.rows_admin(ctl, &ctx)
        } else {
            self.rows_settings(ctl, &ctx)
        }
    }

    fn loading_rows(ctl: &AccountCtl, loaded: bool) -> Option<Vec<Row>> {
        if loaded {
            return None;
        }
        Some(vec![Row::note(if ctl.pending.is_empty() {
            ctl.status.clone()
        } else {
            "Wird geladen …".to_owned()
        })])
    }

    // ---------------------------------------------------------------- Einstellungen

    fn rows_settings(&self, ctl: &AccountCtl, cx: &Ctx) -> Vec<Row> {
        let secs = SetSec::list(cx.active.as_ref().is_some_and(|a| a.is_admin));
        let Some(sec) = secs.get(ctl.section).copied() else {
            return Vec::new();
        };
        match sec {
            SetSec::Display => vec![
                Row::heading("Darstellung"),
                Row::choice("Design", DESIGN_NAMES[cx.design.min(3)], Act::Design),
                Row::toggle("Datensparmodus", cx.ds_on, Act::DataSaver),
                self.local_row(lg::KEY_COUCH),
                self.local_row(lg::KEY_SOUND),
                self.local_row(lg::KEY_REMOTE), // W5-remote
                Row::note(
                    "Gilt für diesen Benutzer auf diesem Gerät. Die Web-Einstellungen bleiben unberührt.",
                ),
                Row::heading("Updates"),
                self.local_row(lg::KEY_UPD_AUTO),
                self.local_row(lg::KEY_UPD_CHANNEL),
                Row::info("Installierte Version", env!("CARGO_PKG_VERSION")),
            ],
            SetSec::Home => {
                if let Some(r) = Self::loading_rows(ctl, ctl.cfg.is_some()) {
                    return r;
                }
                vec![
                    Row::heading("Startseite (auf dem Server gespeichert)"),
                    Row::toggle(
                        CfgToggle::HidePlayedInLatest.label(),
                        ctl.cfg
                            .as_ref()
                            .is_some_and(|c| CfgToggle::HidePlayedInLatest.get(c)),
                        Act::CfgToggle(CfgToggle::HidePlayedInLatest),
                    ),
                    Row::toggle(
                        CfgToggle::DisplayMissingEpisodes.label(),
                        ctl.cfg
                            .as_ref()
                            .is_some_and(|c| CfgToggle::DisplayMissingEpisodes.get(c)),
                        Act::CfgToggle(CfgToggle::DisplayMissingEpisodes),
                    ),
                    Row::note(
                        "Reihenfolge der Bibliotheken und weitere Zeilen stellst du im Browser ein.",
                    ),
                ]
            }
            SetSec::Playback => {
                if let Some(r) = Self::loading_rows(ctl, ctl.cfg.is_some()) {
                    return r;
                }
                let c = ctl.cfg.clone().unwrap_or_default();
                vec![
                    Row::heading("Ton (auf dem Server gespeichert)"),
                    Row::choice(
                        CfgChoice::AudioLang.label(),
                        CfgChoice::AudioLang.show(&c),
                        Act::CfgChoice(CfgChoice::AudioLang),
                    ),
                    Row::toggle(
                        CfgToggle::PlayDefaultAudio.label(),
                        CfgToggle::PlayDefaultAudio.get(&c),
                        Act::CfgToggle(CfgToggle::PlayDefaultAudio),
                    ),
                    Row::toggle(
                        CfgToggle::RememberAudio.label(),
                        CfgToggle::RememberAudio.get(&c),
                        Act::CfgToggle(CfgToggle::RememberAudio),
                    ),
                    Row::heading("Wiedergabe"),
                    Row::toggle(
                        CfgToggle::NextEpisode.label(),
                        CfgToggle::NextEpisode.get(&c),
                        Act::CfgToggle(CfgToggle::NextEpisode),
                    ),
                    Row::heading("Qualität (nur dieses Gerät)"),
                    self.local_row(lg::KEY_BITRATE),
                    Row::note("„Automatisch“ nutzt die Leitung voll aus; eine Grenze spart Daten."),
                ]
            }
            SetSec::Subtitles => {
                if let Some(r) = Self::loading_rows(ctl, ctl.cfg.is_some()) {
                    return r;
                }
                let c = ctl.cfg.clone().unwrap_or_default();
                vec![
                    Row::heading("Untertitel (auf dem Server gespeichert)"),
                    Row::choice(
                        CfgChoice::SubLang.label(),
                        CfgChoice::SubLang.show(&c),
                        Act::CfgChoice(CfgChoice::SubLang),
                    ),
                    Row::choice(
                        CfgChoice::SubMode.label(),
                        CfgChoice::SubMode.show(&c),
                        Act::CfgChoice(CfgChoice::SubMode),
                    ),
                    Row::toggle(
                        CfgToggle::RememberSubs.label(),
                        CfgToggle::RememberSubs.get(&c),
                        Act::CfgToggle(CfgToggle::RememberSubs),
                    ),
                    Row::heading("Aussehen (nur dieses Gerät)"),
                    self.local_row(lg::KEY_SUB_SIZE),
                    self.local_row(lg::KEY_SUB_COLOR),
                    self.local_row(lg::KEY_SUB_BG),
                    {
                        let mut r = Row::note("So sieht ein Untertitel aus.");
                        r.kind = lg::RowKind::Preview;
                        r
                    },
                ]
            }
            SetSec::Controls => vec![
                Row::heading("Bedienung (nur dieses Gerät)"),
                self.local_row(lg::KEY_PAD),
                self.local_row(lg::KEY_SEEK_BACK),
                self.local_row(lg::KEY_SEEK_FWD),
                Row::heading("Tastatur"),
                Row::info("Pfeiltasten", "Navigieren"),
                Row::info("Eingabe oder Leertaste", "Auswählen"),
                Row::info("Esc oder Rücktaste", "Zurück"),
                Row::info("Alt + Links", "Zurück"),
                Row::info("F11", "Vollbild"),
            ],
            SetSec::Downloads => self.rows_downloads(ctl), // W5-downloads
            SetSec::Profile => self.rows_profile(ctl, cx),
            SetSec::QuickConnect => self.rows_qc(ctl),
            SetSec::Servers => self.rows_servers(ctl, cx),
            SetSec::Admin => vec![Row::note(
                "Eingabe öffnet die Verwaltung des Servers (Benutzer, Bibliotheken, Aufgaben, Protokolle).",
            )],
            SetSec::About => vec![Row::note(
                "Eingabe öffnet die Infos zu LENZFLIX Desktop und den verwendeten Komponenten.",
            )],
            SetSec::SignOut => vec![Row::note("Eingabe meldet dich von diesem Server ab.")],
        }
    }

    /// W5-downloads: Einstellungen des Offline-Modus.
    fn rows_downloads(&self, ctl: &AccountCtl) -> Vec<Row> {
        let auto = self.dl_library().is_some_and(|l| l.auto_delete());
        let par = self
            .local_get(crate::downloads::KEY_PARALLEL)
            .and_then(|v| v.as_u64())
            .unwrap_or(2)
            .clamp(1, 4);
        let q = self
            .local_get(crate::downloads::KEY_QUALITY)
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let q_name = ["Original", "Mittel (720p)", "Klein (480p)"][(q as usize).min(2)];
        let used = self
            .dl_library()
            .and_then(|l| l.usage().ok())
            .map_or(0, |u| u.completed_bytes + u.partial_bytes);
        let has_any = !self.dl_records().is_empty();
        let root = ctl.fields.get("dl_root").cloned().unwrap_or_else(|| {
            self.local_get(crate::downloads::KEY_ROOT)
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default()
        });
        let mut set_root = Row::button("Speicherort übernehmen", Act::DlRoot);
        if has_any {
            set_root = set_root.off();
        }
        vec![
            Row::heading("Offline-Downloads"),
            Row::toggle("Gesehene Titel automatisch löschen", auto, Act::DlAuto),
            Row::choice("Gleichzeitige Downloads", par.to_string(), Act::DlParallel),
            Row::choice("Standard-Qualität", q_name, Act::DlQuality),
            Row::info("Belegt", crate::downloads_page::format_bytes(used)),
            Row::button("Downloads öffnen", Act::DlOpenPage),
            Row::heading("Speicherort"),
            Row::field(
                "Ordner",
                "dl_root",
                root,
                "Standard (Datenordner der App)",
                false,
            ),
            set_root,
            Row::note(if has_any {
                "Der Speicherort lässt sich nur ändern, solange nichts heruntergeladen ist. Zum Wechseln zuerst alle Downloads löschen."
            } else {
                "Leer lassen für den Standardordner. Neue Downloads landen im gewählten Ordner."
            }),
        ]
    }

    fn rows_profile(&self, ctl: &AccountCtl, cx: &Ctx) -> Vec<Row> {
        let Some(a) = &cx.active else {
            return Vec::new();
        };
        let field = |key: &'static str| ctl.fields.get(key).cloned().unwrap_or_default();
        let has_img = ctl
            .me
            .as_ref()
            .is_some_and(|m| m.primary_image_tag.is_some());
        let server = self
            .session
            .servers()
            .into_iter()
            .find(|s| s.id == a.server_id)
            .map(|s| s.name)
            .unwrap_or_default();
        let mut rows = vec![
            Row::heading("Konto"),
            Row::info("Name", a.user_name.clone()),
            Row::info("Server", server),
            Row::info(
                "Rolle",
                if a.is_admin {
                    "Administrator"
                } else {
                    "Benutzer"
                },
            ),
            Row::heading("Profilbild"),
            Row::field(
                "Bilddatei",
                "avatar_path",
                field("avatar_path"),
                "Pfad zu einer PNG-, JPEG- oder WebP-Datei",
                false,
            ),
            Row::button("Profilbild hochladen", Act::UploadAvatar),
        ];
        let mut rm = Row::button("Profilbild entfernen", Act::RemoveAvatar);
        if !has_img {
            rm = rm.off();
        }
        rows.push(rm);
        rows.push(Row::heading("Passwort ändern"));
        rows.push(Row::field(
            "Aktuelles Passwort",
            "pw_cur",
            field("pw_cur"),
            "Aktuelles Passwort",
            true,
        ));
        rows.push(Row::field(
            "Neues Passwort",
            "pw_new",
            field("pw_new"),
            "Neues Passwort",
            true,
        ));
        rows.push(Row::field(
            "Wiederholen",
            "pw_rep",
            field("pw_rep"),
            "Neues Passwort wiederholen",
            true,
        ));
        rows.push(Row::button("Passwort ändern", Act::ChangePassword));
        rows
    }

    fn rows_qc(&self, ctl: &AccountCtl) -> Vec<Row> {
        let enabled = ctl.qc_enabled;
        let code = ctl.fields.get("qc_code").cloned().unwrap_or_default();
        let mut go = Row::button("Gerät freigeben", Act::QcAuthorize);
        if enabled == Some(false) {
            go = go.off();
        }
        vec![
            Row::heading("Anderes Gerät anmelden"),
            Row::note(
                "Wähle auf dem anderen Gerät „Quick Connect“ und gib den dort gezeigten Code ein.",
            ),
            Row::info(
                "Quick Connect auf dem Server",
                match enabled {
                    Some(true) => "Aktiv",
                    Some(false) => "Ausgeschaltet",
                    None => "Wird geprüft …",
                },
            ),
            Row::field("Code", "qc_code", code, "Sechsstelliger Code", false),
            go,
        ]
    }

    fn rows_servers(&self, ctl: &AccountCtl, cx: &Ctx) -> Vec<Row> {
        let servers = self.session.servers();
        let active_server = cx.active.as_ref().map(|a| a.server_id.clone());
        if let View::Server(id) = &ctl.view {
            let Some(s) = servers.iter().find(|s| &s.id == id) else {
                return vec![Row::button("‹ Zurück", Act::Back)];
            };
            return vec![
                Row::button("‹ Zurück", Act::Back),
                Row::heading(s.name.clone()),
                Row::info("Adresse", s.url.clone()),
                Row::info("Gespeicherte Benutzer", s.users.len().to_string()),
                Row::info(
                    "Status",
                    if active_server.as_deref() == Some(&s.id) {
                        "Aktuell verbunden"
                    } else {
                        "Gespeichert"
                    },
                ),
                Row::button(
                    "Wer schaut? (auf diesem Server)",
                    Act::ServerWho(s.id.clone()),
                ),
                Row::danger("Server entfernen", Act::ServerRemove(s.id.clone())),
            ];
        }
        let mut rows = vec![Row::heading("Server")];
        for s in &servers {
            rows.push(Row::item(
                s.name.clone(),
                s.url.clone(),
                if active_server.as_deref() == Some(&s.id) {
                    "Verbunden"
                } else {
                    ""
                },
                Act::ServerOpen(s.id.clone()),
            ));
        }
        rows.push(Row::button("Server hinzufügen", Act::ServerAdd));
        rows.push(Row::button(
            if ctl.discovering {
                "Suche läuft …"
            } else {
                "Im Netzwerk suchen"
            },
            Act::ServerDiscover,
        ));
        for d in ctl
            .discovered
            .iter()
            .filter(|d| !servers.iter().any(|s| s.id == d.id))
        {
            rows.push(Row::item(
                d.name.clone(),
                d.address.clone(),
                "Hinzufügen",
                Act::ServerAddFound(d.address.clone()),
            ));
        }
        rows.push(Row::heading("Konto"));
        rows.push(Row::button(
            "Benutzer wechseln (Wer schaut?)",
            Act::UserSwitch,
        ));
        rows.push(Row::danger(
            "Konto von diesem Gerät entfernen",
            Act::RemoveAccount,
        ));
        rows
    }

    // ---------------------------------------------------------------- Verwaltung

    fn rows_admin(&self, ctl: &AccountCtl, cx: &Ctx) -> Vec<Row> {
        let Some(sec) = AdmSec::ALL.get(ctl.section).copied() else {
            return Vec::new();
        };
        let me_id = cx
            .active
            .as_ref()
            .map(|a| nid(&a.user_id))
            .unwrap_or_default();
        match sec {
            AdmSec::Overview => self.rows_overview(ctl),
            AdmSec::Users => Self::rows_users(ctl, &me_id),
            AdmSec::Libraries => {
                if let Some(r) = Self::loading_rows(ctl, ctl.folders.is_some()) {
                    return r;
                }
                let mut rows = vec![
                    Row::button("Alle Bibliotheken scannen", Act::LibScanAll),
                    Row::heading("Bibliotheken"),
                ];
                for f in ctl.folders.iter().flatten() {
                    let prog = match (f.refresh_status.as_deref(), f.refresh_progress) {
                        (Some(s), Some(p)) if s != "Idle" => format!("Scan {p:.0} %"),
                        _ => "Scannen".to_owned(),
                    };
                    let act = match &f.item_id {
                        Some(id) => Act::LibScan(id.clone()),
                        None => Act::None,
                    };
                    rows.push(Row::item(
                        f.name.clone(),
                        format!(
                            "{} · {}",
                            Self::collection_label(f.collection_type.as_deref()),
                            f.locations.join(", ")
                        ),
                        prog,
                        act,
                    ));
                }
                rows
            }
            AdmSec::Tasks => {
                if let Some(r) = Self::loading_rows(ctl, ctl.tasks.is_some()) {
                    return r;
                }
                let mut rows = vec![Row::button("Aktualisieren", Act::Refresh)];
                let mut cat = String::new();
                for t in ctl.tasks.iter().flatten().filter(|t| !t.is_hidden) {
                    let c = t.category.clone().unwrap_or_default();
                    if c != cat {
                        rows.push(Row::heading(c.clone()));
                        cat = c;
                    }
                    let running = t.state == "Running";
                    rows.push(Row::item(
                        t.name.clone(),
                        task_last(t),
                        if running {
                            format!("{} · Stoppen", task_state(t))
                        } else {
                            format!("{} · Starten", task_state(t))
                        },
                        Act::TaskToggle(t.id.clone(), running),
                    ));
                }
                rows
            }
            AdmSec::Activity => {
                if let Some(r) = Self::loading_rows(ctl, ctl.activity.is_some()) {
                    return r;
                }
                let mut rows = vec![Row::button("Aktualisieren", Act::Refresh)];
                for e in ctl.activity.iter().flatten() {
                    rows.push(Row::item(
                        e.name.clone(),
                        e.short_overview
                            .clone()
                            .or_else(|| e.overview.clone())
                            .unwrap_or_default(),
                        e.date.as_deref().map(fmt_date).unwrap_or_default(),
                        Act::None,
                    ));
                }
                rows
            }
            AdmSec::Logs => {
                if let View::Log(name) = &ctl.view {
                    let mut rows = vec![
                        Row::button("‹ Zurück", Act::Back),
                        Row::heading(name.clone()),
                    ];
                    match &ctl.log_lines {
                        None => rows.push(Row::note("Wird geladen …")),
                        Some(l) if l.is_empty() => rows.push(Row::note("Die Datei ist leer.")),
                        Some(l) => {
                            for line in l {
                                rows.push(Row::item(line.clone(), "", "", Act::None));
                            }
                        }
                    }
                    return rows;
                }
                if let Some(r) = Self::loading_rows(ctl, ctl.logs.is_some()) {
                    return r;
                }
                let mut rows = vec![Row::heading("Protokolldateien")];
                for f in ctl.logs.iter().flatten() {
                    rows.push(Row::item(
                        f.name.clone(),
                        f.date_modified.as_deref().map(fmt_date).unwrap_or_default(),
                        fmt_bytes(f.size),
                        Act::LogOpen(f.name.clone()),
                    ));
                }
                rows
            }
            AdmSec::Devices => {
                if let Some(r) = Self::loading_rows(ctl, ctl.devices.is_some()) {
                    return r;
                }
                let mut rows = vec![Row::heading("Registrierte Geräte")];
                for d in ctl.devices.iter().flatten() {
                    let name = d
                        .custom_name
                        .clone()
                        .or_else(|| d.name.clone())
                        .unwrap_or_else(|| "Gerät".to_owned());
                    rows.push(Row::item(
                        name,
                        format!(
                            "{} {} · {}",
                            d.app_name.clone().unwrap_or_default(),
                            d.app_version.clone().unwrap_or_default(),
                            d.last_user_name.clone().unwrap_or_default()
                        ),
                        d.date_last_activity
                            .as_deref()
                            .map(fmt_date)
                            .unwrap_or_default(),
                        Act::DeviceDelete(d.id.clone()),
                    ));
                }
                rows.push(Row::note(
                    "Eingabe meldet das gewählte Gerät vom Server ab.",
                ));
                rows
            }
            AdmSec::Server => vec![
                Row::heading("Weitere Einstellungen"),
                Row::button("Weitere Einstellungen im Browser öffnen", Act::OpenBrowser),
                Row::note(
                    "Öffnet die vollständige Verwaltungsseite des Servers im Standardbrowser.",
                ),
                Row::heading("Server-Steuerung"),
                Row::danger("Server neu starten", Act::Restart),
                Row::danger("Server herunterfahren", Act::Shutdown),
            ],
            AdmSec::Back => vec![Row::note("Eingabe führt zurück zu den Einstellungen.")],
        }
    }

    fn collection_label(t: Option<&str>) -> &'static str {
        match t {
            Some("movies") => "Filme",
            Some("tvshows") => "Serien",
            Some("music") => "Musik",
            Some("books") => "Bücher",
            Some("homevideos") => "Fotos",
            Some("livetv") => "Live-TV",
            _ => "Gemischt",
        }
    }

    fn rows_overview(&self, ctl: &AccountCtl) -> Vec<Row> {
        if let View::Session(id) = &ctl.view {
            let Some(s) = ctl.sessions.iter().flatten().find(|s| &s.id == id) else {
                return vec![
                    Row::button("‹ Zurück", Act::Back),
                    Row::note("Die Sitzung ist beendet."),
                ];
            };
            let (title, sub, playing) = session_lines(s);
            let paused = s.play_state.as_ref().is_some_and(|p| p.is_paused);
            let msg = ctl.fields.get("msg_text").cloned().unwrap_or_default();
            let mut rows = vec![
                Row::button("‹ Zurück", Act::Back),
                Row::heading(title),
                Row::info("Gerät", s.device_name.clone().unwrap_or_default()),
                Row::info(
                    "Programm",
                    format!(
                        "{} {}",
                        s.client.clone().unwrap_or_default(),
                        s.application_version.clone().unwrap_or_default()
                    ),
                ),
                Row::info(
                    "Zuletzt aktiv",
                    s.last_activity_date
                        .as_deref()
                        .map(fmt_date)
                        .unwrap_or_default(),
                ),
                Row::info("Wiedergabe", sub),
                Row::heading("Nachricht senden"),
                Row::field("Text", "msg_text", msg, "Nachricht an das Gerät", false),
                Row::button("Nachricht senden", Act::SessionMsg(s.id.clone())),
            ];
            if playing {
                rows.push(Row::heading("Wiedergabe"));
                let mut p = Row::button(
                    if paused { "Fortsetzen" } else { "Pausieren" },
                    Act::SessionPause(s.id.clone(), paused),
                );
                let mut st = Row::danger("Wiedergabe stoppen", Act::SessionStop(s.id.clone()));
                if !s.supports_remote_control {
                    p = p.off();
                    st = st.off();
                }
                rows.push(p);
                rows.push(st);
            }
            return rows;
        }
        let Some(info) = &ctl.info else {
            return Self::loading_rows(ctl, false).unwrap_or_default();
        };
        let mut rows = vec![
            Row::heading("Server"),
            Row::info("Name", info.server_name.clone()),
            Row::info("Version", format!("{} {}", info.product_name, info.version)),
            Row::info(
                "System",
                format!(
                    "{} {}",
                    info.operating_system_display_name
                        .clone()
                        .unwrap_or_default(),
                    info.system_architecture.clone().unwrap_or_default()
                ),
            ),
            Row::info(
                "Neustart ausstehend",
                if info.has_pending_restart {
                    "Ja"
                } else {
                    "Nein"
                },
            ),
            Row::info(
                "Update verfügbar",
                if info.has_update_available {
                    "Ja"
                } else {
                    "Nein"
                },
            ),
        ];
        let sessions: Vec<_> = ctl.sessions.iter().flatten().collect();
        rows.push(Row::heading(format!(
            "Aktive Sitzungen ({})",
            sessions.len()
        )));
        for s in sessions {
            let (title, sub, _) = session_lines(s);
            rows.push(Row::item(title, sub, "", Act::SessionOpen(s.id.clone())));
        }
        rows.push(Row::button("Aktualisieren", Act::Refresh));
        rows
    }

    fn rows_users(ctl: &AccountCtl, me_id: &str) -> Vec<Row> {
        if ctl.view == View::NewUser {
            return vec![
                Row::button("‹ Zurück", Act::Back),
                Row::heading("Neuer Benutzer"),
                Row::field(
                    "Name",
                    "new_user",
                    ctl.fields.get("new_user").cloned().unwrap_or_default(),
                    "Benutzername",
                    false,
                ),
                Row::field(
                    "Passwort",
                    "new_pass",
                    ctl.fields.get("new_pass").cloned().unwrap_or_default(),
                    "Passwort (darf leer bleiben)",
                    true,
                ),
                Row::button("Benutzer anlegen", Act::UserCreate),
            ];
        }
        if let View::User(id) = &ctl.view {
            let Some(u) = ctl.users.iter().flatten().find(|u| &u.id == id) else {
                return vec![
                    Row::button("‹ Zurück", Act::Back),
                    Row::note("Der Benutzer existiert nicht mehr."),
                ];
            };
            let is_self = nid(&u.id) == me_id;
            let p = u.policy.clone().unwrap_or_default();
            let mut rows = vec![
                Row::button("‹ Zurück", Act::Back),
                Row::heading(format!("Rechte von {}", u.name)),
            ];
            for (i, d) in POLICIES.iter().enumerate() {
                let mut r = Row::toggle(d.label, (d.get)(&p), Act::Policy(u.id.clone(), i));
                if lg::policy_protected(i, is_self) {
                    r = r.off();
                }
                rows.push(r);
            }
            if is_self {
                rows.push(Row::note("Das eigene Konto kann sich hier nicht selbst entmachten, sperren oder löschen."));
            }
            let mut del = Row::danger("Benutzer löschen", Act::UserDelete(u.id.clone()));
            if is_self {
                del = del.off();
            }
            rows.push(del);
            return rows;
        }
        if let Some(r) = Self::loading_rows(ctl, ctl.users.is_some()) {
            return r;
        }
        let mut rows = vec![
            Row::button("Neuen Benutzer anlegen", Act::UserNew),
            Row::heading("Benutzer"),
        ];
        for u in ctl.users.iter().flatten() {
            let p = u.policy.clone().unwrap_or_default();
            let mut tags = Vec::new();
            if p.is_administrator {
                tags.push("Administrator");
            }
            if p.is_disabled {
                tags.push("Deaktiviert");
            }
            if p.is_hidden {
                tags.push("Versteckt");
            }
            let last = u
                .last_activity_date
                .as_deref()
                .map(|d| format!("Zuletzt aktiv {}", fmt_date(d)))
                .unwrap_or_else(|| "Noch nie angemeldet".to_owned());
            rows.push(Row::item(
                u.name.clone(),
                last,
                tags.join(" · "),
                Act::UserOpen(u.id.clone()),
            ));
        }
        rows
    }
}
