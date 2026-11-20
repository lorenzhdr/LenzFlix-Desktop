//! Lader-Tests mit Fake-Fetcher (kein Netzwerk) und synthetischen Bildern.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use image::{ImageEncoder, codecs::png::PngEncoder};
use lfx_api::ImageType;
use lfx_images::{
    BoxFuture, DiskCache, ImageError, ImageFetcher, ImageLoader, ImageRequest, LoaderConfig,
    Priority,
};
use tokio::sync::Semaphore;

fn png(w: u32, h: u32, shade: u8) -> Bytes {
    let img = image::RgbaImage::from_fn(w, h, |x, y| {
        image::Rgba([shade, (x % 256) as u8, (y % 256) as u8, 255])
    });
    let mut out = Vec::new();
    PngEncoder::new(&mut out)
        .write_image(img.as_raw(), w, h, image::ExtendedColorType::Rgba8)
        .unwrap();
    Bytes::from(out)
}

fn req(id: &str) -> ImageRequest {
    ImageRequest::new(id, ImageType::Primary).tag("t1")
}

/// Fetcher mit Zählern, Abrufreihenfolge, optionalem Tor und Abbruch-Erkennung.
struct Fake {
    calls: AtomicUsize,
    order: Mutex<Vec<String>>,
    gate: Option<Arc<Semaphore>>,
    started: Arc<Semaphore>,
    dropped: Arc<AtomicBool>,
    fail: AtomicBool,
    size: (u32, u32),
}

impl Fake {
    fn new(gate: bool) -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicUsize::new(0),
            order: Mutex::new(Vec::new()),
            gate: gate.then(|| Arc::new(Semaphore::new(0))),
            started: Arc::new(Semaphore::new(0)),
            dropped: Arc::new(AtomicBool::new(false)),
            fail: AtomicBool::new(false),
            size: (40, 60),
        })
    }
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
    fn open_gate(&self, n: usize) {
        self.gate.as_ref().unwrap().add_permits(n);
    }
}

struct DropFlag(Arc<AtomicBool>, bool);
impl Drop for DropFlag {
    fn drop(&mut self) {
        if !self.1 {
            self.0.store(true, Ordering::SeqCst);
        }
    }
}

impl ImageFetcher for Fake {
    fn fetch<'a>(&'a self, request: &'a ImageRequest) -> BoxFuture<'a, Result<Bytes, ImageError>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.order.lock().unwrap().push(request.item_id.clone());
            self.started.add_permits(1);
            let mut flag = DropFlag(self.dropped.clone(), false);
            if let Some(gate) = &self.gate {
                gate.acquire().await.unwrap().forget();
            }
            flag.1 = true;
            if self.fail.load(Ordering::SeqCst) {
                return Err(ImageError::Fetch("Server antwortet nicht".into()));
            }
            Ok(png(self.size.0, self.size.1, request.item_id.len() as u8))
        })
    }
}

fn cfg(max_concurrent: usize) -> LoaderConfig {
    LoaderConfig {
        max_concurrent,
        ..LoaderConfig::default()
    }
}

#[tokio::test]
async fn loads_and_scales() {
    let fake = Fake::new(false);
    let loader = ImageLoader::new(fake.clone(), None, cfg(4));
    let img = loader
        .request(req("a").max_size(20, 20), Priority::Visible)
        .await
        .unwrap();
    assert_eq!((img.width, img.height), (13, 20));
    assert_eq!(img.pixels.len(), 13 * 20 * 4);
}

#[tokio::test]
async fn dedupes_concurrent_requests_and_hits_memory() {
    let fake = Fake::new(true);
    let loader = ImageLoader::new(fake.clone(), None, cfg(4));
    let handles: Vec<_> = (0..5)
        .map(|_| loader.request(req("a"), Priority::Visible))
        .collect();
    fake.open_gate(1);
    let results = futures_join(handles).await;
    assert!(
        results
            .iter()
            .all(|r| r.as_ref().unwrap() == results[0].as_ref().unwrap())
    );
    assert_eq!(fake.calls(), 1);
    assert_eq!(loader.stats().deduped, 4);
    // Zweiter Zugriff kommt aus dem Speicher.
    let again = loader.request(req("a"), Priority::Visible).await.unwrap();
    assert_eq!(again, *results[0].as_ref().unwrap());
    assert_eq!(fake.calls(), 1);
    assert_eq!(loader.stats().memory_hits, 1);
    assert!(loader.peek(&req("a")).is_some());
    assert_eq!(loader.pending_jobs(), 0);
}

async fn futures_join(handles: Vec<lfx_images::ImageHandle>) -> Vec<lfx_images::LoadResult> {
    let mut out = Vec::new();
    for h in handles {
        out.push(h.await);
    }
    out
}

#[tokio::test]
async fn visible_runs_before_prefetch() {
    let fake = Fake::new(true);
    let loader = ImageLoader::new(fake.clone(), None, cfg(1));
    let first = loader.request(req("first"), Priority::Visible);
    assert!(loader.prefetch(req("pre")));
    let vis = loader.request(req("vis"), Priority::Visible);
    fake.open_gate(3);
    first.await.unwrap();
    vis.await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while loader.pending_jobs() > 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(*fake.order.lock().unwrap(), ["first", "vis", "pre"]);
}

#[tokio::test]
async fn visible_request_upgrades_queued_prefetch() {
    let fake = Fake::new(true);
    let loader = ImageLoader::new(fake.clone(), None, cfg(1));
    let first = loader.request(req("first"), Priority::Visible);
    assert!(loader.prefetch(req("p1")));
    assert!(loader.prefetch(req("p2")));
    let up = loader.request(req("p2"), Priority::Visible);
    fake.open_gate(3);
    first.await.unwrap();
    up.await.unwrap();
    assert_eq!(fake.order.lock().unwrap()[..2], ["first", "p2"]);
}

#[tokio::test]
async fn dropping_a_queued_handle_cancels_the_job() {
    let fake = Fake::new(true);
    let loader = ImageLoader::new(fake.clone(), None, cfg(1));
    let first = loader.request(req("first"), Priority::Visible);
    let queued = loader.request(req("queued"), Priority::Visible);
    drop(queued);
    fake.open_gate(2);
    first.await.unwrap();
    assert_eq!(loader.stats().cancelled, 1);
    assert_eq!(loader.pending_jobs(), 0);
    assert_eq!(*fake.order.lock().unwrap(), ["first"]);
}

#[tokio::test]
async fn dropping_a_running_handle_aborts_the_fetch() {
    let fake = Fake::new(true);
    let loader = ImageLoader::new(fake.clone(), None, cfg(2));
    let h = loader.request(req("a"), Priority::Visible);
    fake.started.acquire().await.unwrap().forget();
    drop(h);
    tokio::time::timeout(Duration::from_secs(5), async {
        while !fake.dropped.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("Abruf wurde nicht abgebrochen");
    assert_eq!(loader.pending_jobs(), 0);
    assert_eq!(loader.stats().cancelled, 1);
    // Der Platz ist wieder frei: neue Anfrage läuft durch.
    fake.open_gate(1);
    loader.request(req("b"), Priority::Visible).await.unwrap();
}

#[tokio::test]
async fn shared_job_survives_one_dropped_handle() {
    let fake = Fake::new(true);
    let loader = ImageLoader::new(fake.clone(), None, cfg(2));
    let a = loader.request(req("a"), Priority::Visible);
    let b = loader.request(req("a"), Priority::Visible);
    drop(a);
    fake.open_gate(1);
    b.await.unwrap();
    assert_eq!(fake.calls(), 1);
    assert_eq!(loader.stats().cancelled, 0);
}

#[tokio::test]
async fn disk_cache_roundtrip_across_loaders() {
    let tmp = tempfile::tempdir().unwrap();
    let disk = Arc::new(DiskCache::open_default(tmp.path(), 1 << 20).unwrap());
    let fake = Fake::new(false);
    let loader = ImageLoader::new(fake.clone(), Some(disk.clone()), cfg(2));
    let first = loader.request(req("a"), Priority::Visible).await.unwrap();
    assert_eq!(fake.calls(), 1);
    assert_eq!(disk.len(), 1);
    drop(loader);
    drop(disk);

    // Neuer Lader, leerer Speicher, gleicher Ordner: kein Abruf.
    let disk2 = Arc::new(DiskCache::open_default(tmp.path(), 1 << 20).unwrap());
    let fake2 = Fake::new(false);
    let loader2 = ImageLoader::new(fake2.clone(), Some(disk2), cfg(2));
    let second = loader2.request(req("a"), Priority::Visible).await.unwrap();
    assert_eq!(fake2.calls(), 0);
    assert_eq!(loader2.stats().disk_hits, 1);
    assert_eq!(first, second);
}

#[tokio::test]
async fn corrupt_disk_entry_falls_back_to_fetch() {
    let tmp = tempfile::tempdir().unwrap();
    let disk = Arc::new(DiskCache::open_default(tmp.path(), 1 << 20).unwrap());
    disk.put(&req("a").disk_key(), b"kein bild").unwrap();
    let fake = Fake::new(false);
    let loader = ImageLoader::new(fake.clone(), Some(disk.clone()), cfg(2));
    loader.request(req("a"), Priority::Visible).await.unwrap();
    assert_eq!(fake.calls(), 1);
    assert_eq!(loader.stats().disk_hits, 0);
    // Gültige Bytes ersetzen den kaputten Eintrag.
    assert!(image::load_from_memory(&disk.get(&req("a").disk_key()).unwrap()).is_ok());
}

#[tokio::test]
async fn memory_budget_evicts_least_recently_used() {
    let fake = Fake::new(false);
    // Ein Bild 40x60x4 = 9600 Bytes; Budget für zwei.
    let loader = ImageLoader::new(
        fake.clone(),
        None,
        LoaderConfig {
            memory_budget_bytes: 20_000,
            ..cfg(2)
        },
    );
    for id in ["a", "b"] {
        loader.request(req(id), Priority::Visible).await.unwrap();
    }
    assert!(loader.peek(&req("a")).is_some()); // a ist jetzt jünger als b
    loader.request(req("c"), Priority::Visible).await.unwrap();
    assert!(loader.peek(&req("a")).is_some());
    assert!(loader.peek(&req("b")).is_none());
    assert!(loader.peek(&req("c")).is_some());
    assert!(loader.memory_bytes() <= 20_000);
}

#[tokio::test]
async fn prefetch_obeys_switch_and_queue_limit() {
    let fake = Fake::new(true);
    let loader = ImageLoader::new(
        fake.clone(),
        None,
        LoaderConfig {
            max_prefetch_queue: 2,
            ..cfg(1)
        },
    );
    // Platz blockieren, damit Vorladen wartet.
    let first = loader.request(req("first"), Priority::Visible);
    assert!(loader.prefetch(req("p1")));
    assert!(loader.prefetch(req("p2")));
    assert!(!loader.prefetch(req("p3")), "Warteschlange voll");
    assert!(loader.prefetch(req("p1")), "doppelt ist ok");

    loader.set_prefetch_enabled(false);
    assert!(!loader.prefetch_enabled());
    assert!(!loader.prefetch(req("p4")));
    assert_eq!(loader.pending_jobs(), 1, "wartende Vorlade-Jobs entfallen");
    fake.open_gate(5);
    first.await.unwrap();
    assert_eq!(*fake.order.lock().unwrap(), ["first"]);

    loader.set_prefetch_enabled(true);
    assert!(loader.prefetch(req("p5")));
    tokio::time::timeout(Duration::from_secs(5), async {
        while loader.peek(&req("p5")).is_none() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert!(loader.prefetch(req("p5")), "schon im Speicher");
    assert_eq!(fake.calls(), 2);
}

#[tokio::test]
async fn disabling_prefetch_aborts_running_prefetch_without_waiters() {
    let fake = Fake::new(true);
    let loader = ImageLoader::new(fake.clone(), None, cfg(2));
    assert!(loader.prefetch(req("p")));
    fake.started.acquire().await.unwrap().forget();
    loader.set_prefetch_enabled(false);
    tokio::time::timeout(Duration::from_secs(5), async {
        while !fake.dropped.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(loader.pending_jobs(), 0);
}

#[tokio::test]
async fn errors_are_reported_to_all_waiters_and_not_cached() {
    let fake = Fake::new(false);
    fake.fail.store(true, Ordering::SeqCst);
    let loader = ImageLoader::new(fake.clone(), None, cfg(2));
    let a = loader.request(req("a"), Priority::Visible);
    let b = loader.request(req("a"), Priority::Visible);
    assert!(matches!(a.await, Err(ImageError::Fetch(_))));
    assert!(matches!(b.await, Err(ImageError::Fetch(_))));
    assert_eq!(loader.stats().failed, 1);
    fake.fail.store(false, Ordering::SeqCst);
    loader.request(req("a"), Priority::Visible).await.unwrap();
    assert_eq!(fake.calls(), 2);
}

#[tokio::test]
async fn undecodable_bytes_give_decode_error() {
    struct Junk;
    impl ImageFetcher for Junk {
        fn fetch<'a>(&'a self, _: &'a ImageRequest) -> BoxFuture<'a, Result<Bytes, ImageError>> {
            Box::pin(async { Ok(Bytes::from_static(b"<html>nope</html>")) })
        }
    }
    let loader = ImageLoader::new(Arc::new(Junk), None, cfg(2));
    let err = loader
        .request(req("x"), Priority::Visible)
        .await
        .unwrap_err();
    assert!(matches!(err, ImageError::Decode(_)));
}

#[tokio::test]
async fn premultiplied_and_straight_are_separate_cache_entries() {
    let fake = Fake::new(false);
    let loader = ImageLoader::new(fake.clone(), None, cfg(2));
    let a = loader.request(req("a"), Priority::Visible).await.unwrap();
    let b = loader
        .request(req("a").premultiplied(true), Priority::Visible)
        .await
        .unwrap();
    assert!(!a.premultiplied && b.premultiplied);
    assert_eq!(fake.calls(), 2);
}

#[tokio::test]
async fn api_fetcher_uses_header_auth_without_token_in_url() {
    use lfx_api::{Client, ClientInfo};
    use lfx_mock_jellyfin::{DEMO_PASSWORD, DEMO_USER};
    let (addr, mock) = lfx_mock_jellyfin::start().await;
    let client = Client::new(
        &format!("http://{addr}"),
        ClientInfo::new("Test-Rechner", "00000000-0000-4000-8000-000000000000"),
    )
    .unwrap();
    client.login(DEMO_USER, DEMO_PASSWORD).await.unwrap();
    let token = client.session().unwrap().token;
    let loader = ImageLoader::new(Arc::new(lfx_images::ApiFetcher::new(client)), None, cfg(2));
    let request = ImageRequest::new(lfx_mock_jellyfin::sample_movie_id(0), ImageType::Primary)
        .tag("t1")
        .max_size(100, 100);
    // Der Mock liefert ein gültiges PNG (24x36): der Abruf und das Dekodieren gelingen.
    let image = loader
        .request(request, Priority::Visible)
        .await
        .expect("Bild laden");
    assert_eq!((image.width, image.height), (24, 36));
    // Auth läuft über den Header; im Anfrageprotokoll steht kein Token und kein ApiKey.
    let header = mock.last_auth_header().unwrap();
    assert!(header.contains(token.expose()));
    let log = format!("{:?}", mock.requests());
    assert!(!log.contains(token.expose()) && !log.contains("api_key") && !log.contains("ApiKey"));
    assert!(log.contains("/Images/Primary"));
    mock.shutdown().await;
}
