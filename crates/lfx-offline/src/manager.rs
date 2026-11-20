//! Download-Manager: Warteschlange mit begrenzter Parallelität, Pause/Fortsetzen/Abbrechen, Wiederaufnahme
//! nach Neustart, Platzprüfung und Zubehör (Bilder, Untertitel, Element-JSON).
//!
//! Alle Zustandswechsel eines Downloads laufen über den Planer (`Sched`): ein Download hat höchstens einen
//! Task; Pause und Fortsetzen werden unter derselben Sperre entschieden, in der der Task sich beendet.
//! Die Methoden `enqueue`, `resume`, `restore` usw. brauchen einen laufenden Tokio-Runtime-Kontext.

use crate::error::{OfflineError, Result};
use crate::fetch::{self, Control, Outcome, Transfer};
use crate::layout::{OfflineLayout, SpaceProbe, SystemSpace, safe_extension, safe_segment};
use crate::library::OfflineLibrary;
use crate::plan::{self, Target};
use crate::store::DownloadStore;
use crate::time::{Clock, SystemClock};
use crate::types::{DownloadRecord, DownloadState, OfflineEvent, Quality, StoredFile};
use lfx_api::{Client, ImageOptions};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, watch};
use tokio::task::JoinHandle;

/// Einstellungen des Managers.
#[derive(Debug, Clone)]
pub struct ManagerConfig {
    /// Gleichzeitige Downloads (mindestens 1).
    pub max_parallel: usize,
    /// Reserve, die nach einem Download frei bleiben muss (Bytes).
    pub min_free_bytes: u64,
    /// Automatische Wiederholungen bei Netzfehlern.
    pub max_retries: u32,
    /// Wartezeit vor der ersten Wiederholung (verdoppelt sich).
    pub retry_delay: Duration,
    /// Zeitlimit ohne neue Daten.
    pub idle_timeout: Duration,
    /// Zeitlimit für den Verbindungsaufbau.
    pub connect_timeout: Duration,
    /// Mindestabstand der Fortschrittsereignisse.
    pub progress_interval: Duration,
}

impl Default for ManagerConfig {
    fn default() -> Self {
        Self {
            max_parallel: 2,
            min_free_bytes: 512 * 1024 * 1024,
            max_retries: 3,
            retry_delay: Duration::from_secs(2),
            idle_timeout: Duration::from_secs(30),
            connect_timeout: Duration::from_secs(10),
            progress_interval: Duration::from_millis(250),
        }
    }
}

/// Alles, was der Manager zum Start braucht.
pub struct ManagerSetup {
    /// Angemeldeter API-Client (Sitzung gesetzt).
    pub client: Client,
    /// Server des Kontos.
    pub server_id: String,
    /// Benutzer des Kontos.
    pub user_id: String,
    /// Speicherort.
    pub layout: OfflineLayout,
    /// Ablage der Datensätze.
    pub store: Arc<dyn DownloadStore>,
    /// Abfrage des freien Platzes.
    pub space: Arc<dyn SpaceProbe>,
    /// Uhr.
    pub clock: Arc<dyn Clock>,
    /// Einstellungen.
    pub config: ManagerConfig,
}

impl ManagerSetup {
    /// Standardwerte: echte Platzabfrage, Systemuhr, Standardeinstellungen.
    pub fn new(
        client: Client,
        server_id: impl Into<String>,
        user_id: impl Into<String>,
        layout: OfflineLayout,
        store: Arc<dyn DownloadStore>,
    ) -> Self {
        Self {
            client,
            server_id: server_id.into(),
            user_id: user_id.into(),
            layout,
            store,
            space: Arc::new(SystemSpace),
            clock: Arc::new(SystemClock),
            config: ManagerConfig::default(),
        }
    }
}

/// Optionen beim Einreihen.
#[derive(Debug, Clone)]
pub struct EnqueueOptions {
    /// Qualität.
    pub quality: Quality,
    /// Bei Serien/Staffeln gesehene Folgen auslassen.
    pub only_unplayed: bool,
    /// Erwartete SHA-256 (nur für Einzeltitel; wird beim Abschluss geprüft).
    pub expected_sha256: Option<String>,
}

impl EnqueueOptions {
    /// Originaldatei, alles laden.
    pub fn original() -> Self {
        Self {
            quality: Quality::Original,
            only_unplayed: false,
            expected_sha256: None,
        }
    }

    /// Umgewandelt mit dem gegebenen Profil.
    pub fn transcoded(profile: crate::types::TranscodeProfile) -> Self {
        Self {
            quality: Quality::Transcoded(profile),
            only_unplayed: false,
            expected_sha256: None,
        }
    }
}

struct Active {
    ctl: watch::Sender<Control>,
    handle: Option<JoinHandle<()>>,
}

struct Sched {
    active: HashMap<String, Active>,
    max_parallel: usize,
}

pub(crate) struct Inner {
    pub(crate) client: Client,
    pub(crate) server_id: String,
    pub(crate) user_id: String,
    pub(crate) layout: OfflineLayout,
    pub(crate) store: Arc<dyn DownloadStore>,
    space: Arc<dyn SpaceProbe>,
    pub(crate) clock: Arc<dyn Clock>,
    cfg: ManagerConfig,
    http: reqwest::Client,
    events: broadcast::Sender<OfflineEvent>,
    sched: Mutex<Sched>,
    session_counter: AtomicU64,
    auto_delete: Arc<std::sync::atomic::AtomicBool>,
}

/// Der Download-Manager eines Kontos. Klonen ist billig.
#[derive(Clone)]
pub struct DownloadManager {
    pub(crate) inner: Arc<Inner>,
}

impl std::fmt::Debug for DownloadManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DownloadManager")
    }
}

/// Endergebnis eines Übertragungsversuchs.
enum Final {
    Done,
    Paused,
    Canceled,
}

/// Wartet `dur` oder bis Pause/Abbruch kommt.
async fn sleep_or_control(rx: &mut watch::Receiver<Control>, dur: Duration) -> Option<Control> {
    let now = *rx.borrow();
    if now != Control::Run {
        return Some(now);
    }
    tokio::select! {
        biased;
        changed = rx.changed() => {
            let state = if changed.is_ok() { *rx.borrow() } else { Control::Cancel };
            (state != Control::Run).then_some(state)
        }
        () = tokio::time::sleep(dur) => None,
    }
}

impl DownloadManager {
    /// Neuer Manager. Startet noch nichts; [`DownloadManager::restore`] nimmt Offenes nach einem Neustart wieder auf.
    pub fn new(setup: ManagerSetup) -> Result<Self> {
        let http = reqwest::Client::builder()
            .connect_timeout(setup.config.connect_timeout)
            .user_agent(format!("LENZFLIX/{}", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| OfflineError::Network(e.without_url().to_string()))?;
        let (events, _) = broadcast::channel(512);
        let max_parallel = setup.config.max_parallel.max(1);
        Ok(Self {
            inner: Arc::new(Inner {
                client: setup.client,
                server_id: setup.server_id,
                user_id: setup.user_id,
                layout: setup.layout,
                store: setup.store,
                space: setup.space,
                clock: setup.clock,
                cfg: setup.config,
                http,
                events,
                sched: Mutex::new(Sched {
                    active: HashMap::new(),
                    max_parallel,
                }),
                session_counter: AtomicU64::new(0),
                auto_delete: Arc::default(),
            }),
        })
    }

    /// Ereignisse (Zustand, Fortschritt).
    pub fn subscribe(&self) -> broadcast::Receiver<OfflineEvent> {
        self.inner.events.subscribe()
    }

    /// Offline-Bibliothek über denselben Speicher.
    pub fn library(&self) -> OfflineLibrary {
        OfflineLibrary::new(
            self.inner.store.clone(),
            self.inner.layout.clone(),
            self.inner.server_id.clone(),
            self.inner.user_id.clone(),
        )
        .with_auto_delete_flag(self.inner.auto_delete.clone())
    }

    /// Alle Downloads des Kontos in Anlegereihenfolge.
    pub fn list(&self) -> Result<Vec<DownloadRecord>> {
        self.inner
            .store
            .list(&self.inner.server_id, &self.inner.user_id)
    }

    /// Ein Download.
    pub fn get(&self, id: &str) -> Result<Option<DownloadRecord>> {
        self.inner.store.get(id)
    }

    /// Kennung eines Downloads dieses Kontos.
    pub fn download_id(&self, item_id: &str) -> Result<String> {
        Ok(format!(
            "{}.{}.{}",
            safe_segment(&self.inner.server_id)?,
            safe_segment(&self.inner.user_id)?,
            safe_segment(item_id)?
        ))
    }

    /// Ändert die Zahl gleichzeitiger Downloads.
    pub fn set_max_parallel(&self, n: usize) {
        self.lock().max_parallel = n.max(1);
        self.pump();
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Sched> {
        self.inner.sched.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn emit(&self, event: OfflineEvent) {
        let _ = self.inner.events.send(event);
    }

    fn save(&self, rec: &DownloadRecord) {
        if let Err(e) = self.inner.store.put(rec) {
            tracing::warn!(error = %e, "Download-Datensatz nicht gespeichert");
        }
    }

    fn set_state(&self, rec: &mut DownloadRecord, state: DownloadState) {
        rec.state = state;
        self.save(rec);
        self.emit(OfflineEvent::State {
            id: rec.id.clone(),
            state,
        });
    }

    // ------------------------------------------------------------------ Einreihen

    /// Reiht einen Titel, eine Staffel oder eine Serie ein. Gibt die Kennungen aller betroffenen Downloads
    /// zurück (bereits vorhandene eingeschlossen). Prüft vorher Recht und Platz; bei Fehlern wird nichts angelegt.
    pub async fn enqueue(&self, item_id: &str, options: EnqueueOptions) -> Result<Vec<String>> {
        safe_segment(item_id)?;
        let inner = &self.inner;
        if options.quality == Quality::Original {
            let me = inner.client.me().await?;
            if !me
                .policy
                .as_ref()
                .is_some_and(|p| p.enable_content_downloading)
            {
                return Err(OfflineError::NotAllowed);
            }
        }
        let targets = plan::resolve(&inner.client, item_id, options.only_unplayed).await?;
        let single = targets.len() == 1;
        let mut fresh: Vec<(DownloadRecord, Target)> = Vec::new();
        let mut ids = Vec::new();
        let mut blocked = 0usize;
        for target in targets {
            let rec = self.new_record(
                &target,
                &options,
                single && options.expected_sha256.is_some(),
                item_id,
            )?;
            if target.item.get("CanDownload").and_then(Value::as_bool) == Some(false)
                && options.quality == Quality::Original
            {
                blocked += 1;
                continue;
            }
            ids.push(rec.id.clone());
            match inner.store.get(&rec.id)? {
                Some(mut old) => {
                    if old.state == DownloadState::Failed {
                        old.error = None;
                        self.set_state(&mut old, DownloadState::Queued);
                    }
                }
                None => fresh.push((rec, target)),
            }
        }
        if ids.is_empty() && blocked > 0 {
            return Err(OfflineError::NotAllowed);
        }
        // Platz für alle neuen Downloads zusammen prüfen, bevor etwas angelegt wird.
        let needed: u64 = fresh.iter().map(|(r, _)| r.size_estimate).sum();
        if !fresh.is_empty() {
            let dir = inner.layout.root().join(safe_segment(&inner.server_id)?);
            if let Some(avail) = inner.space.available(&dir)
                && avail < needed.saturating_add(inner.cfg.min_free_bytes)
            {
                return Err(OfflineError::InsufficientSpace {
                    needed: needed.saturating_add(inner.cfg.min_free_bytes),
                    available: avail,
                });
            }
        }
        for (rec, target) in fresh {
            self.write_metadata(&rec, &target).await?;
            self.save(&rec);
            self.emit(OfflineEvent::Added { id: rec.id.clone() });
        }
        self.pump();
        Ok(ids)
    }

    fn new_record(
        &self,
        target: &Target,
        options: &EnqueueOptions,
        with_hash: bool,
        requested: &str,
    ) -> Result<DownloadRecord> {
        let inner = &self.inner;
        let item = &target.item;
        let item_id = plan::jstr(item, "Id")
            .ok_or_else(|| OfflineError::Unsupported("Element ohne Kennung".into()))?;
        let source = plan::pick_source(item);
        let source_id = source
            .and_then(|s| plan::jstr(s, "Id"))
            .unwrap_or(item_id)
            .to_owned();
        let container = source
            .and_then(|s| plan::jstr(s, "Container"))
            .or_else(|| plan::jstr(item, "Container"))
            .unwrap_or("");
        let extension = match options.quality {
            Quality::Original => safe_extension(container, "bin"),
            Quality::Transcoded(_) => "mp4".to_owned(),
        };
        let kind = plan::jstr(item, "Type").unwrap_or("Video").to_owned();
        Ok(DownloadRecord {
            id: self.download_id(item_id)?,
            server_id: inner.server_id.clone(),
            user_id: inner.user_id.clone(),
            item_id: item_id.to_owned(),
            kind,
            name: plan::jstr(item, "Name").unwrap_or("").to_owned(),
            series_id: plan::jstr(item, "SeriesId").map(str::to_owned),
            series_name: plan::jstr(item, "SeriesName").map(str::to_owned),
            season_id: plan::jstr(item, "SeasonId").map(str::to_owned),
            season_number: plan::jint(item, "ParentIndexNumber").map(|v| v as i32),
            episode_number: plan::jint(item, "IndexNumber").map(|v| v as i32),
            year: plan::jint(item, "ProductionYear").map(|v| v as i32),
            runtime_ticks: plan::jint(item, "RunTimeTicks"),
            quality: options.quality.clone(),
            state: DownloadState::Queued,
            media_source_id: source_id,
            extension,
            bytes_total: None,
            size_estimate: plan::estimate_size(item, &options.quality),
            bytes_done: 0,
            sha256: None,
            expected_sha256: if with_hash {
                options.expected_sha256.clone()
            } else {
                None
            },
            files: Vec::new(),
            accessory_errors: Vec::new(),
            error: None,
            play_session_id: None,
            local: None,
            group: (item_id != requested).then(|| requested.to_owned()),
            created_ms: inner.clock.now_millis(),
        })
    }

    /// Legt den Elementordner an und schreibt Element-, Serien- und Staffel-JSON (für die Offline-Anzeige).
    /// Die Dateiliste im Datensatz füllt `fetch_accessories` nach dem Download.
    async fn write_metadata(&self, rec: &DownloadRecord, target: &Target) -> Result<()> {
        let dir = self
            .inner
            .layout
            .item_dir(&rec.server_id, &rec.user_id, &rec.item_id)?;
        tokio::fs::create_dir_all(&dir).await?;
        for (name, value) in [
            ("item", Some(&target.item)),
            ("series", target.series.as_ref()),
            ("season", target.season.as_ref()),
        ] {
            if let Some(v) = value {
                tokio::fs::write(dir.join(format!("{name}.json")), serde_json::to_vec(v)?).await?;
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------------ Steuerung

    /// Nimmt nach einem Neustart auf: war etwas „Läuft“, wartet es wieder; danach wird die Warteschlange abgearbeitet.
    pub fn restore(&self) -> Result<()> {
        for mut rec in self.list()? {
            if rec.state == DownloadState::Downloading && !self.lock().active.contains_key(&rec.id)
            {
                self.set_state(&mut rec, DownloadState::Queued);
            }
        }
        self.pump();
        Ok(())
    }

    /// Hält einen Download an (Datei bleibt, Fortsetzen macht dort weiter, sofern der Server `Range` kann).
    pub fn pause(&self, id: &str) -> Result<()> {
        let s = self.lock();
        if let Some(a) = s.active.get(id) {
            let _ = a.ctl.send(Control::Pause);
            return Ok(());
        }
        let mut rec = self
            .inner
            .store
            .get(id)?
            .ok_or(OfflineError::UnknownDownload)?;
        if rec.state == DownloadState::Queued {
            self.set_state(&mut rec, DownloadState::Paused);
        }
        Ok(())
    }

    /// Setzt einen angehaltenen oder fehlgeschlagenen Download fort.
    pub fn resume(&self, id: &str) -> Result<()> {
        {
            let s = self.lock();
            if let Some(a) = s.active.get(id) {
                let _ = a.ctl.send(Control::Run);
                return Ok(());
            }
            let mut rec = self
                .inner
                .store
                .get(id)?
                .ok_or(OfflineError::UnknownDownload)?;
            match rec.state {
                DownloadState::Paused | DownloadState::Failed => {
                    rec.error = None;
                    self.set_state(&mut rec, DownloadState::Queued);
                }
                _ => return Ok(()),
            }
        }
        self.pump();
        Ok(())
    }

    /// Hält alle laufenden und wartenden Downloads an.
    pub fn pause_all(&self) -> Result<()> {
        for rec in self.list()? {
            if matches!(
                rec.state,
                DownloadState::Queued | DownloadState::Downloading
            ) {
                self.pause(&rec.id)?;
            }
        }
        Ok(())
    }

    /// Setzt alle angehaltenen Downloads fort.
    pub fn resume_all(&self) -> Result<()> {
        for rec in self.list()? {
            if rec.state == DownloadState::Paused {
                self.resume(&rec.id)?;
            }
        }
        Ok(())
    }

    /// Bricht einen Download ab und löscht Datensatz und Dateien (auch bei fertigen Downloads).
    pub async fn cancel(&self, id: &str) -> Result<()> {
        let taken = self.lock().active.remove(id);
        if let Some(mut a) = taken {
            let _ = a.ctl.send(Control::Cancel);
            if let Some(h) = a.handle.take() {
                let _ = h.await;
            }
        }
        let Some(rec) = self.inner.store.get(id)? else {
            return Err(OfflineError::UnknownDownload);
        };
        let dir = self
            .inner
            .layout
            .item_dir(&rec.server_id, &rec.user_id, &rec.item_id)?;
        if let Err(e) = tokio::fs::remove_dir_all(&dir).await
            && e.kind() != std::io::ErrorKind::NotFound
        {
            return Err(e.into());
        }
        self.inner.store.delete(id)?;
        self.emit(OfflineEvent::Removed { id: id.to_owned() });
        self.pump();
        Ok(())
    }

    /// Startet wartende Downloads bis zur Parallelitätsgrenze.
    pub(crate) fn pump(&self) {
        let mut s = self.lock();
        let Ok(list) = self.list() else { return };
        for rec in list
            .into_iter()
            .filter(|r| r.state == DownloadState::Queued)
        {
            if s.active.len() >= s.max_parallel {
                break;
            }
            if s.active.contains_key(&rec.id) {
                continue;
            }
            let (tx, rx) = watch::channel(Control::Run);
            let this = self.clone();
            let id = rec.id.clone();
            let handle = tokio::spawn(async move { this.run_task(id, rx).await });
            s.active.insert(
                rec.id,
                Active {
                    ctl: tx,
                    handle: Some(handle),
                },
            );
        }
    }

    // ------------------------------------------------------------------ Task

    async fn run_task(self, id: String, mut rx: watch::Receiver<Control>) {
        loop {
            let outcome = self.transfer(&id, &mut rx).await;
            let mut s = self.lock();
            match outcome {
                Ok(Final::Paused) => {
                    // Wurde inzwischen fortgesetzt, nicht anhalten, sondern weiterladen.
                    if *rx.borrow() == Control::Run {
                        continue;
                    }
                    if let Ok(Some(mut rec)) = self.inner.store.get(&id) {
                        rec.bytes_done = self.part_len_sync(&rec);
                        self.set_state(&mut rec, DownloadState::Paused);
                    }
                }
                Ok(Final::Done | Final::Canceled) => {}
                Err(e) => {
                    if let Ok(Some(mut rec)) = self.inner.store.get(&id) {
                        tracing::warn!(error = %e, "Download fehlgeschlagen");
                        rec.error = Some(e.to_string());
                        rec.bytes_done = self.part_len_sync(&rec);
                        self.set_state(&mut rec, DownloadState::Failed);
                    }
                }
            }
            s.active.remove(&id);
            drop(s);
            self.pump();
            return;
        }
    }

    fn part_len_sync(&self, rec: &DownloadRecord) -> u64 {
        self.inner
            .layout
            .item_dir(&rec.server_id, &rec.user_id, &rec.item_id)
            .ok()
            .and_then(|d| std::fs::metadata(d.join("media.part")).ok())
            .map_or(0, |m| m.len())
    }

    async fn transfer(&self, id: &str, rx: &mut watch::Receiver<Control>) -> Result<Final> {
        let inner = &self.inner;
        let Some(mut rec) = inner.store.get(id)? else {
            return Ok(Final::Canceled);
        };
        self.set_state(&mut rec, DownloadState::Downloading);
        let dir = inner
            .layout
            .item_dir(&rec.server_id, &rec.user_id, &rec.item_id)?;
        let part = dir.join("media.part");
        let (url, resumable) = match &rec.quality {
            Quality::Original => (inner.client.download_url(&rec.item_id)?, true),
            Quality::Transcoded(profile) => {
                let session = rec.play_session_id.clone().unwrap_or_else(|| {
                    format!(
                        "lfxdl-{}-{}",
                        inner.clock.now_millis(),
                        inner.session_counter.fetch_add(1, Ordering::SeqCst)
                    )
                });
                rec.play_session_id = Some(session.clone());
                self.save(&rec);
                (
                    plan::transcode_url(
                        &inner.client,
                        &rec.item_id,
                        &rec.media_source_id,
                        profile,
                        &session,
                    )?,
                    false,
                )
            }
        };

        let mut attempt = 0u32;
        let (size, sha) = loop {
            let auth = inner.client.auth_header_value();
            let mut seen_total = rec.bytes_total;
            let mut last_emit = Instant::now();
            let progress_id = rec.id.clone();
            let events = inner.events.clone();
            let interval = inner.cfg.progress_interval;
            let space = inner.space.clone();
            let min_free = inner.cfg.min_free_bytes;
            let estimate = rec.size_estimate;
            let dir_for_space = dir.clone();
            let mut on_start = |total: Option<u64>, start: u64| -> Result<()> {
                seen_total = total;
                let need = total.unwrap_or(estimate).saturating_sub(start);
                if let Some(avail) = space.available(&dir_for_space)
                    && avail < need.saturating_add(min_free)
                {
                    return Err(OfflineError::InsufficientSpace {
                        needed: need.saturating_add(min_free),
                        available: avail,
                    });
                }
                Ok(())
            };
            let mut on_progress = |done: u64, total: Option<u64>| {
                if last_emit.elapsed() >= interval {
                    last_emit = Instant::now();
                    let _ = events.send(OfflineEvent::Progress {
                        id: progress_id.clone(),
                        done,
                        total,
                    });
                }
            };
            let result = fetch::run(
                Transfer {
                    http: &inner.http,
                    url: &url,
                    auth: &auth,
                    part: &part,
                    allow_resume: resumable,
                    idle: inner.cfg.idle_timeout,
                },
                rx,
                &mut on_start,
                &mut on_progress,
            )
            .await;
            rec.bytes_total = seen_total;
            match result {
                Ok(Outcome::Done { size, sha256 }) => break (size, sha256),
                Ok(Outcome::Paused) => return Ok(Final::Paused),
                Ok(Outcome::Canceled) => return Ok(Final::Canceled),
                Err(e) if e.is_transient() && attempt < inner.cfg.max_retries => {
                    let wait = inner.cfg.retry_delay * 2u32.saturating_pow(attempt);
                    attempt += 1;
                    match sleep_or_control(rx, wait).await {
                        Some(Control::Pause) => return Ok(Final::Paused),
                        Some(Control::Cancel) => return Ok(Final::Canceled),
                        _ => {}
                    }
                }
                Err(e) => {
                    if matches!(
                        e,
                        OfflineError::SizeMismatch { .. } | OfflineError::ChecksumMismatch
                    ) {
                        tokio::fs::remove_file(&part).await.ok();
                    }
                    return Err(e);
                }
            }
        };

        if let Some(expected) = &rec.expected_sha256
            && !expected.eq_ignore_ascii_case(&sha)
        {
            tokio::fs::remove_file(&part).await.ok();
            return Err(OfflineError::ChecksumMismatch);
        }
        let final_path = dir.join(format!("media.{}", rec.extension));
        tokio::fs::remove_file(&final_path).await.ok();
        tokio::fs::rename(&part, &final_path).await?;
        rec.bytes_total = Some(size);
        rec.bytes_done = size;
        rec.sha256 = Some(sha);
        self.save(&rec);
        // Antwort ist längst verworfen; jetzt darf der Serverjob der Umwandlung enden.
        if let (Quality::Transcoded(_), Some(session)) = (&rec.quality, &rec.play_session_id)
            && let Err(e) = inner.client.delete_active_encodings(session).await
        {
            tracing::debug!(error = %e, "Umwandlungsjob nicht beendet");
        }
        self.fetch_accessories(&mut rec, &dir).await;
        rec.error = None;
        self.set_state(&mut rec, DownloadState::Completed);
        self.emit(OfflineEvent::Progress {
            id: rec.id.clone(),
            done: size,
            total: Some(size),
        });
        Ok(Final::Done)
    }

    /// Lädt Bilder und externe Untertitel. Fehler sind nicht fatal und landen in `accessory_errors`.
    async fn fetch_accessories(&self, rec: &mut DownloadRecord, dir: &Path) {
        let inner = &self.inner;
        rec.files
            .retain(|f| !matches!(f.kind.as_str(), "item" | "series" | "season"));
        for (kind, name) in [
            ("item", "item.json"),
            ("series", "series.json"),
            ("season", "season.json"),
        ] {
            if dir.join(name).is_file() {
                rec.files.push(plain(kind, name));
            }
        }
        let Ok(raw) = tokio::fs::read(dir.join("item.json")).await else {
            return;
        };
        let Ok(item) = serde_json::from_slice::<Value>(&raw) else {
            return;
        };
        rec.accessory_errors.clear();
        for img in plan::image_plans(&item) {
            let rel = format!("{}.img", img.kind);
            if rec.files.iter().any(|f| f.rel == rel) {
                continue;
            }
            let max_width = match img.kind {
                "backdrop" => 1920,
                "logo" => 800,
                _ => 600,
            };
            let options = ImageOptions {
                max_width: Some(max_width),
                max_height: None,
                quality: Some(90),
            };
            let result: Result<()> = async {
                let url = inner.client.image_url(
                    &img.owner_id,
                    img.api_kind,
                    img.tag.as_deref(),
                    options,
                )?;
                let bytes = inner.client.fetch_bytes(&url).await?;
                tokio::fs::write(dir.join(&rel), &bytes).await?;
                Ok(())
            }
            .await;
            match result {
                Ok(()) => rec.files.push(plain(img.kind, &rel)),
                Err(e) => rec.accessory_errors.push(format!("Bild {}: {e}", img.kind)),
            }
        }
        if let Some(source) = plan::pick_source(&item) {
            for sub in plan::subtitle_plans(&rec.item_id, source, &rec.quality) {
                let rel = format!("sub-{}.{}", sub.index, sub.extension);
                if rec.files.iter().any(|f| f.rel == rel) {
                    continue;
                }
                let result: Result<()> = async {
                    let url = inner.client.absolute_url(&sub.url_path)?;
                    let bytes = inner.client.fetch_bytes(&url).await?;
                    tokio::fs::write(dir.join(&rel), &bytes).await?;
                    Ok(())
                }
                .await;
                match result {
                    Ok(()) => rec.files.push(StoredFile {
                        kind: "subtitle".into(),
                        rel,
                        language: sub.language,
                        title: sub.title,
                        codec: sub.codec,
                        index: Some(sub.index),
                        is_default: sub.is_default,
                    }),
                    Err(e) => rec
                        .accessory_errors
                        .push(format!("Untertitel {}: {e}", sub.index)),
                }
            }
        }
    }

    /// Speicherort der Dateien eines Downloads.
    pub fn item_dir(&self, rec: &DownloadRecord) -> Result<PathBuf> {
        self.inner
            .layout
            .item_dir(&rec.server_id, &rec.user_id, &rec.item_id)
    }
}

fn plain(kind: &str, rel: &str) -> StoredFile {
    StoredFile {
        kind: kind.to_owned(),
        rel: rel.to_owned(),
        language: None,
        title: None,
        codec: None,
        index: None,
        is_default: false,
    }
}
