//! Download-Manager gegen den Mock: Abschluss, Abbruch und Fortsetzen, Pause, Neustart, Platz, Sammel-Downloads,
//! Umwandlung, Zubehör.

mod common;

use common::*;
use lfx_api::{Client, ClientInfo};
use lfx_mock_jellyfin::w35_offline::{TRANSCODE_LEN, download_bytes, transcode_bytes};
use lfx_offline::{
    DownloadState, EnqueueOptions, ManagerConfig, OfflineError, OfflineEvent, Quality,
    SqliteDownloadStore, TranscodeProfile,
};
use lfx_store::Store;
use std::sync::Arc;
use std::time::Duration;

fn media_bytes(mgr: &lfx_offline::DownloadManager, item: &str) -> Vec<u8> {
    let entry = mgr.library().entry(item).unwrap().expect("fertig");
    std::fs::read(entry.media_path).unwrap()
}

#[tokio::test]
async fn original_download_completes_with_accessories() {
    let c = ctx().await;
    c.mock.set_external_subtitles(true);
    let mgr = c.manager();
    let item = movie(0);
    let ids = mgr
        .enqueue(&item, EnqueueOptions::original())
        .await
        .unwrap();
    assert_eq!(ids.len(), 1);
    let rec = wait_state(&mgr, &ids[0], DownloadState::Completed).await;

    assert_eq!(media_bytes(&mgr, &item), download_bytes(&item));
    assert_eq!(rec.bytes_total, Some(download_bytes(&item).len() as u64));
    assert_eq!(rec.sha256.as_deref().map(str::len), Some(64));
    assert!(mgr.library().verify(&item).await.unwrap());
    // Zubehör: Bilder, Untertitel, Element-JSON.
    let entry = mgr.library().entry(&item).unwrap().unwrap();
    for kind in ["primary", "backdrop", "logo"] {
        assert!(entry.images[kind].is_file(), "{kind}");
    }
    assert_eq!(entry.subtitles.len(), 1);
    assert_eq!(entry.subtitles[0].index, 5);
    assert!(
        std::fs::read_to_string(&entry.subtitles[0].path)
            .unwrap()
            .contains("Beispieltext")
    );
    let json = mgr.library().item_json(&item).unwrap();
    assert_eq!(json["Id"], item.as_str());
    assert!(
        rec.accessory_errors.is_empty(),
        "{:?}",
        rec.accessory_errors
    );
    // Kein Token in Datensatz oder Auth-frei erreichbaren Texten.
    let token = c.client.session().unwrap().token.expose().to_owned();
    assert!(!format!("{rec:?}").contains(&token));
    assert!(!json.to_string().contains(&token));
    // Originaldownload ohne Range beim ersten Abruf.
    let hits = c.mock.download_hits();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].range, None);
}

#[tokio::test]
async fn abort_midway_retries_with_range() {
    let c = ctx().await;
    c.mock.abort_next_download_after(50_000);
    let mgr = c.manager();
    let item = movie(1);
    let ids = mgr
        .enqueue(&item, EnqueueOptions::original())
        .await
        .unwrap();
    wait_state(&mgr, &ids[0], DownloadState::Completed).await;
    assert_eq!(media_bytes(&mgr, &item), download_bytes(&item));
    let hits = c.mock.download_hits();
    assert_eq!(hits.len(), 2, "{hits:?}");
    let range = hits[1].range.clone().expect("Fortsetzung per Range");
    let start: u64 = range
        .strip_prefix("bytes=")
        .and_then(|r| r.strip_suffix('-'))
        .and_then(|n| n.parse().ok())
        .expect("offener Bereich");
    assert!(start > 0 && start <= 50_000, "{range}");
}

#[tokio::test]
async fn failed_download_resumes_by_hand() {
    let c = ctx().await;
    c.mock.abort_next_download_after(40_000);
    let mgr = c.manager_with(ManagerConfig {
        max_retries: 0,
        ..test_config()
    });
    let item = movie(2);
    let ids = mgr
        .enqueue(&item, EnqueueOptions::original())
        .await
        .unwrap();
    let failed = wait_state(&mgr, &ids[0], DownloadState::Failed).await;
    assert!(failed.error.is_some());
    let dir = mgr.item_dir(&failed).unwrap();
    let part_len = std::fs::metadata(dir.join("media.part")).unwrap().len();
    assert!(part_len > 0 && part_len <= 40_000);

    mgr.resume(&ids[0]).unwrap();
    wait_state(&mgr, &ids[0], DownloadState::Completed).await;
    assert_eq!(media_bytes(&mgr, &item), download_bytes(&item));
    let hits = c.mock.download_hits();
    assert_eq!(
        hits[1].range.as_deref(),
        Some(format!("bytes={part_len}-").as_str())
    );
    assert!(!dir.join("media.part").exists());
}

#[tokio::test]
async fn pause_and_resume_are_deterministic() {
    let c = ctx().await;
    c.mock.hold_next_download_after(40_000);
    let mgr = c.manager();
    let mut events = mgr.subscribe();
    let item = movie(3);
    let ids = mgr
        .enqueue(&item, EnqueueOptions::original())
        .await
        .unwrap();
    // Warten, bis genau 40000 Byte angekommen sind (der Mock hält danach an).
    loop {
        let ev = tokio::time::timeout(Duration::from_secs(20), events.recv())
            .await
            .expect("Fortschritt")
            .unwrap();
        if let OfflineEvent::Progress { done, .. } = ev
            && done >= 40_000
        {
            assert_eq!(done, 40_000);
            break;
        }
    }
    mgr.pause(&ids[0]).unwrap();
    let paused = wait_state(&mgr, &ids[0], DownloadState::Paused).await;
    assert_eq!(paused.bytes_done, 40_000);
    c.mock.release_download();

    mgr.resume(&ids[0]).unwrap();
    wait_state(&mgr, &ids[0], DownloadState::Completed).await;
    assert_eq!(media_bytes(&mgr, &item), download_bytes(&item));
    let hits = c.mock.download_hits();
    assert_eq!(hits[1].range.as_deref(), Some("bytes=40000-"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn restart_resumes_partial_file() {
    // „Absturz“: der erste Manager läuft in einer eigenen Runtime, die mitten im Download verworfen wird.
    let c = ctx().await;
    let db = c.dir.path().join("lfx.db");
    let item = movie(4);
    c.mock.hold_next_download_after(30_000);
    let mut setup = c.setup(test_config());
    setup.store = Arc::new(SqliteDownloadStore::new(Store::open(&db).unwrap()));
    let first_item = item.clone();
    let crash = std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mgr = lfx_offline::DownloadManager::new(setup).unwrap();
        let id = rt.block_on(async {
            let ids = mgr
                .enqueue(&first_item, EnqueueOptions::original())
                .await
                .unwrap();
            let part = mgr
                .item_dir(&mgr.get(&ids[0]).unwrap().unwrap())
                .unwrap()
                .join("media.part");
            let end = std::time::Instant::now() + Duration::from_secs(20);
            while std::fs::metadata(&part).map_or(0, |m| m.len()) < 30_000 {
                assert!(std::time::Instant::now() < end, "Teildatei wächst nicht");
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            ids[0].clone()
        });
        drop(rt); // bricht alle Tasks ab
        id
    });
    let id = tokio::task::spawn_blocking(move || crash.join().unwrap())
        .await
        .unwrap();
    c.mock.release_download();

    // Neustart: neue Datenbankverbindung, neuer Manager, `restore()`.
    let mut setup = c.setup(test_config());
    setup.store = Arc::new(SqliteDownloadStore::new(Store::open(&db).unwrap()));
    let mgr = lfx_offline::DownloadManager::new(setup).unwrap();
    let before = mgr.get(&id).unwrap().unwrap();
    assert_eq!(before.state, DownloadState::Downloading);
    let part_len = std::fs::metadata(mgr.item_dir(&before).unwrap().join("media.part"))
        .unwrap()
        .len();
    assert!(part_len >= 30_000);
    mgr.restore().unwrap();
    wait_state(&mgr, &id, DownloadState::Completed).await;
    assert_eq!(media_bytes(&mgr, &item), download_bytes(&item));
    let hits = c.mock.download_hits();
    assert_eq!(
        hits.last().unwrap().range.as_deref(),
        Some(format!("bytes={part_len}-").as_str())
    );
}

#[tokio::test]
async fn server_without_range_restarts_cleanly() {
    let c = ctx().await;
    c.mock.set_download_range(false);
    c.mock.abort_next_download_after(30_000);
    let mgr = c.manager();
    let item = movie(5);
    let ids = mgr
        .enqueue(&item, EnqueueOptions::original())
        .await
        .unwrap();
    wait_state(&mgr, &ids[0], DownloadState::Completed).await;
    // Die Antwort 200 auf die Range-Anfrage führt zum Neustart bei null; das Ergebnis bleibt exakt.
    assert_eq!(media_bytes(&mgr, &item), download_bytes(&item));
    assert!(c.mock.download_hits().len() >= 2);
}

#[tokio::test]
async fn forbidden_download_fails_with_message() {
    let c = ctx().await;
    c.mock.forbid_download(true);
    let mgr = c.manager();
    let ids = mgr
        .enqueue(&movie(0), EnqueueOptions::original())
        .await
        .unwrap();
    let rec = wait_state(&mgr, &ids[0], DownloadState::Failed).await;
    assert!(rec.error.unwrap().contains("nicht erlaubt"));
}

#[tokio::test]
async fn too_little_space_is_refused_before_anything_is_created() {
    let c = ctx().await;
    c.space.set(100 * 1024 * 1024); // weniger als die 1,5 GB, die der Mock ankündigt
    let mgr = c.manager();
    let err = mgr
        .enqueue(&movie(0), EnqueueOptions::original())
        .await
        .unwrap_err();
    assert!(
        matches!(err, OfflineError::InsufficientSpace { .. }),
        "{err}"
    );
    assert!(mgr.list().unwrap().is_empty());
    assert!(c.mock.download_hits().is_empty());

    // Reserve zählt mit.
    c.space.set(1_500_000_000 + 10);
    let mgr = c.manager_with(ManagerConfig {
        min_free_bytes: 1_000_000,
        ..test_config()
    });
    assert!(
        mgr.enqueue(&movie(0), EnqueueOptions::original())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn series_batch_respects_parallel_limit_and_only_unplayed() {
    let c = ctx().await;
    let mgr = c.manager();
    let mut events = mgr.subscribe();
    let series = lfx_mock_jellyfin::sample_series_id(0);
    let ids = mgr
        .enqueue(&series, EnqueueOptions::original())
        .await
        .unwrap();
    assert_eq!(ids.len(), 6); // 2 Staffeln mit je 3 Folgen
    for id in &ids {
        wait_state(&mgr, id, DownloadState::Completed).await;
    }
    // Höchstens zwei gleichzeitig „Läuft“.
    let (mut running, mut max) = (0i32, 0i32);
    let mut last = std::collections::HashMap::new();
    while let Ok(ev) = events.try_recv() {
        if let OfflineEvent::State { id, state } = ev {
            let was = last.insert(id, state) == Some(DownloadState::Downloading);
            running -= i32::from(was);
            running += i32::from(state == DownloadState::Downloading);
            max = max.max(running);
        }
    }
    assert!((1..=2).contains(&max), "{max}");
    let recs = mgr.list().unwrap();
    assert!(
        recs.iter()
            .all(|r| r.group.as_deref() == Some(series.as_str()))
    );
    assert!(
        recs.iter()
            .all(|r| r.series_name.is_some() && r.season_number.is_some())
    );
    // Serien-/Staffel-JSON liegt für die Offline-Anzeige bereit.
    let first = &recs[0].item_id;
    assert_eq!(mgr.library().series_json(first).unwrap()["Type"], "Series");
    assert_eq!(mgr.library().season_json(first).unwrap()["Type"], "Season");

    // Zweite Serie: nur Ungesehenes.
    let series2 = lfx_mock_jellyfin::sample_series_id(1);
    let episodes = c.client.episodes(&series2, None, &[]).await.unwrap().items;
    c.mock.set_server_user_data(&episodes[0].id, 0, true, None);
    let opts = EnqueueOptions {
        only_unplayed: true,
        ..EnqueueOptions::original()
    };
    let ids2 = mgr.enqueue(&series2, opts).await.unwrap();
    assert_eq!(ids2.len(), episodes.len() - 1);
}

#[tokio::test]
async fn enqueue_twice_does_not_duplicate() {
    let c = ctx().await;
    let mgr = c.manager();
    let a = mgr
        .enqueue(&movie(7), EnqueueOptions::original())
        .await
        .unwrap();
    let b = mgr
        .enqueue(&movie(7), EnqueueOptions::original())
        .await
        .unwrap();
    assert_eq!(a, b);
    assert_eq!(mgr.list().unwrap().len(), 1);
    wait_state(&mgr, &a[0], DownloadState::Completed).await;
}

#[tokio::test]
async fn cancel_removes_files_and_record() {
    let c = ctx().await;
    c.mock.hold_next_download_after(20_000);
    let mgr = c.manager();
    let mut events = mgr.subscribe();
    let ids = mgr
        .enqueue(&movie(8), EnqueueOptions::original())
        .await
        .unwrap();
    loop {
        if let OfflineEvent::Progress { done, .. } = events.recv().await.unwrap()
            && done >= 20_000
        {
            break;
        }
    }
    let rec = mgr.get(&ids[0]).unwrap().unwrap();
    let dir = mgr.item_dir(&rec).unwrap();
    assert!(dir.exists());
    mgr.cancel(&ids[0]).await.unwrap();
    c.mock.release_download();
    assert!(!dir.exists());
    assert!(mgr.get(&ids[0]).unwrap().is_none());
    assert!(matches!(
        mgr.cancel(&ids[0]).await,
        Err(OfflineError::UnknownDownload)
    ));
}

#[tokio::test]
async fn transcoded_download_uses_safe_url_and_ends_server_job() {
    let c = ctx().await;
    let mgr = c.manager();
    let item = movie(9);
    let ids = mgr
        .enqueue(&item, EnqueueOptions::transcoded(TranscodeProfile::small()))
        .await
        .unwrap();
    let rec = wait_state(&mgr, &ids[0], DownloadState::Completed).await;
    assert_eq!(rec.extension, "mp4");
    assert_eq!(rec.bytes_total, Some(TRANSCODE_LEN));
    assert_eq!(media_bytes(&mgr, &item), transcode_bytes(&item));

    let hits = c.mock.download_hits();
    assert_eq!(hits.len(), 1);
    let q = &hits[0].query;
    assert_eq!(hits[0].path, format!("/Videos/{item}/stream.mp4"));
    assert!(q.contains("MaxStreamingBitrate=1200000"), "{q}");
    assert!(
        q.contains("VideoCodec=h264") && q.contains("maxHeight=480"),
        "{q}"
    );
    let token = c.client.session().unwrap().token.expose().to_owned();
    for bad in [
        "StartTimeTicks",
        "SubtitleStreamIndex",
        "SubtitleMethod",
        "api_key",
        "ApiKey",
    ] {
        assert!(!q.contains(bad), "{bad} in {q}");
    }
    assert!(!q.contains(&token));
    assert_eq!(hits[0].range, None, "Umwandlung ohne Range");
    // Der Serverjob wird beendet.
    assert!(
        c.mock
            .requests()
            .iter()
            .any(|r| r.method == "DELETE" && r.path == "/Videos/ActiveEncodings")
    );
    // Eingebettete Textuntertitel liegen als Dateien bei (stecken nicht in der MP4).
    let entry = mgr.library().entry(&item).unwrap().unwrap();
    assert!(entry.subtitles.iter().any(|s| s.index == 3));
    assert!(matches!(rec.quality, Quality::Transcoded(_)));
}

#[tokio::test]
async fn transcoded_download_restarts_from_zero_after_abort() {
    let c = ctx().await;
    c.mock.abort_next_download_after(30_000);
    let mgr = c.manager();
    let item = movie(10);
    let ids = mgr
        .enqueue(
            &item,
            EnqueueOptions::transcoded(TranscodeProfile::medium()),
        )
        .await
        .unwrap();
    wait_state(&mgr, &ids[0], DownloadState::Completed).await;
    assert_eq!(media_bytes(&mgr, &item), transcode_bytes(&item));
    let hits = c.mock.download_hits();
    assert_eq!(hits.len(), 2);
    assert!(hits.iter().all(|h| h.range.is_none()));
}

#[tokio::test]
async fn account_without_download_right_is_refused_for_original_only() {
    let c = ctx().await;
    let guest = Client::new(
        &format!("http://{}", c.addr),
        ClientInfo::new("Test", "dev-2"),
    )
    .unwrap();
    guest
        .login(
            lfx_mock_jellyfin::GUEST_USER,
            lfx_mock_jellyfin::GUEST_PASSWORD,
        )
        .await
        .unwrap();
    let mut setup = c.setup(test_config());
    setup.client = guest;
    let mgr = lfx_offline::DownloadManager::new(setup).unwrap();
    let err = mgr
        .enqueue(&movie(0), EnqueueOptions::original())
        .await
        .unwrap_err();
    assert!(matches!(err, OfflineError::NotAllowed), "{err}");
    assert!(c.mock.download_hits().is_empty());
    // Umgewandelt braucht das Recht nicht.
    let ids = mgr
        .enqueue(
            &movie(0),
            EnqueueOptions::transcoded(TranscodeProfile::small()),
        )
        .await;
    assert!(ids.is_ok());
}
