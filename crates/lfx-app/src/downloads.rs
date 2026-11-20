//! W5-downloads: Offline-Modus der App. Verbindet `lfx-offline` (Download-Manager, Offline-Bibliothek, Fortschritt
//! offline, Abgleich) mit Sitzung, Startseite und Player.
//!
//! * Der Manager lebt je angemeldetem Konto ([`App::dl_session_ready`]); seine Ereignisse laufen über [`post`] in
//!   die Oberfläche (Seite „Downloads“, Hinweise).
//! * Erreichbarkeit kommt aus `lfx-session` (`SessionEvent::Connectivity`); offline zeigt die Startseite die
//!   Offline-Bibliothek, Karten spielen lokal, Fortschritt geht in die Outbox und wird beim Wiederkommen
//!   abgeglichen ([`App::dl_sync`]).
//! * Seite: `downloads_page.rs`, Dialog: `downloads_dialog.rs`, Automation: `downloads_automation.rs`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use lfx_api::BaseItem;
use lfx_core::rows::RowKind;
use lfx_offline::{
    DownloadManager, DownloadRecord, DownloadState, ManagerSetup, OfflineEvent, OfflineLayout,
    OfflineLibrary, OfflineOutbox, OfflineProgress, OutboxSink, OutboxSyncer, SqliteDownloadStore,
    SqliteOutbox, SystemClock,
};
use lfx_player::ReportSink;
use lfx_session::SessionEvent;
use lfx_ui::{DlState, ToastKind};
use slint::ComponentHandle;

use crate::app::{App, post};
use crate::home::{HomeData, LoadedRow};

/// Einstellungsschlüssel (lokal, je Konto).
pub const KEY_AUTO_DELETE: &str = "dl.auto_delete";
pub const KEY_QUALITY: &str = "dl.quality";
pub const KEY_PARALLEL: &str = "dl.parallel";
pub const KEY_ROOT: &str = "dl.root";

/// Zustand des Offline-Modus (nur UI-Thread).
#[derive(Default)]
pub struct DlCtl {
    pub mgr: Option<DownloadManager>,
    pub lib: Option<OfflineLibrary>,
    pub outbox: Option<Arc<dyn OfflineOutbox>>,
    pub ev_task: Option<tokio::task::AbortHandle>,
    /// Server nicht erreichbar.
    pub offline: bool,
    pub sync_busy: bool,
    /// Bilder fertiger Downloads: Titel-Id -> (Bildart -> Datei).
    pub images: HashMap<String, HashMap<String, PathBuf>>,
    pub page: crate::downloads_page::PageState,
    pub dialog: Option<crate::downloads_dialog::Dialog>,
    pub refresh_pending: bool,
    /// Zähler der abgeschlossenen Abgleiche (Tests).
    pub syncs_done: u64,
    pub last_sync: String,
    /// Laufender Fortschritt je Download-Id (der Datensatz wird nur gelegentlich geschrieben).
    pub progress: HashMap<String, (u64, Option<u64>)>,
}

/// Wurzelordner der Offline-Dateien eines Kontos (ohne Kontoordner).
fn root_of(app: &App) -> OfflineLayout {
    let custom = app
        .local_get(KEY_ROOT)
        .and_then(|v| v.as_str().map(PathBuf::from))
        .filter(|p| !p.as_os_str().is_empty());
    match custom {
        Some(p) => OfflineLayout::new(p),
        None => OfflineLayout::from_dirs(&app.dirs),
    }
}

impl App {
    // ---------------------------------------------------------------- Lebenslauf

    /// Startet den Wächter für Sitzungsereignisse (einmal, vor dem ersten Heartbeat).
    pub fn dl_start(self: &Rc<Self>) {
        let mut rx = self.session.subscribe();
        self.rt.spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(SessionEvent::Connectivity(on)) => post(move |a| a.dl_set_online(on)),
                    Ok(SessionEvent::SessionChanged(None) | SessionEvent::AuthExpired(_)) => {
                        post(|a| a.dl_session_end());
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(_) => return,
                }
            }
        });
    }

    /// Legt den Manager des angemeldeten Kontos an und nimmt unterbrochene Downloads wieder auf.
    pub fn dl_session_ready(self: &Rc<Self>, server_id: &str, user_id: &str) {
        self.dl_session_end();
        let Some(active) = self.session.active() else {
            return;
        };
        let _guard = self.rt.enter();
        let store = Arc::new(SqliteDownloadStore::new(self.store.clone()));
        let mut setup = ManagerSetup::new(
            active.client.clone(),
            server_id,
            user_id,
            root_of(self),
            store,
        );
        if let Some(n) = self.local_get(KEY_PARALLEL).and_then(|v| v.as_u64()) {
            setup.config.max_parallel = (n as usize).clamp(1, 4);
        }
        if let Some(bytes) = std::env::var("LFX_DL_RESERVE")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
        {
            setup.config.min_free_bytes = bytes;
        }
        let mgr = match DownloadManager::new(setup) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!("Download-Manager nicht startbar: {e}");
                return;
            }
        };
        let lib = mgr.library();
        lib.set_auto_delete(
            self.local_get(KEY_AUTO_DELETE)
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        );
        if let Err(e) = mgr.restore() {
            tracing::warn!("Downloads nicht wiederaufgenommen: {e}");
        }
        let mut rx = mgr.subscribe();
        let task = self.rt.spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(ev) => post(move |a| a.dl_offline_event(ev)),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        post(|a| a.dl_schedule_refresh());
                    }
                    Err(_) => return,
                }
            }
        });
        let online = self.session.is_online();
        {
            let mut st = self.st.borrow_mut();
            st.downloads.mgr = Some(mgr);
            st.downloads.lib = Some(lib);
            st.downloads.outbox = Some(Arc::new(SqliteOutbox::new(self.store.clone())));
            st.downloads.ev_task = Some(task.abort_handle());
        }
        self.dl_reload_images();
        self.dl_apply_online(online);
    }

    /// Beendet den Manager (Abmeldung, Konto weg): laufende Downloads werden angehalten.
    pub fn dl_session_end(self: &Rc<Self>) {
        let (mgr, task) = {
            let mut st = self.st.borrow_mut();
            st.downloads.lib = None;
            st.downloads.outbox = None;
            st.downloads.images.clear();
            st.downloads.dialog = None;
            (st.downloads.mgr.take(), st.downloads.ev_task.take())
        };
        if let Some(t) = task {
            t.abort();
        }
        if let Some(m) = mgr {
            let _ = m.pause_all();
        }
        let ui = self.win();
        let g = ui.global::<DlState>();
        g.set_dlg_open(false);
        g.set_offline(false);
    }

    /// Löscht die Offline-Ordner eines Servers (`user = None`) oder Benutzers. Vor dem Entfernen des Kontos
    /// aufrufen: Manager und Wiedergabe halten sonst Dateien offen, und der Speicherort steckt in den
    /// Einstellungen des Kontos.
    pub fn dl_purge(self: &Rc<Self>, server_id: &str, user: Option<&str>) {
        let layout = root_of(self);
        let active_here = self
            .session
            .active()
            .is_some_and(|a| a.server_id == server_id && user.is_none_or(|u| a.user_id == u));
        if active_here {
            self.stop_playback();
            self.dl_session_end();
        }
        let Ok(server) = lfx_offline::safe_segment(server_id) else {
            return;
        };
        let mut dir = layout.root().join(server);
        if let Some(u) = user {
            let Ok(u) = lfx_offline::safe_segment(u) else {
                return;
            };
            dir = dir.join(u);
        }
        if dir.exists()
            && let Err(e) = std::fs::remove_dir_all(&dir)
        {
            tracing::warn!("Offline-Ordner nicht gelöscht: {e}");
        }
        // Standard-Ordner zusätzlich (falls der Speicherort zwischendurch geändert wurde).
        let std_dir = OfflineLayout::from_dirs(&self.dirs);
        if std_dir.root() != layout.root()
            && let Ok(server) = lfx_offline::safe_segment(server_id)
        {
            let mut d = std_dir.root().join(server);
            if let Some(u) = user
                && let Ok(u) = lfx_offline::safe_segment(u)
            {
                d = d.join(u);
            }
            let _ = std::fs::remove_dir_all(d);
        }
    }

    // ---------------------------------------------------------------- Erreichbarkeit

    pub fn dl_offline(&self) -> bool {
        self.st.borrow().downloads.offline
    }

    fn dl_set_online(self: &Rc<Self>, online: bool) {
        self.dl_apply_online(online);
    }

    fn dl_apply_online(self: &Rc<Self>, online: bool) {
        let changed = {
            let mut st = self.st.borrow_mut();
            let was = st.downloads.offline;
            st.downloads.offline = !online;
            was == online
        };
        let ui = self.win();
        let g = ui.global::<DlState>();
        g.set_offline(!online);
        g.set_offline_text(
            "Offline – nur heruntergeladene Titel. Klick öffnet die Downloads.".into(),
        );
        if !changed {
            return;
        }
        tracing::info!(online, "Erreichbarkeit geändert");
        if online {
            self.toast("Wieder mit dem Server verbunden.", ToastKind::Success);
            self.dl_sync();
        } else {
            self.toast(
                "Keine Verbindung zum Server. Heruntergeladene Titel bleiben verfügbar.",
                ToastKind::Info,
            );
        }
        if matches!(self.st.borrow().router.current(), crate::route::Route::Home) {
            self.load_home();
        }
    }

    /// Prüft die Erreichbarkeit sofort (statt auf den Heartbeat zu warten).
    pub fn dl_check_now(self: &Rc<Self>) {
        let session = self.session.clone();
        self.rt.spawn(async move {
            session.check_connectivity().await;
        });
    }

    /// Gleicht Offline-Fortschritt mit dem Server ab (einzeln; nur online).
    pub fn dl_sync(self: &Rc<Self>) {
        let (lib, outbox, active) = {
            let mut st = self.st.borrow_mut();
            if st.downloads.sync_busy || st.downloads.offline {
                return;
            }
            let (Some(lib), Some(ob)) = (st.downloads.lib.clone(), st.downloads.outbox.clone())
            else {
                return;
            };
            let Some(active) = self.session.active() else {
                return;
            };
            st.downloads.sync_busy = true;
            (lib, ob, active)
        };
        let syncer = OutboxSyncer::new(
            active.client.clone(),
            lib,
            outbox,
            active.server_id.clone(),
            active.user_id.clone(),
        );
        self.rt.spawn(async move {
            let result = syncer.sync().await;
            post(move |a| {
                {
                    let mut st = a.st.borrow_mut();
                    st.downloads.sync_busy = false;
                    st.downloads.syncs_done += 1;
                    st.downloads.last_sync = match &result {
                        Ok(r) => format!(
                            "gesendet {}, Server neuer {}, verworfen {}, Fehler {}",
                            r.pushed, r.kept_server, r.dropped, r.failed
                        ),
                        Err(e) => format!("Fehler: {e}"),
                    };
                }
                if let Ok(r) = &result {
                    tracing::info!(pushed = r.pushed, "Offline-Fortschritt abgeglichen");
                    if r.pushed > 0 {
                        a.toast(
                            "Offline-Fortschritt mit dem Server abgeglichen.",
                            ToastKind::Success,
                        );
                    }
                    if r.auto_deleted > 0 {
                        a.dl_reload_images();
                    }
                }
                a.dl_schedule_refresh();
            });
        });
    }

    /// Nach dem Ende einer lokalen Wiedergabe: abgleichen, falls online.
    pub fn dl_sync_if_online(self: &Rc<Self>) {
        if !self.dl_offline() {
            self.dl_sync();
        }
    }

    // ---------------------------------------------------------------- Ereignisse

    fn dl_offline_event(self: &Rc<Self>, ev: OfflineEvent) {
        if let OfflineEvent::Progress { id, done, total } = &ev {
            self.st
                .borrow_mut()
                .downloads
                .progress
                .insert(id.clone(), (*done, *total));
        }
        if let OfflineEvent::State { id, state } = &ev
            && *state == DownloadState::Completed
        {
            let name = self
                .st
                .borrow()
                .downloads
                .mgr
                .as_ref()
                .and_then(|m| m.get(id).ok().flatten())
                .map(|r| r.name)
                .unwrap_or_default();
            self.dl_reload_images();
            self.toast(&format!("Heruntergeladen: {name}"), ToastKind::Success);
        }
        if matches!(ev, OfflineEvent::Removed { .. }) {
            self.dl_reload_images();
        }
        self.dl_schedule_refresh();
    }

    /// Aktualisiert die Downloads-Seite gebündelt (höchstens alle 200 ms).
    pub fn dl_schedule_refresh(self: &Rc<Self>) {
        {
            let mut st = self.st.borrow_mut();
            if st.downloads.refresh_pending {
                return;
            }
            st.downloads.refresh_pending = true;
        }
        let weak = Rc::downgrade(self);
        slint::Timer::single_shot(std::time::Duration::from_millis(200), move || {
            if let Some(a) = weak.upgrade() {
                a.st.borrow_mut().downloads.refresh_pending = false;
                a.dl_refresh_page();
            }
        });
    }

    // ---------------------------------------------------------------- Bibliothek

    pub fn dl_library(&self) -> Option<OfflineLibrary> {
        self.st.borrow().downloads.lib.clone()
    }

    pub fn dl_manager(&self) -> Option<DownloadManager> {
        self.st.borrow().downloads.mgr.clone()
    }

    /// Liest die Bildpfade der fertigen Downloads neu.
    pub fn dl_reload_images(&self) {
        let Some(lib) = self.dl_library() else {
            return;
        };
        let mut map = HashMap::new();
        for e in lib.entries().unwrap_or_default() {
            map.insert(e.record.item_id.clone(), e.images.clone());
        }
        self.st.borrow_mut().downloads.images = map;
    }

    /// Lokales Bild (nur im Offline-Zustand): Datei zu Titel und Bildart.
    pub fn dl_local_image(&self, item_id: &str, kind: lfx_api::ImageType) -> Option<PathBuf> {
        use lfx_api::ImageType as T;
        let st = self.st.borrow();
        if !st.downloads.offline {
            return None;
        }
        let imgs = st.downloads.images.get(item_id)?;
        let order: &[&str] = match kind {
            T::Backdrop | T::Thumb => &["backdrop", "primary"],
            T::Logo => &["logo"],
            _ => &["primary", "backdrop"],
        };
        order.iter().find_map(|k| imgs.get(*k).cloned())
    }

    /// Titel als Anzeigeobjekt aus der Offline-Bibliothek (mit lokalem Stand).
    pub(crate) fn dl_item_of(
        lib: &OfflineLibrary,
        entry: &lfx_offline::OfflineEntry,
    ) -> Option<BaseItem> {
        let json = lib.item_json(&entry.record.item_id).ok()?;
        let mut item: BaseItem = serde_json::from_value(json).ok()?;
        if item.id.is_empty() {
            item.id.clone_from(&entry.record.item_id);
        }
        if let Some(l) = &entry.record.local {
            let ud = item.user_data.get_or_insert_with(Default::default);
            ud.playback_position_ticks = l.position_ticks;
            ud.played = l.played;
        }
        Some(item)
    }

    /// Startseite aus der Offline-Bibliothek; `None`, wenn nichts heruntergeladen ist.
    pub fn dl_offline_home_data(&self) -> Option<HomeData> {
        let lib = self.dl_library()?;
        let mut entries = lib.entries().ok()?;
        entries.sort_by(|a, b| {
            let k = |e: &lfx_offline::OfflineEntry| {
                (
                    e.record.series_name.clone().unwrap_or_default(),
                    e.record.season_number.unwrap_or(0),
                    e.record.episode_number.unwrap_or(0),
                    e.record.name.clone(),
                )
            };
            k(a).cmp(&k(b))
        });
        let mut resume = Vec::new();
        let mut movies = Vec::new();
        let mut series: Vec<(String, Vec<BaseItem>)> = Vec::new();
        let mut all = Vec::new();
        for e in &entries {
            let Some(item) = Self::dl_item_of(&lib, e) else {
                continue;
            };
            let in_progress = e
                .record
                .local
                .as_ref()
                .is_some_and(|l| l.position_ticks > 0 && !l.played);
            if in_progress {
                resume.push(item.clone());
            }
            match e.record.series_name.clone() {
                Some(s) if !s.is_empty() => match series.iter_mut().find(|(n, _)| *n == s) {
                    Some((_, v)) => v.push(item.clone()),
                    None => series.push((s, vec![item.clone()])),
                },
                _ => movies.push(item.clone()),
            }
            all.push(item);
        }
        if all.is_empty() {
            return None;
        }
        let row = |id: &str, title: &str, kind: RowKind, items: Vec<BaseItem>| LoadedRow {
            id: id.to_owned(),
            title: title.to_owned(),
            kind,
            items,
        };
        let mut rows = Vec::new();
        if !resume.is_empty() {
            rows.push(row("dl:resume", "Weiterschauen", RowKind::Resume, resume));
        }
        if !movies.is_empty() {
            rows.push(row(
                "dl:movies",
                "Heruntergeladene Filme",
                RowKind::Landscape,
                movies,
            ));
        }
        for (name, items) in series {
            rows.push(row(
                &format!("dl:series:{name}"),
                &name,
                RowKind::Landscape,
                items,
            ));
        }
        let pool = all.iter().take(5).cloned().collect();
        Some(HomeData {
            rows,
            pool,
            trend: Default::default(),
        })
    }

    /// Karte im Offline-Zustand: lokal abspielen statt das Detailfenster zu laden. `true` = behandelt.
    pub fn dl_card_offline(self: &Rc<Self>, id: &str) -> bool {
        if !self.dl_offline() {
            return false;
        }
        let Some(lib) = self.dl_library() else {
            return false;
        };
        let Ok(Some(entry)) = lib.entry(id) else {
            self.toast(
                "Offline nicht verfügbar – dieser Titel ist nicht heruntergeladen.",
                ToastKind::Info,
            );
            return true;
        };
        match Self::dl_item_of(&lib, &entry) {
            Some(item) => self.play_item(&item),
            None => self.toast("Der Titel lässt sich nicht öffnen.", ToastKind::Error),
        }
        true
    }

    // ---------------------------------------------------------------- Wiedergabe

    /// Soll der Titel aus der Offline-Bibliothek gespielt werden? (Ja, sobald eine fertige Datei da ist.)
    pub fn dl_use_local(&self, item_id: &str) -> bool {
        self.dl_library()
            .is_some_and(|l| l.playback(item_id).is_ok())
    }

    /// Wiedergabeauftrag für eine lokale Datei.
    pub fn dl_local_request(&self, item_id: &str) -> Option<lfx_player::PlayRequest> {
        let lib = self.dl_library()?;
        let pb = lib.playback(item_id).ok()?;
        Some(pb.to_play_request())
    }

    /// Fortschritts-Senke für lokale Wiedergabe (Outbox statt Server); `None` für Netzwiedergabe.
    pub fn dl_report_sink(&self, req: &lfx_player::PlayRequest) -> Option<Arc<dyn ReportSink>> {
        if !Path::new(&req.stream_url).is_file() {
            return None;
        }
        let (lib, outbox) = {
            let st = self.st.borrow();
            (st.downloads.lib.clone()?, st.downloads.outbox.clone()?)
        };
        let a = self.session.active()?;
        let progress = OfflineProgress::new(
            lib,
            outbox,
            Arc::new(SystemClock),
            a.server_id.clone(),
            a.user_id.clone(),
        );
        Some(Arc::new(OutboxSink::new(progress)))
    }

    /// Ob eine Wiedergabe lokal läuft (Dateipfad statt Adresse).
    pub fn dl_is_local_request(req: &lfx_player::PlayRequest) -> bool {
        Path::new(&req.stream_url).is_file()
    }

    /// Einstellung geändert (Schalter in den Einstellungen).
    pub fn dl_setting_changed(self: &Rc<Self>, key: &str) {
        match key {
            KEY_AUTO_DELETE => {
                let on = self
                    .local_get(KEY_AUTO_DELETE)
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if let Some(l) = self.dl_library() {
                    l.set_auto_delete(on);
                }
                self.dl_sync();
            }
            KEY_PARALLEL => {
                if let (Some(m), Some(n)) = (
                    self.dl_manager(),
                    self.local_get(KEY_PARALLEL).and_then(|v| v.as_u64()),
                ) {
                    m.set_max_parallel(n as usize);
                }
            }
            _ => {}
        }
        self.dl_schedule_refresh();
    }

    /// Alle Datensätze (für Seite und Tests).
    pub fn dl_records(&self) -> Vec<DownloadRecord> {
        let mut recs = self
            .dl_manager()
            .and_then(|m| m.list().ok())
            .unwrap_or_default();
        let st = self.st.borrow();
        for r in &mut recs {
            if r.state == DownloadState::Downloading
                && let Some((done, total)) = st.downloads.progress.get(&r.id)
            {
                r.bytes_done = r.bytes_done.max(*done);
                r.bytes_total = r.bytes_total.or(*total);
            }
        }
        recs
    }

    /// Speicherort ändern (nur ohne vorhandene Downloads; leer = Standard).
    pub fn dl_set_root(self: &Rc<Self>, raw: &str) {
        if !self.dl_records().is_empty() {
            self.toast(
                "Der Speicherort lässt sich nur ändern, solange nichts heruntergeladen ist.",
                ToastKind::Error,
            );
            return;
        }
        if raw.is_empty() {
            self.save_setting(KEY_ROOT, serde_json::json!(""));
        } else {
            let p = Path::new(raw);
            if !p.is_absolute() {
                self.toast(
                    "Bitte einen absoluten Ordnerpfad angeben.",
                    ToastKind::Error,
                );
                return;
            }
            if let Err(e) = std::fs::create_dir_all(p) {
                tracing::warn!("Speicherort nicht anlegbar: {e}");
                self.toast("Der Ordner lässt sich nicht anlegen.", ToastKind::Error);
                return;
            }
            self.save_setting(KEY_ROOT, serde_json::json!(raw));
        }
        if let Some(a) = self.session.active() {
            self.dl_session_ready(&a.server_id, &a.user_id);
        }
        self.toast("Speicherort übernommen.", ToastKind::Success);
    }
}
