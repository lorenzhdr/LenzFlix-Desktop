//! Der asynchrone Bild-Lader: Speicher-LRU, Plattencache, Deduplizierung, Priorität,
//! Abbruch und begrenztes Vorladen.
//!
//! Ablauf je Anfrage: Speichercache → (Job anlegen oder an laufenden Job anhängen) →
//! Plattencache → Fetcher → Dekodieren/Skalieren (begrenzt parallel) → Caches füllen.
//! `request` darf von jedem Thread aus aufgerufen werden (auch vom UI-Thread).

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::num::NonZeroUsize;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll};

use lru::LruCache;
use tokio::runtime::Handle;
use tokio::sync::{Semaphore, oneshot};
use tokio::task::AbortHandle;

use crate::decode::decode_image;
use crate::disk::DiskCache;
use crate::fetcher::ImageFetcher;
use crate::types::{DecodedImage, ImageError, ImageRequest, Priority};

/// Ergebnis einer Anfrage.
pub type LoadResult = Result<DecodedImage, ImageError>;

/// Einstellungen des Laders.
#[derive(Debug, Clone)]
pub struct LoaderConfig {
    /// Byte-Budget des Speichercaches (dekodierte Pixel).
    pub memory_budget_bytes: usize,
    /// Gleichzeitig laufende Jobs (Abruf + Dekodieren).
    pub max_concurrent: usize,
    /// Gleichzeitige Dekodier-/Skalier-Aufgaben (begrenzter Pool).
    pub decode_workers: usize,
    /// Höchstzahl wartender Vorlade-Jobs; weitere werden abgelehnt.
    pub max_prefetch_queue: usize,
}

impl Default for LoaderConfig {
    fn default() -> Self {
        let cores = std::thread::available_parallelism().map_or(2, NonZeroUsize::get);
        Self {
            memory_budget_bytes: 96 * 1024 * 1024,
            max_concurrent: 8,
            decode_workers: cores.clamp(1, 4),
            max_prefetch_queue: 64,
        }
    }
}

/// Zähler für Diagnose und Tests.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LoaderStats {
    /// Treffer im Speichercache.
    pub memory_hits: u64,
    /// Treffer im Plattencache.
    pub disk_hits: u64,
    /// Abrufe über den Fetcher.
    pub network_fetches: u64,
    /// Dekodiervorgänge.
    pub decodes: u64,
    /// Anfragen, die an einen laufenden Job angehängt wurden (Deduplizierung).
    pub deduped: u64,
    /// Abgebrochene Jobs.
    pub cancelled: u64,
    /// Fehlgeschlagene Jobs.
    pub failed: u64,
}

#[derive(Default)]
struct Counters {
    memory_hits: AtomicU64,
    disk_hits: AtomicU64,
    network_fetches: AtomicU64,
    decodes: AtomicU64,
    deduped: AtomicU64,
    cancelled: AtomicU64,
    failed: AtomicU64,
}

fn bump(c: &AtomicU64) {
    c.fetch_add(1, Ordering::Relaxed);
}

struct MemCache {
    lru: LruCache<String, DecodedImage>,
    bytes: usize,
    budget: usize,
}

impl MemCache {
    fn new(budget: usize) -> Self {
        Self {
            lru: LruCache::unbounded(),
            bytes: 0,
            budget,
        }
    }

    fn get(&mut self, key: &str) -> Option<DecodedImage> {
        self.lru.get(key).cloned()
    }

    fn insert(&mut self, key: String, img: DecodedImage) {
        if img.byte_len() > self.budget {
            return;
        }
        if let Some(old) = self.lru.put(key, img.clone()) {
            self.bytes -= old.byte_len();
        }
        self.bytes += img.byte_len();
        while self.bytes > self.budget {
            match self.lru.pop_lru() {
                Some((_, old)) => self.bytes -= old.byte_len(),
                None => break,
            }
        }
    }

    fn clear(&mut self) {
        self.lru.clear();
        self.bytes = 0;
    }
}

struct RunState {
    abort: AbortHandle,
    as_prefetch: bool,
}

struct Job {
    serial: u64,
    req: ImageRequest,
    priority: Priority,
    /// Wurde per `prefetch` angelegt: läuft auch ohne Wartende weiter.
    prefetch_owned: bool,
    waiters: Vec<(u64, oneshot::Sender<LoadResult>)>,
    run: Option<RunState>,
}

struct State {
    mem: MemCache,
    jobs: HashMap<String, Job>,
    queue_visible: VecDeque<String>,
    queue_prefetch: VecDeque<String>,
    running: usize,
    running_prefetch: usize,
    next_waiter: u64,
    next_serial: u64,
    prefetch_enabled: bool,
}

struct Inner {
    fetcher: Arc<dyn ImageFetcher>,
    disk: Option<Arc<DiskCache>>,
    cfg: LoaderConfig,
    handle: Handle,
    decode_sem: Arc<Semaphore>,
    counters: Counters,
    state: Mutex<State>,
}

fn lock_state(m: &Mutex<State>) -> MutexGuard<'_, State> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Asynchroner Bild-Lader. Klonen ist billig (geteilter Zustand).
#[derive(Clone)]
pub struct ImageLoader {
    inner: Arc<Inner>,
}

impl ImageLoader {
    /// Lader auf der aktuellen tokio-Laufzeit (muss im Laufzeit-Kontext aufgerufen werden).
    pub fn new(
        fetcher: Arc<dyn ImageFetcher>,
        disk: Option<Arc<DiskCache>>,
        cfg: LoaderConfig,
    ) -> Self {
        Self::with_handle(Handle::current(), fetcher, disk, cfg)
    }

    /// Lader auf einer bestimmten Laufzeit.
    pub fn with_handle(
        handle: Handle,
        fetcher: Arc<dyn ImageFetcher>,
        disk: Option<Arc<DiskCache>>,
        cfg: LoaderConfig,
    ) -> Self {
        let state = State {
            mem: MemCache::new(cfg.memory_budget_bytes),
            jobs: HashMap::new(),
            queue_visible: VecDeque::new(),
            queue_prefetch: VecDeque::new(),
            running: 0,
            running_prefetch: 0,
            next_waiter: 0,
            next_serial: 0,
            prefetch_enabled: true,
        };
        Self {
            inner: Arc::new(Inner {
                fetcher,
                disk,
                decode_sem: Arc::new(Semaphore::new(cfg.decode_workers.max(1))),
                cfg,
                handle,
                counters: Counters::default(),
                state: Mutex::new(state),
            }),
        }
    }

    /// Fordert ein Bild an. Das Ergebnis liefert der zurückgegebene Handle (ein `Future`);
    /// wird er verworfen, bevor das Bild da ist, endet der Job, sobald niemand mehr wartet.
    pub fn request(&self, req: ImageRequest, priority: Priority) -> ImageHandle {
        let key = req.cache_key();
        let mut st = lock_state(&self.inner.state);
        if let Some(img) = st.mem.get(&key) {
            bump(&self.inner.counters.memory_hits);
            return ImageHandle::ready(Ok(img));
        }
        let (tx, rx) = oneshot::channel();
        let id = st.next_waiter;
        st.next_waiter += 1;
        if let Some(job) = st.jobs.get_mut(&key) {
            job.waiters.push((id, tx));
            bump(&self.inner.counters.deduped);
            let upgrade = priority == Priority::Visible
                && job.priority == Priority::Prefetch
                && job.run.is_none();
            if upgrade {
                job.priority = Priority::Visible;
                st.queue_visible.push_back(key.clone());
            }
        } else {
            let serial = st.next_serial;
            st.next_serial += 1;
            st.jobs.insert(
                key.clone(),
                Job {
                    serial,
                    req,
                    priority,
                    prefetch_owned: false,
                    waiters: vec![(id, tx)],
                    run: None,
                },
            );
            match priority {
                Priority::Visible => st.queue_visible.push_back(key.clone()),
                Priority::Prefetch => st.queue_prefetch.push_back(key.clone()),
            }
        }
        drop(st);
        self.inner.pump();
        ImageHandle {
            state: HandleState::Pending {
                rx,
                inner: self.inner.clone(),
                key,
                id,
            },
        }
    }

    /// Reiht ein Bild zum Vorladen ein (Ergebnis nur in den Caches). `true`, wenn angenommen
    /// oder schon vorhanden bzw. in Arbeit; `false`, wenn Vorladen aus ist oder die Warteschlange voll.
    pub fn prefetch(&self, req: ImageRequest) -> bool {
        let key = req.cache_key();
        let mut st = lock_state(&self.inner.state);
        if !st.prefetch_enabled {
            return false;
        }
        if st.mem.lru.contains(&key) {
            return true;
        }
        if let Some(job) = st.jobs.get_mut(&key) {
            job.prefetch_owned = true;
            return true;
        }
        let queued = st
            .jobs
            .values()
            .filter(|j| j.run.is_none() && j.priority == Priority::Prefetch)
            .count();
        if queued >= self.inner.cfg.max_prefetch_queue {
            return false;
        }
        let serial = st.next_serial;
        st.next_serial += 1;
        st.jobs.insert(
            key.clone(),
            Job {
                serial,
                req,
                priority: Priority::Prefetch,
                prefetch_owned: true,
                waiters: Vec::new(),
                run: None,
            },
        );
        st.queue_prefetch.push_back(key);
        drop(st);
        self.inner.pump();
        true
    }

    /// Bild aus dem Speichercache, ohne etwas anzustoßen (zählt als Nutzung).
    pub fn peek(&self, req: &ImageRequest) -> Option<DecodedImage> {
        lock_state(&self.inner.state).mem.get(&req.cache_key())
    }

    /// Datensparmodus-Teil `prefetch`: aus = nichts mehr vorladen, wartende und laufende
    /// reine Vorlade-Jobs ohne Abnehmer werden abgebrochen.
    pub fn set_prefetch_enabled(&self, on: bool) {
        let mut st = lock_state(&self.inner.state);
        st.prefetch_enabled = on;
        if !on {
            let keys: Vec<String> = st
                .jobs
                .iter()
                .filter(|(_, j)| j.prefetch_owned && j.waiters.is_empty())
                .map(|(k, _)| k.clone())
                .collect();
            for key in keys {
                self.inner.cancel_locked(&mut st, &key);
            }
        }
        drop(st);
        self.inner.pump();
    }

    /// Ob Vorladen erlaubt ist.
    pub fn prefetch_enabled(&self) -> bool {
        lock_state(&self.inner.state).prefetch_enabled
    }

    /// Leert den Speichercache (der Plattencache bleibt).
    pub fn clear_memory(&self) {
        lock_state(&self.inner.state).mem.clear();
    }

    /// Aktuell belegte Bytes im Speichercache.
    pub fn memory_bytes(&self) -> usize {
        lock_state(&self.inner.state).mem.bytes
    }

    /// Anzahl offener Jobs (wartend oder laufend).
    pub fn pending_jobs(&self) -> usize {
        lock_state(&self.inner.state).jobs.len()
    }

    /// Zählerstände.
    pub fn stats(&self) -> LoaderStats {
        let c = &self.inner.counters;
        let get = |a: &AtomicU64| a.load(Ordering::Relaxed);
        LoaderStats {
            memory_hits: get(&c.memory_hits),
            disk_hits: get(&c.disk_hits),
            network_fetches: get(&c.network_fetches),
            decodes: get(&c.decodes),
            deduped: get(&c.deduped),
            cancelled: get(&c.cancelled),
            failed: get(&c.failed),
        }
    }
}

impl Inner {
    /// Startet wartende Jobs, solange Platz ist: Sichtbares zuerst, Vorladen nur mit
    /// begrenztem Anteil der Plätze, damit Sichtbares nie hinter Vorladen steckt.
    fn pump(self: &Arc<Self>) {
        let mut st = lock_state(&self.state);
        let prefetch_cap = (self.cfg.max_concurrent / 2).max(1);
        while st.running < self.cfg.max_concurrent.max(1) {
            let next = Self::pop_queued(&mut st, Priority::Visible).or_else(|| {
                if st.running_prefetch < prefetch_cap {
                    Self::pop_queued(&mut st, Priority::Prefetch)
                } else {
                    None
                }
            });
            let Some(key) = next else {
                break;
            };
            let Some(job) = st.jobs.get_mut(&key) else {
                continue;
            };
            let as_prefetch = job.priority == Priority::Prefetch;
            let (serial, req) = (job.serial, job.req.clone());
            let task = self
                .handle
                .spawn(run_job(self.clone(), key.clone(), serial, req));
            job.run = Some(RunState {
                abort: task.abort_handle(),
                as_prefetch,
            });
            st.running += 1;
            if as_prefetch {
                st.running_prefetch += 1;
            }
        }
    }

    /// Nächster gültiger Schlüssel einer Warteschlange (veraltete Einträge werden übersprungen).
    fn pop_queued(st: &mut State, class: Priority) -> Option<String> {
        loop {
            let key = match class {
                Priority::Visible => st.queue_visible.pop_front()?,
                Priority::Prefetch => st.queue_prefetch.pop_front()?,
            };
            if st
                .jobs
                .get(&key)
                .is_some_and(|j| j.run.is_none() && j.priority == class)
            {
                return Some(key);
            }
        }
    }

    /// Entfernt einen Job (wartend oder laufend) und bricht ihn ab.
    fn cancel_locked(&self, st: &mut State, key: &str) {
        let Some(job) = st.jobs.remove(key) else {
            return;
        };
        bump(&self.counters.cancelled);
        if let Some(run) = job.run {
            run.abort.abort();
            st.running -= 1;
            if run.as_prefetch {
                st.running_prefetch -= 1;
            }
        }
        // Wartende (falls vorhanden) erfahren den Abbruch über das geschlossene Sender-Ende.
    }

    /// Ein Handle wurde verworfen, bevor es fertig war.
    fn drop_waiter(self: &Arc<Self>, key: &str, id: u64) {
        let mut st = lock_state(&self.state);
        let prefetch_enabled = st.prefetch_enabled;
        let Some(job) = st.jobs.get_mut(key) else {
            return;
        };
        job.waiters.retain(|(w, _)| *w != id);
        if job.waiters.is_empty() && (!job.prefetch_owned || !prefetch_enabled) {
            self.cancel_locked(&mut st, key);
        }
        drop(st);
        self.pump();
    }

    fn finish(self: &Arc<Self>, key: &str, serial: u64, result: LoadResult) {
        let mut st = lock_state(&self.state);
        if st.jobs.get(key).is_none_or(|j| j.serial != serial) {
            return;
        }
        let Some(job) = st.jobs.remove(key) else {
            return;
        };
        st.running -= 1;
        if job.run.as_ref().is_some_and(|r| r.as_prefetch) {
            st.running_prefetch -= 1;
        }
        match &result {
            Ok(img) => st.mem.insert(key.to_owned(), img.clone()),
            Err(_) => bump(&self.counters.failed),
        }
        drop(st);
        for (_, tx) in job.waiters {
            let _ = tx.send(result.clone());
        }
        self.pump();
    }

    async fn decode(&self, bytes: bytes::Bytes, req: &ImageRequest) -> LoadResult {
        let _permit = self
            .decode_sem
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| ImageError::Cancelled)?;
        bump(&self.counters.decodes);
        let (w, h, pre) = (req.max_width, req.max_height, req.premultiplied);
        tokio::task::spawn_blocking(move || decode_image(&bytes, w, h, pre))
            .await
            .map_err(|_| ImageError::Decode("Dekodierung abgebrochen".into()))?
    }

    async fn process(&self, req: &ImageRequest) -> LoadResult {
        let disk_key = req.disk_key();
        if let Some(disk) = &self.disk {
            let d = disk.clone();
            let k = disk_key.clone();
            let cached = tokio::task::spawn_blocking(move || d.get(&k))
                .await
                .ok()
                .flatten();
            if let Some(bytes) = cached {
                match self.decode(bytes.into(), req).await {
                    Ok(img) => {
                        bump(&self.counters.disk_hits);
                        return Ok(img);
                    }
                    Err(_) => {
                        // Beschädigter Eintrag: verwerfen und neu laden.
                        let d = disk.clone();
                        let k = disk_key.clone();
                        let _ = tokio::task::spawn_blocking(move || d.remove(&k)).await;
                    }
                }
            }
        }
        let bytes = self.fetcher.fetch(req).await?;
        bump(&self.counters.network_fetches);
        let img = self.decode(bytes.clone(), req).await?;
        if let Some(disk) = &self.disk {
            let d = disk.clone();
            let stored = tokio::task::spawn_blocking(move || d.put(&disk_key, &bytes)).await;
            if let Ok(Err(err)) = stored {
                tracing::warn!("Bild konnte nicht in den Plattencache geschrieben werden: {err}");
            }
        }
        Ok(img)
    }
}

async fn run_job(inner: Arc<Inner>, key: String, serial: u64, req: ImageRequest) {
    let result = inner.process(&req).await;
    inner.finish(&key, serial, result);
}

enum HandleState {
    Ready(Option<LoadResult>),
    Pending {
        rx: oneshot::Receiver<LoadResult>,
        inner: Arc<Inner>,
        key: String,
        id: u64,
    },
    Done,
}

/// Wartet auf ein Bild (`Future`). Verwerfen vor Abschluss bricht die Anfrage ab, sofern
/// niemand sonst auf denselben Job wartet.
pub struct ImageHandle {
    state: HandleState,
}

impl ImageHandle {
    fn ready(result: LoadResult) -> Self {
        Self {
            state: HandleState::Ready(Some(result)),
        }
    }

    /// Bricht die Anfrage ab (gleichbedeutend mit `drop`).
    pub fn cancel(self) {}
}

impl Future for ImageHandle {
    type Output = LoadResult;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match &mut self.state {
            HandleState::Ready(slot) => {
                let out = slot.take().unwrap_or(Err(ImageError::Cancelled));
                self.state = HandleState::Done;
                Poll::Ready(out)
            }
            HandleState::Pending { rx, .. } => match Pin::new(rx).poll(cx) {
                Poll::Pending => Poll::Pending,
                Poll::Ready(res) => {
                    self.state = HandleState::Done;
                    Poll::Ready(res.unwrap_or(Err(ImageError::Cancelled)))
                }
            },
            HandleState::Done => Poll::Ready(Err(ImageError::Cancelled)),
        }
    }
}

impl Drop for ImageHandle {
    fn drop(&mut self) {
        if let HandleState::Pending { inner, key, id, .. } = &self.state {
            inner.drop_waiter(key, *id);
        }
    }
}
