//! Offline-Bibliothek, Fortschritt offline und Outbox-Abgleich gegen den Mock.

mod common;

use common::*;
use lfx_api::PlaybackReport;
use lfx_mock_jellyfin::catalog::{SERVER_ID, USER_ID};
use lfx_mock_jellyfin::w35_offline::download_bytes;
use lfx_offline::{
    DownloadManager, DownloadState, EnqueueOptions, MemoryOutbox, OfflineError, OfflineOutbox,
    OfflineProgress, OutboxSink, OutboxSyncer, SqliteDownloadStore, SqliteOutbox, iso_from_millis,
};
use lfx_player::{Report, ReportSink};
use lfx_store::Store;
use serde_json::json;
use std::sync::Arc;

struct Rig {
    c: Ctx,
    mgr: DownloadManager,
    outbox: Arc<dyn OfflineOutbox>,
    progress: OfflineProgress,
    syncer: OutboxSyncer,
}

async fn rig_with(c: Ctx, outbox: Arc<dyn OfflineOutbox>) -> Rig {
    let mgr = c.manager();
    let progress = OfflineProgress::new(
        mgr.library(),
        outbox.clone(),
        c.clock.clone(),
        SERVER_ID,
        USER_ID,
    );
    let syncer = OutboxSyncer::new(
        c.client.clone(),
        mgr.library(),
        outbox.clone(),
        SERVER_ID,
        USER_ID,
    );
    Rig {
        c,
        mgr,
        outbox,
        progress,
        syncer,
    }
}

async fn rig() -> Rig {
    rig_with(ctx().await, Arc::new(MemoryOutbox::new())).await
}

impl Rig {
    async fn download(&self, item: &str) {
        let ids = self
            .mgr
            .enqueue(item, EnqueueOptions::original())
            .await
            .unwrap();
        wait_state(&self.mgr, &ids[0], DownloadState::Completed).await;
    }

    fn pending(&self) -> usize {
        self.outbox
            .pending(SERVER_ID, USER_ID, lfx_offline::OUTBOX_KIND)
            .unwrap()
            .len()
    }
}

const MIN: i64 = 10_000_000 * 60; // Ticks je Minute

#[tokio::test]
async fn library_lists_measures_plays_and_deletes() {
    let r = rig().await;
    let (a, b) = (movie(0), movie(1));
    r.download(&a).await;
    r.download(&b).await;
    let lib = r.mgr.library();

    assert_eq!(lib.entries().unwrap().len(), 2);
    assert!(lib.is_available(&a));
    let usage = lib.usage().unwrap();
    assert_eq!(usage.completed, 2);
    assert_eq!(usage.incomplete, 0);
    assert!(usage.completed_bytes >= (download_bytes(&a).len() + download_bytes(&b).len()) as u64);

    // Lokale Wiedergabe-Anforderung: Pfad statt URL, keine Header.
    let play = lib.playback(&a).unwrap();
    let req = play.to_play_request();
    assert_eq!(std::path::Path::new(&req.stream_url), play.entry.media_path);
    assert!(req.http_headers.is_empty());
    assert_eq!(req.start_seconds, 0.0);

    // Beschädigte Datei wird erkannt.
    std::fs::write(&play.entry.media_path, b"kaputt").unwrap();
    assert!(!lib.verify(&a).await.unwrap());

    lib.delete(&a).unwrap();
    assert!(!play.entry.media_path.exists());
    assert!(lib.entry(&a).unwrap().is_none());
    assert!(matches!(lib.delete(&a), Err(OfflineError::UnknownDownload)));
    assert_eq!(lib.entries().unwrap().len(), 1);
}

#[tokio::test]
async fn delete_series_removes_all_episodes() {
    let r = rig().await;
    let series = lfx_mock_jellyfin::sample_series_id(1);
    let ids = r
        .mgr
        .enqueue(&series, EnqueueOptions::original())
        .await
        .unwrap();
    for id in &ids {
        wait_state(&r.mgr, id, DownloadState::Completed).await;
    }
    assert_eq!(r.mgr.library().delete_series(&series).unwrap(), ids.len());
    assert!(r.mgr.list().unwrap().is_empty());
}

#[tokio::test]
async fn resume_position_comes_from_local_progress() {
    let r = rig().await;
    let item = movie(0);
    r.download(&item).await;
    r.progress.record(&item, 20 * MIN, false).unwrap();
    let play = r.mgr.library().playback(&item).unwrap();
    assert_eq!(play.start_ticks, 20 * MIN);
    assert!((play.to_play_request().start_seconds - 1200.0).abs() < 1e-6);
}

#[tokio::test]
async fn local_newer_wins_and_is_pushed_with_local_time() {
    let r = rig().await;
    let item = movie(0);
    r.c.mock
        .set_server_user_data(&item, 5 * MIN, false, Some("2020-01-01T00:00:00.0000000Z"));
    r.download(&item).await;

    // Mehrere Meldungen: nur die neueste bleibt.
    r.c.clock.set(1_800_000_000_000);
    r.progress.record(&item, 10 * MIN, false).unwrap();
    r.c.clock.set(1_800_000_060_000);
    r.progress.record(&item, 30 * MIN, false).unwrap();
    assert_eq!(r.pending(), 1);

    let report = r.syncer.sync().await.unwrap();
    assert_eq!((report.pushed, report.kept_server), (1, 0));
    assert_eq!(r.pending(), 0);
    let server = r.c.mock.server_user_data(&item);
    assert_eq!(server.position_ticks, 30 * MIN);
    // Der lokale Zeitpunkt bleibt erhalten, nicht „jetzt“.
    let sent = server.last_played.unwrap();
    assert_eq!(
        lfx_offline::millis_from_iso(&sent),
        Some(1_800_000_060_000),
        "{sent}"
    );
}

#[tokio::test]
async fn server_newer_wins_and_local_is_discarded() {
    let r = rig().await;
    let item = movie(1);
    r.download(&item).await;
    r.c.clock.set(1_800_000_000_000);
    r.progress.record(&item, 10 * MIN, false).unwrap();
    // Anderswo wurde später weitergeschaut.
    let later = iso_from_millis(1_900_000_000_000);
    r.c.mock
        .set_server_user_data(&item, 77 * MIN, false, Some(&later));

    let report = r.syncer.sync().await.unwrap();
    assert_eq!((report.pushed, report.kept_server), (0, 1));
    assert_eq!(r.pending(), 0);
    let server = r.c.mock.server_user_data(&item);
    assert_eq!(server.position_ticks, 77 * MIN, "Server bleibt unberührt");
    assert_eq!(server.last_played.as_deref(), Some(later.as_str()));
    // Lokal gilt jetzt der Server-Stand.
    let local = r.mgr.library().local_user_data(&item).unwrap().unwrap();
    assert_eq!(local.position_ticks, 77 * MIN);
    assert_eq!(local.last_played_ms, 1_900_000_000_000);
}

#[tokio::test]
async fn equal_timestamps_prefer_the_server() {
    let r = rig().await;
    let item = movie(2);
    r.download(&item).await;
    r.c.clock.set(1_800_000_000_000);
    r.progress.record(&item, 10 * MIN, false).unwrap();
    r.c.mock.set_server_user_data(
        &item,
        99 * MIN,
        false,
        Some(&iso_from_millis(1_800_000_000_000)),
    );
    let report = r.syncer.sync().await.unwrap();
    assert_eq!((report.pushed, report.kept_server), (0, 1));
    assert_eq!(r.c.mock.server_user_data(&item).position_ticks, 99 * MIN);
}

#[tokio::test]
async fn network_failure_keeps_entries_and_next_sync_succeeds() {
    let r = rig().await;
    let item = movie(3);
    r.download(&item).await;
    r.progress.record(&item, 12 * MIN, false).unwrap();

    // Der Client wiederholt GET zweimal: drei Fehlschläge nacheinander.
    r.c.mock.fail_next(3, 503);
    let report = r.syncer.sync().await.unwrap();
    assert_eq!(report.failed, 1);
    assert_eq!(r.pending(), 1);
    let entry = &r
        .outbox
        .pending(SERVER_ID, USER_ID, lfx_offline::OUTBOX_KIND)
        .unwrap()[0];
    assert_eq!(entry.attempts, 1);

    let report = r.syncer.sync().await.unwrap();
    assert_eq!(report.pushed, 1);
    assert_eq!(r.pending(), 0);
    assert_eq!(r.c.mock.server_user_data(&item).position_ticks, 12 * MIN);
}

#[tokio::test]
async fn expired_token_aborts_without_losing_entries() {
    let r = rig().await;
    let item = movie(4);
    r.download(&item).await;
    r.progress.record(&item, 3 * MIN, false).unwrap();
    r.c.mock.revoke_tokens();
    let err = r.syncer.sync().await.unwrap_err();
    assert!(matches!(
        err,
        OfflineError::Api(lfx_api::ApiError::Unauthorized)
    ));
    assert_eq!(r.pending(), 1);
}

#[tokio::test]
async fn foreign_outbox_kinds_are_left_alone() {
    let r = rig().await;
    let item = movie(5);
    r.download(&item).await;
    r.outbox
        .push(SERVER_ID, USER_ID, "favorite", &json!({"item_id": item}))
        .unwrap();
    r.progress.record(&item, MIN, false).unwrap();
    r.syncer.sync().await.unwrap();
    assert_eq!(
        r.outbox
            .pending(SERVER_ID, USER_ID, "favorite")
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn item_gone_on_server_drops_the_entry() {
    let r = rig().await;
    let item = movie(6);
    r.download(&item).await;
    r.progress.record(&item, MIN, false).unwrap();
    // Eintrag für ein Element, das es nicht (mehr) gibt.
    r.outbox
        .push(
            SERVER_ID,
            USER_ID,
            lfx_offline::OUTBOX_KIND,
            &json!({"item_id": "ffffffffffffffffffffffffffffffff", "position_ticks": 1, "played": false, "last_played_ms": 5}),
        )
        .unwrap();
    let report = r.syncer.sync().await.unwrap();
    assert_eq!((report.dropped, report.pushed), (1, 1));
    assert_eq!(r.pending(), 0);
}

#[tokio::test]
async fn played_threshold_and_auto_delete_wait_for_sync() {
    let r = rig().await;
    let item = movie(7);
    r.download(&item).await;
    let lib = r.mgr.library();
    lib.set_auto_delete(true);
    let runtime = lib
        .entry(&item)
        .unwrap()
        .unwrap()
        .record
        .runtime_ticks
        .unwrap();

    let local = r.progress.record(&item, runtime * 95 / 100, false).unwrap();
    assert!(local.played);
    assert_eq!(local.position_ticks, 0);
    assert_eq!(local.play_count, 1);

    // Noch nicht abgeglichen: nichts wird gelöscht.
    assert!(lib.apply_auto_delete(r.outbox.as_ref()).unwrap().is_empty());
    assert!(lib.is_available(&item));

    let report = r.syncer.sync().await.unwrap();
    assert_eq!(report.pushed, 1);
    assert_eq!(report.auto_deleted, 1);
    assert!(!lib.is_available(&item));
    assert!(r.c.mock.server_user_data(&item).played);
}

#[tokio::test]
async fn auto_delete_is_off_by_default() {
    let r = rig().await;
    let item = movie(8);
    r.download(&item).await;
    r.progress.record(&item, MIN, true).unwrap();
    let report = r.syncer.sync().await.unwrap();
    assert_eq!(report.auto_deleted, 0);
    assert!(r.mgr.library().is_available(&item));
}

#[tokio::test]
async fn outbox_sink_collects_player_reports() {
    let r = rig().await;
    let item = movie(9);
    r.download(&item).await;
    let sink = OutboxSink::new(r.progress.clone());
    let report = |ticks| Report::Progress(PlaybackReport::direct(item.clone(), "ms", "ps", ticks));
    sink.send(Report::Playing(PlaybackReport::direct(
        item.clone(),
        "ms",
        "ps",
        0,
    )))
    .await
    .unwrap();
    sink.send(report(MIN)).await.unwrap();
    sink.send(report(2 * MIN)).await.unwrap();
    assert_eq!(r.pending(), 1);
    // Unbekannte Titel werden still verworfen.
    sink.send(Report::Progress(PlaybackReport::direct(
        "ffff", "ms", "ps", 5,
    )))
    .await
    .unwrap();
    assert_eq!(r.pending(), 1);
    r.syncer.sync().await.unwrap();
    assert_eq!(r.c.mock.server_user_data(&item).position_ticks, 2 * MIN);
}

#[tokio::test]
async fn sqlite_backed_flow_survives_restart() {
    let c = ctx().await;
    let db = c.dir.path().join("lfx.db");
    let item = movie(0);
    {
        let store = Store::open(&db).unwrap();
        let mut setup = c.setup(test_config());
        setup.store = Arc::new(SqliteDownloadStore::new(store.clone()));
        let mgr = DownloadManager::new(setup).unwrap();
        let ids = mgr
            .enqueue(&item, EnqueueOptions::original())
            .await
            .unwrap();
        wait_state(&mgr, &ids[0], DownloadState::Completed).await;
        let progress = OfflineProgress::new(
            mgr.library(),
            Arc::new(SqliteOutbox::new(store)),
            c.clock.clone(),
            SERVER_ID,
            USER_ID,
        );
        progress.record(&item, 8 * MIN, false).unwrap();
    }
    // Neustart: alles kommt aus der Datenbank, Abgleich sendet den Stand.
    let store = Store::open(&db).unwrap();
    let mut setup = c.setup(test_config());
    setup.store = Arc::new(SqliteDownloadStore::new(store.clone()));
    let mgr = DownloadManager::new(setup).unwrap();
    assert!(mgr.library().is_available(&item));
    assert_eq!(mgr.library().playback(&item).unwrap().start_ticks, 8 * MIN);
    let outbox: Arc<dyn OfflineOutbox> = Arc::new(SqliteOutbox::new(store.clone()));
    let syncer = OutboxSyncer::new(c.client.clone(), mgr.library(), outbox, SERVER_ID, USER_ID);
    assert_eq!(syncer.pending().unwrap(), 1);
    let report = syncer.sync().await.unwrap();
    assert_eq!(report.pushed, 1);
    assert_eq!(syncer.pending().unwrap(), 0);
    assert_eq!(c.mock.server_user_data(&item).position_ticks, 8 * MIN);
    assert_eq!(store.outbox_len().unwrap(), 0);
}
