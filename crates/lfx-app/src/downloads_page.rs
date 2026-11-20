//! W5-downloads: Seite „Downloads“ (laufende Downloads, Offline-Bibliothek, Einstellungen). Rust hält Zeilen und
//! Fokus, `pages/downloads.slint` zeigt sie über `DlState`.

use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

use lfx_nav::{InputContext, NavAction};
use lfx_offline::{DownloadRecord, DownloadState, SpaceProbe, SystemSpace};
use lfx_ui::{DlRow, DlState, ToastKind};
use slint::{ComponentHandle, Image, ModelRc, Rgba8Pixel, SharedPixelBuffer, VecModel};

use crate::app::App;
use crate::route::Route;

/// Was ein Knopf oder eine Zeile tut.
#[derive(Clone, Debug, PartialEq)]
pub enum RAct {
    None,
    Pause(String),
    Resume(String),
    Cancel(String),
    Play(String),
    Delete(String),
    ToggleAutoDelete,
    PauseAll,
    ResumeAll,
    DeleteAll,
}

#[derive(Clone, Debug)]
pub struct PRow {
    pub kind: i32,
    pub h: f32,
    pub title: String,
    pub sub: String,
    pub right: String,
    pub progress: f32,
    pub state: i32,
    pub a: Option<(String, RAct)>,
    pub b: Option<(String, RAct)>,
    pub on: bool,
    pub danger: bool,
    /// Titel-Id für das Vorschaubild.
    pub image_of: Option<String>,
    pub act: RAct,
}

impl PRow {
    fn new(kind: i32, h: f32, title: impl Into<String>) -> Self {
        Self {
            kind,
            h,
            title: title.into(),
            sub: String::new(),
            right: String::new(),
            progress: -1.0,
            state: 0,
            a: None,
            b: None,
            on: false,
            danger: false,
            image_of: None,
            act: RAct::None,
        }
    }
    fn focusable(&self) -> bool {
        self.a.is_some() || self.b.is_some() || matches!(self.kind, 4 | 6)
    }
}

/// Zustand der Seite.
#[derive(Default)]
pub struct PageState {
    pub rows: Vec<PRow>,
    pub focus: usize,
    pub col: usize,
    pub confirm: Option<(RAct, i32)>,
    pub status: String,
    pub images: HashMap<String, Image>,
}

/// Bytes lesbar (Binärpräfixe, deutsches Komma).
pub fn format_bytes(b: u64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{b} B")
    } else {
        format!("{v:.1} {}", U[i]).replace('.', ",")
    }
}

fn title_of(r: &DownloadRecord) -> (String, String) {
    match (r.series_name.as_deref(), r.season_number, r.episode_number) {
        (Some(s), sn, en) if !s.is_empty() => {
            let ep = match (sn, en) {
                (Some(a), Some(b)) => format!("Staffel {a}, Folge {b}"),
                (_, Some(b)) => format!("Folge {b}"),
                _ => String::new(),
            };
            (
                r.name.clone(),
                if ep.is_empty() {
                    s.to_owned()
                } else {
                    format!("{s} · {ep}")
                },
            )
        }
        _ => (
            r.name.clone(),
            r.year.map(|y| y.to_string()).unwrap_or_default(),
        ),
    }
}

fn load_image(path: &Path) -> Option<Image> {
    let bytes = std::fs::read(path).ok()?;
    let img = lfx_images::decode_image(&bytes, Some(320), Some(320), false).ok()?;
    let mut buf = SharedPixelBuffer::<Rgba8Pixel>::new(img.width, img.height);
    let dst = buf.make_mut_bytes();
    let n = dst.len().min(img.pixels.len());
    dst[..n].copy_from_slice(&img.pixels[..n]);
    Some(Image::from_rgba8(buf))
}

impl App {
    // ---------------------------------------------------------------- Aufbau

    fn dl_build_rows(&self) -> (Vec<PRow>, String) {
        let recs = self.dl_records();
        let mut running: Vec<&DownloadRecord> = recs
            .iter()
            .filter(|r| r.state != DownloadState::Completed)
            .collect();
        running.sort_by_key(|r| r.created_ms);
        let mut done: Vec<&DownloadRecord> = recs
            .iter()
            .filter(|r| r.state == DownloadState::Completed)
            .collect();
        done.sort_by(|a, b| {
            let k = |r: &DownloadRecord| {
                (
                    r.series_name.clone().unwrap_or_default(),
                    r.season_number.unwrap_or(0),
                    r.episode_number.unwrap_or(0),
                    r.name.clone(),
                )
            };
            k(a).cmp(&k(b))
        });
        let mut rows = Vec::new();
        rows.push(PRow::new(0, 44.0, "Laufende Downloads"));
        if running.is_empty() {
            rows.push(PRow::new(1, 40.0, "Keine laufenden Downloads."));
        }
        for r in &running {
            let (name, sub) = title_of(r);
            let mut row = PRow::new(2, 88.0, name);
            row.sub = sub;
            row.image_of = Some(r.item_id.clone());
            let total = r.bytes_total.unwrap_or(r.size_estimate).max(1);
            row.progress = (r.bytes_done as f32 / total as f32).clamp(0.0, 1.0);
            let state_text = match r.state {
                DownloadState::Queued => "Wartet".to_owned(),
                DownloadState::Downloading => "Läuft".to_owned(),
                DownloadState::Paused => "Pausiert".to_owned(),
                DownloadState::Failed => format!(
                    "Fehlgeschlagen{}",
                    r.error
                        .as_deref()
                        .map(|e| format!(": {e}"))
                        .unwrap_or_default()
                ),
                DownloadState::Completed => String::new(),
            };
            row.state = match r.state {
                DownloadState::Queued => 0,
                DownloadState::Downloading => 1,
                DownloadState::Paused => 2,
                DownloadState::Failed => 4,
                DownloadState::Completed => 3,
            };
            row.right = format!(
                "{state_text} · {} von {} ({} %)",
                format_bytes(r.bytes_done),
                format_bytes(total),
                (row.progress * 100.0).round() as i32
            );
            row.a = Some(match r.state {
                DownloadState::Paused => ("Fortsetzen".into(), RAct::Resume(r.id.clone())),
                DownloadState::Failed => ("Wiederholen".into(), RAct::Resume(r.id.clone())),
                _ => ("Pause".into(), RAct::Pause(r.id.clone())),
            });
            row.b = Some(("Abbrechen".into(), RAct::Cancel(r.id.clone())));
            row.danger = true;
            rows.push(row);
        }
        rows.push(PRow::new(0, 44.0, "Offline-Bibliothek"));
        if done.is_empty() {
            rows.push(PRow::new(
                1,
                40.0,
                "Noch nichts heruntergeladen. Über das Symbol „Herunterladen“ bei einem Titel lässt sich etwas für unterwegs laden.",
            ));
        }
        for r in &done {
            let (name, sub) = title_of(r);
            let mut row = PRow::new(3, 88.0, name);
            row.sub = sub;
            row.image_of = Some(r.item_id.clone());
            let seen = r.local.as_ref().is_some_and(|l| l.played);
            let pos = r.local.as_ref().map_or(0, |l| l.position_ticks);
            row.right = format!(
                "{} · {}",
                format_bytes(r.bytes_total.unwrap_or(r.bytes_done)),
                if seen {
                    "gesehen".to_owned()
                } else if pos > 0 {
                    "angefangen".to_owned()
                } else {
                    "ungesehen".to_owned()
                }
            );
            row.state = 3;
            row.a = Some(("Abspielen".into(), RAct::Play(r.item_id.clone())));
            row.b = Some(("Löschen".into(), RAct::Delete(r.item_id.clone())));
            row.danger = true;
            rows.push(row);
        }
        // Einstellungen und Belegung
        rows.push(PRow::new(0, 44.0, "Einstellungen"));
        let auto = self.dl_library().is_some_and(|l| l.auto_delete());
        let mut t = PRow::new(4, 52.0, "Gesehene Titel automatisch löschen");
        t.on = auto;
        t.act = RAct::ToggleAutoDelete;
        rows.push(t);
        let usage = self.dl_library().and_then(|l| l.usage().ok());
        let used = usage
            .as_ref()
            .map_or(0, |u| u.completed_bytes + u.partial_bytes);
        let mut used_row = PRow::new(5, 52.0, "Belegt");
        used_row.right = format_bytes(used);
        rows.push(used_row);
        let free = SystemSpace.available(&self.dirs.data);
        let mut free_row = PRow::new(5, 52.0, "Freier Speicher");
        free_row.right = free.map_or("unbekannt".into(), format_bytes);
        rows.push(free_row);
        let mut loc = PRow::new(5, 52.0, "Speicherort");
        loc.right = "Einstellungen › Downloads".into();
        rows.push(loc);
        if !running.is_empty() {
            let any_active = running
                .iter()
                .any(|r| matches!(r.state, DownloadState::Downloading | DownloadState::Queued));
            let mut b = PRow::new(
                6,
                52.0,
                if any_active {
                    "Alle pausieren"
                } else {
                    "Alle fortsetzen"
                },
            );
            b.act = if any_active {
                RAct::PauseAll
            } else {
                RAct::ResumeAll
            };
            rows.push(b);
        }
        if !recs.is_empty() {
            let mut b = PRow::new(6, 52.0, "Alle Downloads löschen");
            b.act = RAct::DeleteAll;
            b.danger = true;
            rows.push(b);
        }
        let n_done = done.len();
        let sub = format!("{} {} · {} belegt", n_done, "Titel", format_bytes(used));
        (rows, sub)
    }

    /// Schreibt Zeilen, Fokus und Texte in die Oberfläche.
    pub fn dl_refresh_page(self: &Rc<Self>) {
        if !matches!(self.st.borrow().router.current(), Route::Downloads) {
            return;
        }
        let (rows, sub) = self.dl_build_rows();
        // Bilder (zwischengespeichert)
        let local = self.st.borrow().downloads.images.clone();
        let mut cache = std::mem::take(&mut self.st.borrow_mut().downloads.page.images);
        let mut y = 0.0f32;
        let mut out = Vec::with_capacity(rows.len());
        for r in &rows {
            let img = r.image_of.as_ref().and_then(|id| {
                if let Some(i) = cache.get(id) {
                    return Some(i.clone());
                }
                let files = local.get(id)?;
                let p = files.get("primary").or_else(|| files.get("backdrop"))?;
                let i = load_image(p)?;
                cache.insert(id.clone(), i.clone());
                Some(i)
            });
            out.push(DlRow {
                kind: r.kind,
                y,
                h: r.h,
                title: r.title.clone().into(),
                sub: r.sub.clone().into(),
                right: r.right.clone().into(),
                progress: r.progress,
                state: r.state,
                a_label: r.a.as_ref().map(|a| a.0.clone()).unwrap_or_default().into(),
                b_label: r.b.as_ref().map(|a| a.0.clone()).unwrap_or_default().into(),
                on: r.on,
                danger: r.danger,
                has_image: img.is_some(),
                image: img.unwrap_or_default(),
            });
            y += r.h;
        }
        let focus = {
            let mut st = self.st.borrow_mut();
            let p = &mut st.downloads.page;
            p.images = cache;
            p.rows = rows;
            if p.rows.get(p.focus).is_none_or(|r| !r.focusable()) {
                p.focus = p.rows.iter().position(PRow::focusable).unwrap_or(0);
            }
            p.col = p.col.min(1);
            if let Some(r) = p.rows.get(p.focus) {
                match (&r.a, &r.b) {
                    (Some(_), None) => p.col = 0,
                    (None, Some(_)) => p.col = 1,
                    _ => {}
                }
            }
            p.focus
        };
        let ui = self.win();
        let g = ui.global::<DlState>();
        g.set_rows(ModelRc::new(VecModel::from(out)));
        g.set_content_h(y);
        g.set_subtitle(sub.into());
        g.set_focus_row(focus as i32);
        g.set_focus_col(self.st.borrow().downloads.page.col as i32);
        let status = self.st.borrow().downloads.page.status.clone();
        g.set_status(status.into());
        g.set_title("Downloads".into());
    }

    /// Seite betreten.
    pub fn dl_enter_page(self: &Rc<Self>) {
        {
            let mut st = self.st.borrow_mut();
            st.downloads.page.focus = usize::MAX;
            st.downloads.page.confirm = None;
        }
        self.dl_reload_images();
        let ui = self.win();
        ui.global::<DlState>().set_confirm_open(false);
        self.dl_refresh_page();
    }

    // ---------------------------------------------------------------- Eingabe

    /// Taste auf der Downloads-Seite oder im Dialog; `true` = verbraucht.
    pub fn dl_handle_action(self: &Rc<Self>, action: NavAction) -> bool {
        if self.dl_dialog_open() {
            return self.dl_dialog_action(action);
        }
        if !matches!(self.st.borrow().router.current(), Route::Downloads) {
            return false;
        }
        // Rückfrage
        let confirm = self.st.borrow().downloads.page.confirm.clone();
        if let Some((act, focus)) = confirm {
            match action {
                NavAction::Left | NavAction::Right => {
                    self.dl_set_confirm(Some((act, 1 - focus)));
                }
                NavAction::Activate => {
                    self.dl_set_confirm(None);
                    if focus == 1 {
                        self.dl_do(act);
                    }
                }
                NavAction::Back | NavAction::Menu => self.dl_set_confirm(None),
                _ => {}
            }
            return true;
        }
        match action {
            NavAction::Up | NavAction::Down => {
                let step: isize = if action == NavAction::Down { 1 } else { -1 };
                let mut st = self.st.borrow_mut();
                let p = &mut st.downloads.page;
                let mut i = p.focus as isize;
                loop {
                    i += step;
                    if i < 0 || i >= p.rows.len() as isize {
                        break;
                    }
                    if p.rows[i as usize].focusable() {
                        p.focus = i as usize;
                        break;
                    }
                }
                drop(st);
                self.dl_refresh_page();
            }
            NavAction::Left | NavAction::Right => {
                let dir = if action == NavAction::Right { 1 } else { 0 };
                {
                    let mut st = self.st.borrow_mut();
                    let p = &mut st.downloads.page;
                    if let Some(r) = p.rows.get(p.focus)
                        && r.a.is_some()
                        && r.b.is_some()
                    {
                        p.col = dir;
                    }
                }
                self.dl_refresh_page();
            }
            NavAction::Activate => {
                let (focus, col) = {
                    let st = self.st.borrow();
                    (st.downloads.page.focus, st.downloads.page.col)
                };
                self.dl_activate(focus, Some(col));
            }
            NavAction::Back => self.go_back(),
            _ => return false,
        }
        true
    }

    fn dl_set_confirm(self: &Rc<Self>, c: Option<(RAct, i32)>) {
        let ui = self.win();
        let g = ui.global::<DlState>();
        match &c {
            Some((act, f)) => {
                let (text, yes) = match act {
                    RAct::Cancel(_) => (
                        "Diesen Download abbrechen? Die bisher geladenen Daten werden gelöscht.",
                        "Abbrechen",
                    ),
                    RAct::Delete(_) => (
                        "Diesen Titel von diesem Gerät löschen? Auf dem Server bleibt er erhalten.",
                        "Löschen",
                    ),
                    _ => (
                        "Alle Downloads und alle heruntergeladenen Dateien von diesem Gerät löschen?",
                        "Alle löschen",
                    ),
                };
                g.set_confirm_text(text.into());
                g.set_confirm_yes(yes.into());
                g.set_confirm_focus(*f);
                g.set_confirm_open(true);
            }
            None => g.set_confirm_open(false),
        }
        self.st.borrow_mut().downloads.page.confirm = c;
    }

    /// Zeile (und Knopf) aktivieren.
    pub fn dl_activate(self: &Rc<Self>, row: usize, col: Option<usize>) {
        let act = {
            let st = self.st.borrow();
            let Some(r) = st.downloads.page.rows.get(row) else {
                return;
            };
            match col {
                Some(0) if r.a.is_some() => r.a.clone().map(|a| a.1),
                Some(1) if r.b.is_some() => r.b.clone().map(|a| a.1),
                _ if r.a.is_none() && r.b.is_none() => Some(r.act.clone()),
                _ => r.a.clone().map(|a| a.1),
            }
        };
        let Some(act) = act else { return };
        match act {
            RAct::Cancel(_) | RAct::Delete(_) | RAct::DeleteAll => {
                self.dl_set_confirm(Some((act, 0)));
            }
            other => self.dl_do(other),
        }
    }

    /// Führt eine Aktion aus.
    pub fn dl_do(self: &Rc<Self>, act: RAct) {
        let Some(mgr) = self.dl_manager() else { return };
        match act {
            RAct::None => {}
            RAct::Pause(id) => {
                let _ = mgr.pause(&id);
            }
            RAct::Resume(id) => {
                let _ = mgr.resume(&id);
            }
            RAct::PauseAll => {
                let _ = mgr.pause_all();
            }
            RAct::ResumeAll => {
                let _ = mgr.resume_all();
            }
            RAct::Cancel(id) => {
                let m = mgr.clone();
                self.rt.spawn(async move {
                    let _ = m.cancel(&id).await;
                });
            }
            RAct::Delete(item_id) => {
                if let Some(lib) = self.dl_library() {
                    if let Err(e) = lib.delete(&item_id) {
                        tracing::warn!("Download nicht gelöscht: {e}");
                        self.toast("Der Titel ließ sich nicht löschen.", ToastKind::Error);
                    }
                    self.dl_reload_images();
                    self.st.borrow_mut().downloads.page.images.remove(&item_id);
                }
            }
            RAct::DeleteAll => {
                let recs = self.dl_records();
                let lib = self.dl_library();
                let m = mgr.clone();
                self.rt.spawn(async move {
                    for r in recs {
                        if r.state == DownloadState::Completed {
                            if let Some(l) = &lib {
                                let _ = l.delete(&r.item_id);
                            }
                        } else {
                            let _ = m.cancel(&r.id).await;
                        }
                    }
                    crate::app::post(|a| {
                        a.dl_reload_images();
                        a.dl_schedule_refresh();
                    });
                });
            }
            RAct::Play(item_id) => {
                self.dl_play_item(&item_id);
                return;
            }
            RAct::ToggleAutoDelete => {
                let on = !self.dl_library().is_some_and(|l| l.auto_delete());
                self.save_setting(crate::downloads::KEY_AUTO_DELETE, serde_json::json!(on));
                self.dl_setting_changed(crate::downloads::KEY_AUTO_DELETE);
            }
        }
        self.dl_refresh_page();
    }

    /// Spielt einen heruntergeladenen Titel (auch online) aus der Datei.
    pub fn dl_play_item(self: &Rc<Self>, item_id: &str) {
        let Some(lib) = self.dl_library() else { return };
        let Ok(Some(entry)) = lib.entry(item_id) else {
            self.toast("Der Titel ist nicht mehr vorhanden.", ToastKind::Error);
            return;
        };
        match Self::dl_item_of(&lib, &entry) {
            Some(item) => self.play_item(&item),
            None => self.toast("Der Titel lässt sich nicht öffnen.", ToastKind::Error),
        }
    }

    /// Klick aus der Oberfläche.
    pub fn dl_event(self: &Rc<Self>, kind: &str, a: i32, b: i32) {
        match kind {
            "row" => {
                self.st.borrow_mut().downloads.page.focus = a.max(0) as usize;
                self.dl_activate(a.max(0) as usize, None);
            }
            "btn" => {
                {
                    let mut st = self.st.borrow_mut();
                    st.downloads.page.focus = a.max(0) as usize;
                    st.downloads.page.col = b.max(0) as usize;
                }
                self.dl_activate(a.max(0) as usize, Some(b.max(0) as usize));
            }
            "confirm" => {
                let c = self.st.borrow().downloads.page.confirm.clone();
                self.dl_set_confirm(None);
                if let (1, Some((act, _))) = (a, c) {
                    self.dl_do(act);
                }
            }
            "banner" => self.go(Route::Downloads),
            k if k.starts_with("dlg-") => self.dl_dialog_event(k, a),
            _ => {}
        }
    }

    /// Eintritt in die Route (Aufruf aus `render_route`).
    pub fn dl_render_route(self: &Rc<Self>) -> InputContext {
        self.win().set_page(lfx_ui::Page::Downloads);
        self.st.borrow_mut().graph = lfx_nav::FocusGraph::new();
        self.dl_enter_page();
        InputContext::Browse
    }
}
