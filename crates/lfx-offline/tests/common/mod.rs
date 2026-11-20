//! Gemeinsame Hilfen der Integrationstests (Mock-Server, Client, Manager).
#![allow(dead_code)]

use lfx_api::{Client, ClientInfo};
use lfx_mock_jellyfin::catalog::{SERVER_ID, USER_ID};
use lfx_mock_jellyfin::{DEMO_PASSWORD, DEMO_USER, MockHandle};
use lfx_offline::{
    Clock, DownloadManager, DownloadRecord, DownloadState, DownloadStore, FixedSpace,
    ManagerConfig, ManagerSetup, MemoryDownloadStore, OfflineLayout,
};
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{Duration, Instant};

/// Uhr mit einstellbarer Zeit.
pub struct FixedClock(pub AtomicI64);

impl FixedClock {
    pub fn new(ms: i64) -> Arc<Self> {
        Arc::new(Self(AtomicI64::new(ms)))
    }
    pub fn set(&self, ms: i64) {
        self.0.store(ms, Ordering::SeqCst);
    }
}

impl Clock for FixedClock {
    fn now_millis(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

pub struct Ctx {
    pub addr: SocketAddr,
    pub mock: MockHandle,
    pub client: Client,
    pub dir: tempfile::TempDir,
    pub layout: OfflineLayout,
    pub store: Arc<dyn DownloadStore>,
    pub space: Arc<FixedSpace>,
    pub clock: Arc<FixedClock>,
}

pub fn test_config() -> ManagerConfig {
    ManagerConfig {
        max_parallel: 2,
        min_free_bytes: 0,
        max_retries: 3,
        retry_delay: Duration::from_millis(10),
        idle_timeout: Duration::from_secs(10),
        connect_timeout: Duration::from_secs(5),
        progress_interval: Duration::ZERO,
    }
}

pub async fn client_for(addr: SocketAddr) -> Client {
    let client = Client::new(&format!("http://{addr}"), ClientInfo::new("Test", "dev-1")).unwrap();
    client.login(DEMO_USER, DEMO_PASSWORD).await.unwrap();
    client
}

pub async fn ctx() -> Ctx {
    let (addr, mock) = lfx_mock_jellyfin::start().await;
    let client = client_for(addr).await;
    let dir = tempfile::tempdir().unwrap();
    Ctx {
        addr,
        mock,
        client,
        layout: OfflineLayout::new(dir.path().join("offline")),
        dir,
        store: Arc::new(MemoryDownloadStore::new()),
        space: Arc::new(FixedSpace::new(10 * 1024 * 1024 * 1024)),
        clock: FixedClock::new(1_800_000_000_000),
    }
}

impl Ctx {
    pub fn setup(&self, config: ManagerConfig) -> ManagerSetup {
        let mut s = ManagerSetup::new(
            self.client.clone(),
            SERVER_ID,
            USER_ID,
            self.layout.clone(),
            self.store.clone(),
        );
        s.space = self.space.clone();
        s.clock = self.clock.clone();
        s.config = config;
        s
    }

    pub fn manager(&self) -> DownloadManager {
        self.manager_with(test_config())
    }

    pub fn manager_with(&self, config: ManagerConfig) -> DownloadManager {
        DownloadManager::new(self.setup(config)).unwrap()
    }
}

/// Wartet (mit Zeitlimit), bis der Datensatz die Bedingung erfüllt.
pub async fn wait_record(
    mgr: &DownloadManager,
    id: &str,
    what: &str,
    ok: impl Fn(&DownloadRecord) -> bool,
) -> DownloadRecord {
    let end = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(rec) = mgr.get(id).unwrap()
            && ok(&rec)
        {
            return rec;
        }
        assert!(
            Instant::now() < end,
            "Zeitüberschreitung beim Warten auf {what}: {:?}",
            mgr.get(id).unwrap().map(|r| (r.state, r.error))
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

pub async fn wait_state(mgr: &DownloadManager, id: &str, state: DownloadState) -> DownloadRecord {
    wait_record(mgr, id, state.as_str(), |r| r.state == state).await
}

pub fn movie(n: usize) -> String {
    lfx_mock_jellyfin::sample_movie_id(n)
}
