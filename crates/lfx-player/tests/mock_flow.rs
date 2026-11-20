//! `prepare`, Fortschrittsmeldungen und Segmente gegen den Mock-Server (nur 127.0.0.1).

mod common;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use common::{headless_player, write_wav};
use lfx_api::{Client, ClientInfo, PlayMethod};
use lfx_mock_jellyfin::{
    DEMO_PASSWORD, DEMO_USER, MockHandle, PlaybackMode, Recorded, sample_movie_id,
    sample_series_id, start,
};
use lfx_player::segments::{self, SegmentKind, SkipAction};
use lfx_player::{
    ApiSink, PrepareError, PrepareOptions, ProgressReporter, ReporterConfig, StreamKind, prepare,
};

async fn logged_in() -> (Client, MockHandle, SocketAddr) {
    let (addr, handle) = start().await;
    let client = Client::new(
        &format!("http://{addr}"),
        ClientInfo::new("Test-Rechner", "00000000-0000-4000-8000-000000000000"),
    )
    .unwrap();
    client.login(DEMO_USER, DEMO_PASSWORD).await.unwrap();
    (client, handle, addr)
}

fn playback_infos(handle: &MockHandle) -> Vec<serde_json::Value> {
    handle
        .recorded()
        .into_iter()
        .filter_map(|r| match r {
            Recorded::PlaybackInfo { body, .. } => Some(body),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn prepare_direct_play_builds_url_headers_and_track_mapping() {
    let (client, handle, _) = logged_in().await;
    let id = sample_movie_id(0);
    let opts = PrepareOptions {
        audio_stream_index: Some(2),
        subtitle_stream_index: Some(3),
        start_seconds: 42.0,
        ..Default::default()
    };
    let req = prepare(&client, &id, &opts).await.unwrap();

    assert_eq!(req.item_id, id);
    assert_eq!(req.play_method, PlayMethod::DirectPlay);
    assert!(req.stream_url.contains(&format!("/Videos/{id}/stream")));
    assert!(req.stream_url.contains("static=true"));
    assert!(
        req.play_session_id
            .as_deref()
            .is_some_and(|s| s.starts_with("play-"))
    );
    assert_eq!(req.start_seconds, 42.0);
    // Token nur im Header, nie in der Adresse.
    let token = handle.issued_tokens().last().cloned().unwrap();
    assert!(!req.stream_url.contains(&token));
    assert!(!req.stream_url.to_lowercase().contains("api_key"));
    assert_eq!(req.http_headers.len(), 1);
    assert!(req.http_headers[0].starts_with("Authorization: MediaBrowser "));
    assert!(req.http_headers[0].contains(&token));
    // Spurzuordnung: zweite Tonspur (Jellyfin 2 = mpv aid 2), erster Untertitel (3 = sid 1).
    assert_eq!(req.mpv_track_id(StreamKind::Audio, 2), Some(2));
    assert_eq!(req.mpv_track_id(StreamKind::Subtitle, 3), Some(1));
    assert_eq!(req.audio_stream_index, Some(2));
    assert_eq!(req.subtitle_stream_index, Some(3));
    assert!(req.external_subtitles.is_empty());
    // Anfrage: Direktwiedergabe, keine Umwandlung, keine Untertitel einbrennen.
    let infos = playback_infos(&handle);
    assert_eq!(infos.len(), 1);
    assert_eq!(infos[0]["EnableTranscoding"], false);
    assert_eq!(infos[0]["SubtitleStreamIndex"], -1);
}

#[tokio::test]
async fn prepare_uses_source_defaults() {
    let (client, _handle, _) = logged_in().await;
    let req = prepare(&client, &sample_movie_id(1), &PrepareOptions::default())
        .await
        .unwrap();
    assert_eq!(req.audio_stream_index, Some(1));
    assert_eq!(req.subtitle_stream_index, Some(-1));
}

#[tokio::test]
async fn prepare_adds_external_subtitles_without_token() {
    let (client, handle, _) = logged_in().await;
    handle.set_playback_mode(PlaybackMode::ExternalSubtitle);
    let req = prepare(&client, &sample_movie_id(0), &PrepareOptions::default())
        .await
        .unwrap();
    assert_eq!(req.external_subtitles.len(), 1);
    let sub = &req.external_subtitles[0];
    assert!(sub.url.contains("/Subtitles/5/0/Stream.srt"), "{}", sub.url);
    assert!(sub.url.starts_with("http://127.0.0.1"));
    assert_eq!(sub.lang.as_deref(), Some("fra"));
    // Der externe Untertitel (Index 5) kommt nach den zwei eingebetteten: mpv sid 3.
    assert_eq!(req.mpv_track_id(StreamKind::Subtitle, 5), Some(3));
}

#[tokio::test]
async fn prepare_falls_back_to_hls_without_start_time_token_or_subtitle_index() {
    let (client, handle, _) = logged_in().await;
    handle.set_playback_mode(PlaybackMode::ForceTranscode);
    let opts = PrepareOptions {
        audio_stream_index: Some(2),
        start_seconds: 90.0,
        ..Default::default()
    };
    let req = prepare(&client, &sample_movie_id(0), &opts).await.unwrap();
    assert_eq!(req.play_method, PlayMethod::Transcode);
    assert!(req.stream_url.contains("master.m3u8"), "{}", req.stream_url);
    let low = req.stream_url.to_lowercase();
    assert!(
        !low.contains("api_key") && !low.contains("starttimeticks"),
        "{low}"
    );
    assert!(!req.track_mapping);
    assert_eq!(req.start_seconds, 90.0);
    // Zwei Anfragen: erst direkt (ohne Umwandlung), dann mit Umwandlung, Quell-Id und Tonspur.
    let infos = playback_infos(&handle);
    assert_eq!(infos.len(), 2);
    assert_eq!(infos[0]["EnableTranscoding"], false);
    assert_eq!(infos[1]["EnableTranscoding"], true);
    assert_eq!(infos[1]["AudioStreamIndex"], 2);
    assert!(infos[1]["MediaSourceId"].is_string());
    assert!(infos[1].get("SubtitleStreamIndex").is_none());
}

#[tokio::test]
async fn prepare_without_transcode_permission_reports_server_code() {
    let (client, handle, _) = logged_in().await;
    handle.set_playback_mode(PlaybackMode::ForceTranscode);
    let opts = PrepareOptions {
        allow_transcode: false,
        ..Default::default()
    };
    let err = prepare(&client, &sample_movie_id(0), &opts)
        .await
        .unwrap_err();
    assert!(
        matches!(&err, PrepareError::NoPlayableSource(c) if c == "NoCompatibleStream"),
        "{err:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn reporter_sends_playing_progress_and_stopped() {
    let Some(player) = headless_player() else {
        return;
    };
    let (client, handle, _) = logged_in().await;
    let id = sample_movie_id(0);
    let mut req = prepare(&client, &id, &PrepareOptions::default())
        .await
        .unwrap();
    let wav = write_wav(30);
    req.stream_url = wav.to_string_lossy().into_owned();
    req.http_headers.clear();
    handle.clear_recorded();

    let reporter = ProgressReporter::spawn(
        &player,
        req.clone(),
        Arc::new(ApiSink::new(client.clone())),
        ReporterConfig {
            report_progress: true,
            interval: Duration::from_millis(300),
        },
    );
    player.open(req);
    let count = |pred: fn(&Recorded) -> bool| handle.recorded().iter().filter(|r| pred(r)).count();
    wait_until("Playing + 2 Progress", || {
        count(|r| matches!(r, Recorded::Playing(_))) == 1
            && count(|r| matches!(r, Recorded::Progress(_))) >= 2
    })
    .await;

    player.pause();
    wait_until("Pause-Progress", || {
        handle
            .recorded()
            .iter()
            .any(|r| matches!(r, Recorded::Progress(v) if v["IsPaused"] == true))
    })
    .await;
    player.seek_to(20.0);
    wait_until("Seek-Progress", || {
        handle.recorded().iter().any(|r| {
            matches!(r, Recorded::Progress(v)
                if (v["PositionTicks"].as_i64().unwrap_or(0) - 200_000_000).abs() < 15_000_000)
        })
    })
    .await;
    player.stop();
    tokio::time::timeout(Duration::from_secs(10), reporter.join())
        .await
        .expect("Melder endet");

    let rec = handle.recorded();
    let kinds: Vec<&str> = rec
        .iter()
        .filter_map(|r| match r {
            Recorded::Playing(_) => Some("P"),
            Recorded::Progress(_) => Some("G"),
            Recorded::Stopped(_) => Some("S"),
            Recorded::PlaybackInfo { .. } => None,
        })
        .collect();
    assert_eq!(kinds.first(), Some(&"P"), "{kinds:?}");
    assert_eq!(kinds.last(), Some(&"S"), "{kinds:?}");
    assert_eq!(kinds.iter().filter(|k| **k == "S").count(), 1);
    let Some(Recorded::Stopped(stop)) = rec.last() else {
        panic!("kein Stopped am Ende");
    };
    let ticks = stop["PositionTicks"].as_i64().unwrap();
    assert!(
        (ticks - 200_000_000).abs() < 40_000_000,
        "Stopped bei {ticks}"
    );
    assert_eq!(stop["PlaySessionId"], req_session(&rec));
    // Zwischenmeldungen: Position nie rückwärts außer nach dem Seek, Ticks plausibel (> 0).
    for r in &rec {
        if let Recorded::Progress(v) | Recorded::Playing(v) = r {
            assert!(v["PositionTicks"].as_i64().unwrap() >= 0);
            assert_eq!(v["ItemId"], id);
            assert_eq!(v["PlayMethod"], "DirectPlay");
        }
    }
    let _ = std::fs::remove_file(wav);
}

fn req_session(rec: &[Recorded]) -> serde_json::Value {
    match &rec[0] {
        Recorded::Playing(v) => v["PlaySessionId"].clone(),
        other => panic!("{other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn reporter_stops_at_duration_on_eof() {
    let Some(player) = headless_player() else {
        return;
    };
    let (client, handle, _) = logged_in().await;
    let mut req = prepare(&client, &sample_movie_id(0), &PrepareOptions::default())
        .await
        .unwrap();
    let wav = write_wav(2);
    req.stream_url = wav.to_string_lossy().into_owned();
    req.http_headers.clear();
    handle.clear_recorded();
    let reporter = ProgressReporter::spawn(
        &player,
        req.clone(),
        Arc::new(ApiSink::new(client)),
        ReporterConfig::default(),
    );
    player.open(req);
    tokio::time::timeout(Duration::from_secs(15), reporter.join())
        .await
        .expect("Melder endet nach Eof");
    let rec = handle.recorded();
    let Some(Recorded::Stopped(stop)) = rec.last() else {
        panic!("{rec:?}");
    };
    let ticks = stop["PositionTicks"].as_i64().unwrap();
    assert!((ticks - 20_000_000).abs() < 3_000_000, "{ticks}");
    assert_eq!(stop["Failed"], false);
    let _ = std::fs::remove_file(wav);
}

#[tokio::test(flavor = "multi_thread")]
async fn reporter_is_silent_when_disabled() {
    let Some(player) = headless_player() else {
        return;
    };
    let (client, handle, _) = logged_in().await;
    let mut req = prepare(&client, &sample_movie_id(0), &PrepareOptions::default())
        .await
        .unwrap();
    let wav = write_wav(2);
    req.stream_url = wav.to_string_lossy().into_owned();
    req.http_headers.clear();
    handle.clear_recorded();
    let reporter = ProgressReporter::spawn(
        &player,
        req.clone(),
        Arc::new(ApiSink::new(client)),
        ReporterConfig {
            report_progress: false,
            ..Default::default()
        },
    );
    player.open(req);
    let mut ev = player.subscribe_events();
    // Ende abwarten (Eof), dann darf nichts aufgezeichnet sein.
    tokio::time::timeout(Duration::from_secs(15), async {
        while !matches!(ev.recv().await, Ok(lfx_player::PlayerEvent::Eof { .. })) {}
    })
    .await
    .expect("Eof");
    reporter.join().await;
    assert!(
        handle
            .recorded()
            .iter()
            .all(|r| matches!(r, Recorded::PlaybackInfo { .. }))
    );
    let _ = std::fs::remove_file(wav);
}

#[tokio::test]
async fn segments_and_next_episode_from_mock() {
    let (client, _handle, _) = logged_in().await;
    let id = sample_movie_id(0);
    let segs = segments::fetch_segments(&client, &id).await.unwrap();
    assert_eq!(segs.len(), 2);
    assert_eq!(segs[0].kind, SegmentKind::Intro);
    assert_eq!((segs[0].start, segs[0].end), (30.0, 90.0));
    let (_, action) = segments::skippable_at(&segs, 31.0, None).unwrap();
    assert_eq!(action, SkipAction::SeekTo(90.0));
    let outro = segs[1];
    let (_, action) = segments::skippable_at(&segs, outro.start + 1.0, Some(outro.end)).unwrap();
    assert_eq!(action, SkipAction::NextItem);
    assert!(segments::should_show_up_next(
        &segs,
        outro.start + 1.0,
        Some(outro.end),
        30.0
    ));

    let series = sample_series_id(0);
    let eps = client.episodes(&series, None, &[]).await.unwrap().items;
    assert!(eps.len() >= 2, "Mock liefert zu wenige Folgen");
    let first = &eps[0];
    let next = segments::next_episode(&client, first).await.unwrap();
    assert!(next.is_some());
    assert_ne!(next.unwrap().id, first.id);
    let last = eps.last().unwrap();
    let after_last = segments::next_episode(&client, last).await.unwrap();
    // Die letzte Folge der Serie hat keinen Nachfolger.
    let max_key = eps
        .iter()
        .map(|e| (e.parent_index_number, e.index_number))
        .max()
        .unwrap();
    if (last.parent_index_number, last.index_number) == max_key {
        assert!(after_last.is_none());
    }
    // Filme haben keine nächste Folge.
    let movie = client.item(&id, &[]).await.unwrap();
    assert!(
        segments::next_episode(&client, &movie)
            .await
            .unwrap()
            .is_none()
    );
}

async fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let t0 = std::time::Instant::now();
    while !f() {
        assert!(
            t0.elapsed() < Duration::from_secs(15),
            "Zeitüberschreitung: {what}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}
