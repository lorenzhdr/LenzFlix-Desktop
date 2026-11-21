//! W5-downloads: Download-Dialog (Qualität, Umfang, „nur Ungesehene“) und Einreihen.

use std::rc::Rc;

use lfx_api::{BaseItem, ItemKind};
use lfx_nav::NavAction;
use lfx_offline::{EnqueueOptions, OfflineError, SpaceProbe, SystemSpace, TranscodeProfile};
use lfx_ui::{DlOpt, DlState, ToastKind};
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::app::{App, post};

/// Qualitätsstufen im Dialog.
const QUALITIES: [&str; 3] = [
    "Original",
    "Mittel (720p, 3 MBit/s)",
    "Klein (480p, 1,2 MBit/s)",
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Opt {
    Scope,
    Quality,
    Unplayed,
    Note,
}

/// Offener Dialog.
pub struct Dialog {
    item: BaseItem,
    /// Staffel, die „Diese Staffel“ meint (nur bei Serien aus der Detailseite).
    season: Option<(String, String)>,
    opts: Vec<Opt>,
    scope_season: bool,
    quality: usize,
    unplayed: bool,
    focus: usize,
    busy: bool,
}

impl Dialog {
    /// Fokusziele: bedienbare Optionen, danach „Abbrechen“ und „Herunterladen“.
    fn slots(&self) -> Vec<usize> {
        let mut v: Vec<usize> = self
            .opts
            .iter()
            .enumerate()
            .filter(|(_, o)| **o != Opt::Note)
            .map(|(i, _)| i)
            .collect();
        v.push(self.opts.len());
        v.push(self.opts.len() + 1);
        v
    }
}

/// Darf diese Art geladen werden?
pub fn downloadable(kind: ItemKind) -> bool {
    matches!(
        kind,
        ItemKind::Movie | ItemKind::Video | ItemKind::Episode | ItemKind::Series | ItemKind::Season
    )
}

impl App {
    /// Öffnet den Dialog zu einem Titel (Detailseite, Modal, Hovercard, Kontextmenü).
    pub fn dl_open_dialog(self: &Rc<Self>, item: &BaseItem) {
        if !downloadable(item.kind) {
            self.toast(
                "Dieser Titel lässt sich nicht herunterladen.",
                ToastKind::Info,
            );
            return;
        }
        if self.dl_manager().is_none() {
            self.toast("Bitte zuerst anmelden.", ToastKind::Info);
            return;
        }
        if self.dl_offline() {
            self.toast(
                "Herunterladen braucht eine Verbindung zum Server.",
                ToastKind::Info,
            );
            return;
        }
        // Schon da?
        if item.kind != ItemKind::Series
            && item.kind != ItemKind::Season
            && let Some(m) = self.dl_manager()
            && let Ok(id) = m.download_id(&item.id)
            && let Ok(Some(rec)) = m.get(&id)
        {
            let text = if rec.state == lfx_offline::DownloadState::Completed {
                "Schon heruntergeladen – zu finden unter Downloads."
            } else {
                "Läuft schon – Fortschritt unter Downloads."
            };
            self.toast(text, ToastKind::Info);
            return;
        }
        let season = if item.kind == ItemKind::Series {
            let st = self.st.borrow();
            let d = &st.detail;
            (d.item.as_ref().is_some_and(|i| i.id == item.id) && d.seasons.len() > 1)
                .then(|| d.seasons.get(d.season_idx))
                .flatten()
                .map(|s| (s.id.clone(), s.name.clone()))
        } else {
            None
        };
        let mut opts = Vec::new();
        if season.is_some() {
            opts.push(Opt::Scope);
        }
        opts.push(Opt::Quality);
        if matches!(item.kind, ItemKind::Series | ItemKind::Season) {
            opts.push(Opt::Unplayed);
        }
        opts.push(Opt::Note);
        let quality = self
            .local_get(crate::downloads::KEY_QUALITY)
            .and_then(|v| v.as_u64())
            .map_or(0, |n| (n as usize).min(QUALITIES.len() - 1));
        let d = Dialog {
            item: item.clone(),
            season,
            opts,
            scope_season: false,
            quality,
            unplayed: true,
            focus: 0,
            busy: false,
        };
        self.st.borrow_mut().downloads.dialog = Some(d);
        self.dl_push_dialog();
    }

    pub fn dl_dialog_open(&self) -> bool {
        self.st.borrow().downloads.dialog.is_some()
    }

    fn dl_free_text(&self) -> String {
        match SystemSpace.available(&self.dirs.data) {
            Some(b) => format!(
                "Freier Speicher: {}",
                crate::downloads_page::format_bytes(b)
            ),
            None => String::new(),
        }
    }

    /// Schreibt den Dialogzustand in die Oberfläche.
    pub fn dl_push_dialog(self: &Rc<Self>) {
        let free = self.dl_free_text();
        let ui = self.win();
        let g = ui.global::<DlState>();
        let st = self.st.borrow();
        let Some(d) = &st.downloads.dialog else {
            g.set_dlg_open(false);
            return;
        };
        let title = match d.item.kind {
            ItemKind::Series if d.scope_season => d
                .season
                .as_ref()
                .map(|s| format!("{} – {}", d.item.name, s.1))
                .unwrap_or_else(|| d.item.name.clone()),
            _ => d.item.name.clone(),
        };
        let rows: Vec<DlOpt> = d
            .opts
            .iter()
            .map(|o| match o {
                Opt::Scope => DlOpt {
                    kind: 0,
                    label: "Umfang".into(),
                    value: if d.scope_season {
                        d.season
                            .as_ref()
                            .map_or("Diese Staffel".to_owned(), |s| s.1.clone())
                    } else {
                        "Ganze Serie".into()
                    }
                    .into(),
                    on: false,
                },
                Opt::Quality => DlOpt {
                    kind: 0,
                    label: "Qualität".into(),
                    value: QUALITIES[d.quality].into(),
                    on: false,
                },
                Opt::Unplayed => DlOpt {
                    kind: 1,
                    label: "Nur ungesehene Folgen".into(),
                    value: "".into(),
                    on: d.unplayed,
                },
                Opt::Note => DlOpt {
                    kind: 2,
                    label: if d.quality == 0 {
                        format!("Die Originaldatei wird unverändert geladen. {free}")
                    } else {
                        format!("Der Server wandelt das Video um; das kann dauern. {free}")
                    }
                    .into(),
                    value: "".into(),
                    on: false,
                },
            })
            .collect();
        g.set_dlg_title(format!("Herunterladen: {title}").into());
        g.set_dlg_opts(ModelRc::new(VecModel::from(rows)));
        let slots = d.slots();
        g.set_dlg_focus(slots.get(d.focus).copied().unwrap_or(0) as i32);
        g.set_dlg_busy(d.busy);
        g.set_dlg_open(true);
    }

    pub fn dl_close_dialog(self: &Rc<Self>) {
        self.st.borrow_mut().downloads.dialog = None;
        self.win().global::<DlState>().set_dlg_open(false);
    }

    /// Taste im Dialog; `true` = verbraucht.
    pub fn dl_dialog_action(self: &Rc<Self>, action: NavAction) -> bool {
        let (slot, n_slots, opt) = {
            let st = self.st.borrow();
            let Some(d) = &st.downloads.dialog else {
                return false;
            };
            let slots = d.slots();
            let slot = slots.get(d.focus).copied().unwrap_or(0);
            (slot, slots.len(), d.opts.get(slot).copied())
        };
        match action {
            NavAction::Up | NavAction::Down => {
                let step: isize = if action == NavAction::Down { 1 } else { -1 };
                let mut st = self.st.borrow_mut();
                if let Some(d) = st.downloads.dialog.as_mut() {
                    d.focus = (d.focus as isize + step).rem_euclid(n_slots as isize) as usize;
                }
            }
            NavAction::Left | NavAction::Right => {
                let dir = if action == NavAction::Right { 1 } else { -1 };
                match opt {
                    Some(o) => self.dl_adjust(o, dir),
                    None => {
                        // Zwischen den Knöpfen wechseln
                        let mut st = self.st.borrow_mut();
                        if let Some(d) = st.downloads.dialog.as_mut() {
                            d.focus = (d.focus as isize + dir)
                                .clamp(n_slots as isize - 2, n_slots as isize - 1)
                                as usize;
                        }
                    }
                }
            }
            NavAction::Activate => match opt {
                Some(o) => self.dl_adjust(o, 1),
                None if slot == self.dl_opts_len() => self.dl_close_dialog(),
                None => {
                    self.dl_confirm_dialog();
                    return true;
                }
            },
            NavAction::Back | NavAction::Menu => {
                self.dl_close_dialog();
                return true;
            }
            _ => {}
        }
        self.dl_push_dialog();
        true
    }

    fn dl_opts_len(&self) -> usize {
        self.st
            .borrow()
            .downloads
            .dialog
            .as_ref()
            .map_or(0, |d| d.opts.len())
    }

    fn dl_adjust(self: &Rc<Self>, o: Opt, dir: isize) {
        let mut st = self.st.borrow_mut();
        let Some(d) = st.downloads.dialog.as_mut() else {
            return;
        };
        match o {
            Opt::Scope => d.scope_season = !d.scope_season,
            Opt::Quality => {
                d.quality =
                    (d.quality as isize + dir).rem_euclid(QUALITIES.len() as isize) as usize;
            }
            Opt::Unplayed => d.unplayed = !d.unplayed,
            Opt::Note => {}
        }
    }

    /// Klick aus der Oberfläche.
    pub fn dl_dialog_event(self: &Rc<Self>, kind: &str, a: i32) {
        match kind {
            "dlg-opt" => {
                let o = self
                    .st
                    .borrow()
                    .downloads
                    .dialog
                    .as_ref()
                    .and_then(|d| d.opts.get(a.max(0) as usize).copied());
                if let Some(o) = o {
                    self.dl_adjust(o, 1);
                    self.dl_push_dialog();
                }
            }
            "dlg-btn" => {
                if a == 0 {
                    self.dl_close_dialog();
                } else {
                    self.dl_confirm_dialog();
                }
            }
            _ => {}
        }
    }

    /// Reiht ein.
    pub fn dl_confirm_dialog(self: &Rc<Self>) {
        let (target, options, quality, name) = {
            let mut st = self.st.borrow_mut();
            let Some(d) = st.downloads.dialog.as_mut() else {
                return;
            };
            if d.busy {
                return;
            }
            d.busy = true;
            let target = if d.scope_season
                && let Some((id, _)) = &d.season
            {
                id.clone()
            } else {
                d.item.id.clone()
            };
            let mut opts = match d.quality {
                0 => EnqueueOptions::original(),
                1 => EnqueueOptions::transcoded(TranscodeProfile::medium()),
                _ => EnqueueOptions::transcoded(TranscodeProfile::small()),
            };
            opts.only_unplayed = d.unplayed && d.opts.contains(&Opt::Unplayed);
            (target, opts, d.quality, d.item.name.clone())
        };
        self.dl_push_dialog();
        self.save_setting(crate::downloads::KEY_QUALITY, serde_json::json!(quality));
        let Some(mgr) = self.dl_manager() else {
            return;
        };
        self.rt.spawn(async move {
            let result = mgr.enqueue(&target, options).await;
            post(move |a| {
                a.dl_close_dialog();
                match result {
                    Ok(ids) if ids.is_empty() => {
                        a.toast("Nichts zu laden – alles schon vorhanden.", ToastKind::Info)
                    }
                    Ok(ids) => {
                        let text = if ids.len() == 1 {
                            format!("„{name}“ wird heruntergeladen.")
                        } else {
                            format!("{} Titel werden heruntergeladen.", ids.len())
                        };
                        a.toast(&text, ToastKind::Success);
                    }
                    Err(OfflineError::NotAllowed) => a.toast(
                        "Dieses Konto darf keine Originale herunterladen.",
                        ToastKind::Error,
                    ),
                    Err(OfflineError::InsufficientSpace { .. }) => {
                        a.toast("Zu wenig freier Speicherplatz.", ToastKind::Error);
                    }
                    Err(e) => {
                        tracing::warn!("Download nicht eingereiht: {e}");
                        a.toast(
                            "Der Download konnte nicht gestartet werden.",
                            ToastKind::Error,
                        );
                    }
                }
                a.dl_schedule_refresh();
            });
        });
    }
}
