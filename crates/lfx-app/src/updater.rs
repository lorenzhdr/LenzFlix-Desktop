//! W5-tray: Update-Oberfläche auf `lfx-update`.
//!
//! Prüfung beim Start (Intervall aus `UpdateSettings`), Hinweisdialog mit Versionshinweisen, Download mit Fortschritt,
//! „Jetzt installieren“ / „Später“ / „Überspringen“ und der Abschnitt „Updates“ in den Einstellungen.
//! Die Einstellungen gelten für das Gerät (Datei `update.json` im Konfigurationsordner), weil die Prüfung läuft, bevor
//! jemand angemeldet ist. Der Dialog ist ein Overlay (`UpdateOverlay`), Tastatur und Gamepad laufen über
//! [`App::update_action`].
//!
//! Entwicklung: Nur mit dem Feature `dev-automation` lassen sich Manifest-Adresse (`LFX_UPDATE_URL`) und öffentlicher
//! Schlüssel (`LFX_UPDATE_KEY`) per Umgebung überschreiben; die Installation startet dann nie, der Pfad des geprüften
//! Pakets wird nur vermerkt. Läufe mit `--home` prüfen sonst nicht automatisch (kein Netz aus Tests).

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use lfx_nav::NavAction;
use lfx_ui::{ToastKind, UpdateUi};
use lfx_update::{Channel, UpdateConfig, UpdateInfo, UpdateSettings, Updater, unix_now};
use slint::{ComponentHandle, ModelRc, Timer, VecModel};

use crate::account::{Act, Row};
use crate::app::App;
use crate::updater_logic::{
    Button, Phase, buttons, error_text, fraction, progress_text, status_line, trim_notes,
};

const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Wartezeit nach dem Start, bevor automatisch geprüft wird (der Start soll nicht konkurrieren).
const START_DELAY: Duration = Duration::from_secs(5);

/// Einstellungszeilen dieses Abschnitts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdAct {
    /// Automatisch prüfen an/aus.
    Auto,
    /// Kanal Stabil / Vorabversionen.
    Channel,
    /// Jetzt prüfen.
    Check,
    /// Beim Schließen im Infobereich weiterlaufen.
    TrayClose,
}

#[derive(Default)]
struct UpdCtl {
    settings: UpdateSettings,
    path: PathBuf,
    cache: PathBuf,
    phase: Phase,
    info: Option<UpdateInfo>,
    dialog: bool,
    focus: usize,
    progress: f32,
    progress_text: String,
    error: String,
    task: Option<tokio::task::AbortHandle>,
    /// Zähler gegen späte Antworten abgebrochener Vorgänge.
    generation: u64,
    cfg: Option<UpdateConfig>,
    dev: bool,
    /// Nur Entwicklung: Pfad des geprüften Pakets statt Installation.
    installed: Option<PathBuf>,
    timer: Timer,
}

thread_local! {
    static UPD: RefCell<UpdCtl> = RefCell::new(UpdCtl::default());
}

#[cfg(feature = "dev-automation")]
fn dev_config() -> Option<UpdateConfig> {
    let url = std::env::var("LFX_UPDATE_URL").ok()?;
    let mut cfg = UpdateConfig {
        stable_manifest_url: url,
        allow_loopback_http: true,
        ..UpdateConfig::default()
    };
    if let Ok(k) = std::env::var("LFX_UPDATE_KEY") {
        cfg.public_key = k;
    }
    if let Ok(p) = std::env::var("LFX_UPDATE_PLATFORM") {
        cfg.platform = Some(p);
    }
    Some(cfg)
}

#[cfg(not(feature = "dev-automation"))]
fn dev_config() -> Option<UpdateConfig> {
    None
}

fn channel_label(c: Channel) -> &'static str {
    match c {
        Channel::Stable => "Stabil",
        Channel::Prerelease => "Vorabversionen",
    }
}

impl App {
    /// Einstellungen laden und die automatische Prüfung einplanen.
    pub fn update_start(self: &Rc<Self>) {
        let path = self.dirs.config.join("update.json");
        let settings = UpdateSettings::load(&path);
        let dev = dev_config();
        let auto_allowed = self.opts.home.is_none() || dev.is_some();
        UPD.with(|u| {
            let mut u = u.borrow_mut();
            u.settings = settings;
            u.path = path;
            u.cache = self.dirs.cache.join("updates");
            u.dev = dev.is_some();
            u.cfg = Some(dev.unwrap_or_default());
        });
        self.update_wire();
        if auto_allowed {
            let a = self.clone();
            UPD.with(|u| {
                u.borrow()
                    .timer
                    .start(slint::TimerMode::SingleShot, START_DELAY, move || {
                        let due = UPD.with(|u| u.borrow().settings.check_due(unix_now()));
                        if due {
                            a.update_check(false);
                        }
                    });
            });
        }
    }

    fn update_wire(self: &Rc<Self>) {
        let a = self.clone();
        self.win()
            .global::<UpdateUi>()
            .on_action(move |i| a.update_press(i.max(0) as usize));
    }

    fn update_save(&self) {
        UPD.with(|u| {
            let u = u.borrow();
            if let Err(e) = u.settings.save(&u.path) {
                tracing::warn!("Update-Einstellungen nicht gespeichert: {e}");
            }
        });
    }

    /// Prüft jetzt (`manual`: Nutzer hat es angestoßen, dann gibt es immer eine Rückmeldung).
    pub fn update_check(self: &Rc<Self>, manual: bool) {
        let (cfg, channel, epoch) = UPD.with(|u| {
            let mut u = u.borrow_mut();
            if matches!(
                u.phase,
                Phase::Checking | Phase::Downloading | Phase::Installing
            ) {
                return (None, Channel::Stable, 0);
            }
            u.phase = Phase::Checking;
            u.generation += 1;
            (u.cfg.clone(), u.settings.channel, u.generation)
        });
        let Some(cfg) = cfg else { return };
        self.update_changed();
        let handle = self.rt.spawn(async move {
            let res = match Updater::new(cfg, VERSION) {
                Ok(up) => up.check(VERSION, channel).await,
                Err(e) => Err(e),
            };
            let res = res.map_err(|e| error_text(&e));
            crate::app::post(move |app| app.update_checked(epoch, manual, res));
        });
        UPD.with(|u| u.borrow_mut().task = Some(handle.abort_handle()));
    }

    fn update_checked(
        self: &Rc<Self>,
        epoch: u64,
        manual: bool,
        res: Result<Option<UpdateInfo>, String>,
    ) {
        let now = unix_now();
        let announce = UPD.with(|u| {
            let mut u = u.borrow_mut();
            if u.generation != epoch {
                return None;
            }
            u.task = None;
            u.settings.mark_checked(now);
            match res {
                Ok(Some(info)) => {
                    let show = manual || u.settings.should_announce(&info.version, now);
                    u.phase = Phase::Available;
                    u.info = Some(info);
                    u.error.clear();
                    u.dialog = show;
                    u.focus = 0;
                    Some((show, None))
                }
                Ok(None) => {
                    u.phase = Phase::UpToDate;
                    u.info = None;
                    Some((false, None))
                }
                Err(text) => {
                    u.phase = Phase::Error;
                    u.error.clone_from(&text);
                    Some((false, Some(text)))
                }
            }
        });
        let Some((_, error)) = announce else { return };
        self.update_save();
        self.update_changed();
        if manual {
            match error {
                Some(text) => self.toast(&text, ToastKind::Error),
                None if UPD.with(|u| u.borrow().phase == Phase::UpToDate) => {
                    self.toast("LENZFLIX ist auf dem neuesten Stand.", ToastKind::Success);
                }
                None => {}
            }
        }
    }

    /// Dialog und Einstellungszeile aus dem Zustand neu aufbauen.
    fn update_changed(self: &Rc<Self>) {
        self.update_push();
        if self.on_account_page() {
            self.acc_rebuild();
        }
    }

    fn update_buttons(&self) -> Vec<Button> {
        UPD.with(|u| {
            let u = u.borrow();
            buttons(u.phase, u.info.is_some())
        })
    }

    /// Schreibt den Dialogzustand in die Oberfläche.
    pub fn update_push(&self) {
        let ui = self.win();
        let g = ui.global::<UpdateUi>();
        let btns = self.update_buttons();
        UPD.with(|u| {
            let u = u.borrow();
            let open = u.dialog && u.phase != Phase::Idle;
            let version = u
                .info
                .as_ref()
                .map(|i| i.version.clone())
                .unwrap_or_default();
            let (title, subtitle) = match u.phase {
                Phase::Available => (
                    "Update verfügbar".to_owned(),
                    format!("LENZFLIX {version} ist da (installiert: {VERSION})."),
                ),
                Phase::Downloading => (
                    format!("LENZFLIX {version} wird geladen"),
                    "Das Paket wird anschließend auf Echtheit geprüft.".to_owned(),
                ),
                Phase::Installing => (
                    "Installation wird gestartet".to_owned(),
                    "LENZFLIX startet gleich neu.".to_owned(),
                ),
                Phase::Error => ("Update nicht möglich".to_owned(), String::new()),
                _ => (String::new(), String::new()),
            };
            g.set_open(open);
            g.set_title(title.into());
            g.set_subtitle(subtitle.into());
            g.set_notes(
                if u.phase == Phase::Available {
                    u.info
                        .as_ref()
                        .map(|i| trim_notes(&i.notes, 900))
                        .unwrap_or_default()
                } else {
                    String::new()
                }
                .into(),
            );
            g.set_error(
                if u.phase == Phase::Error {
                    u.error.clone()
                } else {
                    String::new()
                }
                .into(),
            );
            g.set_progress(u.progress);
            g.set_progress_text(
                if matches!(u.phase, Phase::Downloading | Phase::Installing) {
                    u.progress_text.clone()
                } else {
                    String::new()
                }
                .into(),
            );
            g.set_focus(u.focus.min(btns.len().saturating_sub(1)) as i32);
            let phase = u.phase;
            g.set_buttons(ModelRc::new(VecModel::from(
                btns.iter()
                    .map(|b| slint::SharedString::from(b.label(phase)))
                    .collect::<Vec<_>>(),
            )));
        });
    }

    /// Knopf des Dialogs (Mausklick, Tastatur, Automation).
    pub fn update_press(self: &Rc<Self>, index: usize) {
        let Some(b) = self.update_buttons().get(index).copied() else {
            return;
        };
        self.update_button(b);
    }

    fn update_button(self: &Rc<Self>, b: Button) {
        let now = unix_now();
        match b {
            Button::Install => self.update_download(),
            Button::Later => {
                UPD.with(|u| {
                    let mut u = u.borrow_mut();
                    u.settings.remind_later(now);
                    u.dialog = false;
                });
                self.update_save();
                self.update_changed();
            }
            Button::Skip => {
                UPD.with(|u| {
                    let mut u = u.borrow_mut();
                    if let Some(v) = u.info.as_ref().map(|i| i.version.clone()) {
                        u.settings.skip_version(&v);
                    }
                    u.dialog = false;
                });
                self.update_save();
                self.update_changed();
            }
            Button::Cancel => {
                UPD.with(|u| {
                    let mut u = u.borrow_mut();
                    u.generation += 1;
                    if let Some(t) = u.task.take() {
                        t.abort();
                    }
                    u.phase = Phase::Available;
                    u.progress = 0.0;
                    u.progress_text.clear();
                });
                self.update_changed();
            }
            Button::Close => {
                UPD.with(|u| {
                    let mut u = u.borrow_mut();
                    u.dialog = false;
                    u.phase = if u.info.is_some() {
                        Phase::Available
                    } else {
                        Phase::Idle
                    };
                });
                self.update_changed();
            }
        }
    }

    fn update_download(self: &Rc<Self>) {
        let (info, cfg, dir, epoch) = UPD.with(|u| {
            let mut u = u.borrow_mut();
            let Some(info) = u.info.clone() else {
                return (None, None, PathBuf::new(), 0);
            };
            u.phase = Phase::Downloading;
            u.progress = 0.0;
            u.progress_text = progress_text(0, info.asset.size);
            u.error.clear();
            u.generation += 1;
            (Some(info), u.cfg.clone(), u.cache.clone(), u.generation)
        });
        let (Some(info), Some(cfg)) = (info, cfg) else {
            return;
        };
        self.update_changed();
        let handle = self.rt.spawn(async move {
            let _ = tokio::fs::remove_dir_all(&dir).await;
            let res = match Updater::new(cfg, VERSION) {
                Ok(up) => {
                    let mut last_pct = -1i32;
                    up.download(&info, &dir, |done, total| {
                        let pct = (fraction(done, total) * 100.0) as i32;
                        if pct != last_pct {
                            last_pct = pct;
                            crate::app::post(move |app| app.update_progress(epoch, done, total));
                        }
                    })
                    .await
                }
                Err(e) => Err(e),
            };
            let res = res.map_err(|e| error_text(&e));
            crate::app::post(move |app| app.update_downloaded(epoch, res));
        });
        UPD.with(|u| u.borrow_mut().task = Some(handle.abort_handle()));
    }

    fn update_progress(self: &Rc<Self>, epoch: u64, done: u64, total: u64) {
        let ok = UPD.with(|u| {
            let mut u = u.borrow_mut();
            if u.generation != epoch || u.phase != Phase::Downloading {
                return false;
            }
            u.progress = fraction(done, total);
            u.progress_text = progress_text(done, total);
            true
        });
        if ok {
            self.update_push();
        }
    }

    fn update_downloaded(self: &Rc<Self>, epoch: u64, res: Result<PathBuf, String>) {
        let outcome = UPD.with(|u| {
            let mut u = u.borrow_mut();
            if u.generation != epoch {
                return None;
            }
            u.task = None;
            match res {
                Ok(path) => {
                    u.phase = Phase::Installing;
                    u.progress = 1.0;
                    Some(Ok((path, u.dev)))
                }
                Err(text) => {
                    u.phase = Phase::Error;
                    u.error.clone_from(&text);
                    Some(Err(text))
                }
            }
        });
        let Some(outcome) = outcome else { return };
        match outcome {
            Ok((path, dev)) => {
                self.update_changed();
                if dev {
                    // Entwicklung: nichts starten, nur den geprüften Pfad merken.
                    UPD.with(|u| u.borrow_mut().installed = Some(path));
                    return;
                }
                match lfx_update::install::start_install(&path) {
                    Ok(()) => self.quit_app(),
                    Err(e) => {
                        let text = error_text(&e);
                        UPD.with(|u| {
                            let mut u = u.borrow_mut();
                            u.phase = Phase::Error;
                            u.error = text;
                        });
                        self.update_changed();
                    }
                }
            }
            Err(_) => self.update_changed(),
        }
    }

    /// Navigationsaktion bei offenem Dialog; `true`, wenn der Dialog sie genommen hat.
    pub fn update_action(self: &Rc<Self>, action: NavAction) -> bool {
        let (open, focus) = UPD.with(|u| {
            let u = u.borrow();
            (u.dialog && u.phase != Phase::Idle, u.focus)
        });
        if !open {
            return false;
        }
        let btns = self.update_buttons();
        match action {
            NavAction::Left | NavAction::Up => {
                UPD.with(|u| u.borrow_mut().focus = focus.saturating_sub(1));
                self.update_push();
            }
            NavAction::Right | NavAction::Down => {
                UPD.with(|u| u.borrow_mut().focus = (focus + 1).min(btns.len().saturating_sub(1)));
                self.update_push();
            }
            NavAction::Activate => self.update_press(focus),
            NavAction::Back | NavAction::Menu => {
                // Zurück = die vorsichtigste Wahl: später / abbrechen / schließen.
                if let Some(b) = btns
                    .iter()
                    .copied()
                    .find(|b| matches!(b, Button::Later | Button::Cancel | Button::Close))
                {
                    self.update_button(b);
                }
            }
            _ => {}
        }
        true
    }

    // ---------------------------------------------------------------- Einstellungen

    /// Zeilen „Updates“ und „Fenster“ der Einstellungsseite.
    pub fn update_rows(&self) -> Vec<Row> {
        let (s, status) = UPD.with(|u| {
            let u = u.borrow();
            let version = u
                .info
                .as_ref()
                .map(|i| i.version.clone())
                .unwrap_or_default();
            (u.settings.clone(), status_line(u.phase, &version, &u.error))
        });
        vec![
            Row::heading("Updates"),
            Row::toggle(
                "Automatisch nach Updates suchen",
                s.enabled,
                Act::Upd(UpdAct::Auto),
            ),
            Row::choice(
                "Update-Kanal",
                channel_label(s.channel),
                Act::Upd(UpdAct::Channel),
            ),
            Row::info("Installierte Version", VERSION),
            Row::info("Update-Status", status),
            Row::button("Jetzt nach Updates suchen", Act::Upd(UpdAct::Check)),
            Row::heading("Fenster"),
            Row::toggle(
                "Beim Schließen im Infobereich weiterlaufen",
                self.tray_close_enabled(),
                Act::Upd(UpdAct::TrayClose),
            ),
            Row::note(if cfg!(target_os = "macos") {
                "LENZFLIX bleibt in der Menüleiste und kann dort geöffnet oder beendet werden."
            } else {
                "LENZFLIX bleibt im Infobereich der Taskleiste und kann dort geöffnet oder beendet werden."
            }),
        ]
    }

    /// Schalter/Auswahl in den Einstellungen (`dir` ±1).
    pub fn update_adjust(self: &Rc<Self>, act: UpdAct, dir: i32) {
        match act {
            UpdAct::Auto => {
                UPD.with(|u| {
                    let mut u = u.borrow_mut();
                    u.settings.enabled = !u.settings.enabled;
                });
                self.update_save();
            }
            UpdAct::Channel => {
                UPD.with(|u| {
                    let mut u = u.borrow_mut();
                    let _ = dir;
                    u.settings.channel = match u.settings.channel {
                        Channel::Stable => Channel::Prerelease,
                        Channel::Prerelease => Channel::Stable,
                    };
                    // Anderer Kanal: neu prüfen lassen, alte Meldung verwerfen.
                    u.settings.last_check = 0;
                    u.settings.skipped_version = None;
                });
                self.update_save();
            }
            UpdAct::Check => self.update_check(true),
            UpdAct::TrayClose => {
                let on = !self.tray_close_enabled();
                self.tray_set_close_enabled(on);
            }
        }
        if self.on_account_page() {
            self.acc_rebuild();
        }
    }

    /// Zustand für Tests.
    pub fn update_state_json(&self) -> serde_json::Value {
        let btns = self.update_buttons();
        UPD.with(|u| {
            let u = u.borrow();
            serde_json::json!({
                "phase": u.phase.name(),
                "dialog": u.dialog,
                "version": u.info.as_ref().map(|i| i.version.clone()),
                "notes": u.info.as_ref().map(|i| i.notes.clone()),
                "progress": u.progress,
                "progress_text": u.progress_text,
                "error": u.error,
                "buttons": btns.iter().map(|b| b.name()).collect::<Vec<_>>(),
                "focus": u.focus,
                "enabled": u.settings.enabled,
                "channel": channel_label(u.settings.channel),
                "skipped": u.settings.skipped_version,
                "remind_after": u.settings.remind_after,
                "installed": u.installed.as_ref().map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())),
            })
        })
    }

    /// Nur Entwicklung: setzt „zuletzt geprüft“ zurück (damit die Startprüfung fällig ist).
    pub fn update_reset_for_test(&self) {
        UPD.with(|u| {
            let mut u = u.borrow_mut();
            u.settings = UpdateSettings::default();
            u.phase = Phase::Idle;
            u.info = None;
            u.dialog = false;
        });
        self.update_save();
    }
}
