//! Integrationstests gegen den Mock-Server (kein echtes Netz, nur 127.0.0.1).

use lfx_api::{
    ApiError, Client, ClientInfo, ClientOptions, ImageOptions, ImageType, ItemFilter, ItemKind,
    ItemsQuery, PlaybackInfoRequest, PlaybackReport, PlaybackStop, RetryPolicy, Secret, SortBy,
    SortOrder, TICKS_PER_SECOND, discover_at,
};
use lfx_mock_jellyfin::{
    DEMO_PASSWORD, DEMO_USER, MockConfig, MockHandle, Recorded, sample_movie_id, sample_series_id,
    start, start_with,
};
use std::net::SocketAddr;
use std::time::Duration;

fn info() -> ClientInfo {
    ClientInfo::new("Test-Rechner", "00000000-0000-4000-8000-000000000000")
}

fn fast_options() -> ClientOptions {
    ClientOptions {
        timeout: Duration::from_secs(5),
        connect_timeout: Duration::from_secs(2),
        retry: RetryPolicy {
            retries: 2,
            base_delay: Duration::from_millis(5),
        },
    }
}

fn client_for(addr: SocketAddr) -> Client {
    Client::with_options(&format!("http://{addr}"), info(), fast_options()).unwrap()
}

async fn logged_in() -> (Client, MockHandle) {
    let (addr, handle) = start().await;
    let client = client_for(addr);
    client.login(DEMO_USER, DEMO_PASSWORD).await.unwrap();
    (client, handle)
}

#[tokio::test]
async fn public_info_needs_no_login() {
    let (addr, _h) = start().await;
    let info = client_for(addr).public_info().await.unwrap();
    assert_eq!(info.server_name, "Mock-Videothek");
    assert!(info.startup_wizard_completed);
}

#[tokio::test]
async fn login_ok_and_wrong_password() {
    let (addr, handle) = start().await;
    let client = client_for(addr);
    let err = client.login(DEMO_USER, "falsch").await.unwrap_err();
    assert!(matches!(err, ApiError::Unauthorized), "{err:?}");
    assert!(!client.is_logged_in());

    let result = client.login(DEMO_USER, DEMO_PASSWORD).await.unwrap();
    assert_eq!(result.user.name, DEMO_USER);
    assert!(client.is_logged_in());
    assert_eq!(client.me().await.unwrap().name, DEMO_USER);
    assert_eq!(handle.issued_tokens().len(), 1);
}

#[tokio::test]
async fn requests_without_token_are_rejected() {
    let (addr, _h) = start().await;
    let client = client_for(addr);
    client.set_session(Some(lfx_api::Session {
        token: Secret::new("falsch"),
        user_id: "x".into(),
    }));
    assert!(matches!(
        client.views().await.unwrap_err(),
        ApiError::Unauthorized
    ));
    client.set_session(None);
    assert!(matches!(
        client.views().await.unwrap_err(),
        ApiError::NotLoggedIn
    ));
}

#[tokio::test]
async fn logout_invalidates_token() {
    let (client, handle) = logged_in().await;
    let session = client.session().unwrap();
    client.logout().await.unwrap();
    assert!(!client.is_logged_in());
    assert!(
        handle.issued_tokens().is_empty(),
        "Server hat das Token verworfen"
    );
    client.set_session(Some(session));
    assert!(matches!(
        client.me().await.unwrap_err(),
        ApiError::Unauthorized
    ));
}

#[tokio::test]
async fn quick_connect_flow() {
    let (addr, handle) = start().await;
    let client = client_for(addr);
    assert!(client.quick_connect_enabled().await.unwrap());

    let qc = client.quick_connect_initiate().await.unwrap();
    assert_eq!(qc.code.len(), 6);
    assert!(!qc.authenticated);
    assert!(
        !client
            .quick_connect_poll(&qc.secret)
            .await
            .unwrap()
            .authenticated
    );
    // Vor der Bestätigung lehnt der Server den Abschluss ab.
    assert!(client.quick_connect_login(&qc.secret).await.is_err());

    assert_eq!(handle.authorize_quick_connect(), 1);
    assert!(
        client
            .quick_connect_poll(&qc.secret)
            .await
            .unwrap()
            .authenticated
    );
    let result = client.quick_connect_login(&qc.secret).await.unwrap();
    assert_eq!(result.user.name, DEMO_USER);
    assert!(client.is_logged_in());
    assert_eq!(client.me().await.unwrap().name, DEMO_USER);
}

#[tokio::test]
async fn quick_connect_auto_and_disabled() {
    let (addr, _h) = start_with(MockConfig {
        quick_connect_auto_after_polls: Some(1),
        ..MockConfig::default()
    })
    .await;
    let client = client_for(addr);
    let qc = client.quick_connect_initiate().await.unwrap();
    assert!(
        !client
            .quick_connect_poll(&qc.secret)
            .await
            .unwrap()
            .authenticated
    );
    assert!(
        client
            .quick_connect_poll(&qc.secret)
            .await
            .unwrap()
            .authenticated
    );

    let (addr, _h2) = start_with(MockConfig {
        quick_connect_enabled: false,
        ..MockConfig::default()
    })
    .await;
    let client = client_for(addr);
    assert!(!client.quick_connect_enabled().await.unwrap());
    assert!(matches!(
        client.quick_connect_initiate().await.unwrap_err(),
        ApiError::QuickConnectDisabled
    ));
}

#[tokio::test]
async fn auth_header_format_is_sent() {
    let (client, handle) = logged_in().await;
    client.me().await.unwrap();
    let sent = handle.last_auth_header().unwrap();
    let token = client.session().unwrap().token;
    let expected = format!(
        "MediaBrowser Client=\"LENZFLIX\", Device=\"Test-Rechner\", DeviceId=\"00000000-0000-4000-8000-000000000000\", Version=\"{}\", Token=\"{}\"",
        env!("CARGO_PKG_VERSION"),
        token.expose()
    );
    assert_eq!(sent, expected);
    assert_eq!(client.auth_header_value(), expected);

    // Ohne Sitzung: kein Token-Feld.
    client.set_session(None);
    let anon = client.auth_header_value();
    assert!(anon.starts_with("MediaBrowser Client=\"LENZFLIX\", Device=\"Test-Rechner\""));
    assert!(!anon.contains("Token="));
}

#[tokio::test]
async fn secret_is_redacted_everywhere() {
    let (client, handle) = logged_in().await;
    let token = handle.issued_tokens().remove(0);
    let secret = Secret::new(token.clone());
    assert_eq!(format!("{secret:?}"), "***");
    assert_eq!(format!("{secret}"), "***");
    let session = client.session().unwrap();
    for text in [
        format!("{client:?}"),
        format!("{session:?}"),
        format!(
            "{:?}",
            client.login(DEMO_USER, DEMO_PASSWORD).await.unwrap()
        ),
    ] {
        assert!(!text.contains("mock-token"), "Token im Debug-Text: {text}");
    }
    // Bild- und Stream-URLs enthalten kein Token.
    let id = sample_movie_id(0);
    let urls = [
        client
            .image_url(
                &id,
                ImageType::Primary,
                Some("tag1"),
                ImageOptions {
                    max_width: Some(300),
                    ..Default::default()
                },
            )
            .unwrap(),
        client.stream_url(&id, "ms1").unwrap(),
    ];
    for url in urls {
        assert!(
            !url.contains("mock-token")
                && !url.to_lowercase().contains("api_key")
                && !url.to_lowercase().contains("apikey"),
            "{url}"
        );
    }
}

#[tokio::test]
async fn views_and_items_queries() {
    let (client, _h) = logged_in().await;
    let views = client.views().await.unwrap();
    assert_eq!(views.items.len(), 2);
    assert_eq!(views.items[0].collection_type.as_deref(), Some("movies"));

    let movies = client
        .items(
            &ItemsQuery::new()
                .include_types(&[ItemKind::Movie])
                .recursive(true)
                .sort_by(&[SortBy::SortName])
                .total_count(true),
        )
        .await
        .unwrap();
    assert_eq!(movies.items.len(), 12);
    assert_eq!(movies.total_record_count, 12);
    let names: Vec<_> = movies.items.iter().map(|i| i.name.clone()).collect();
    let mut sorted = names.clone();
    sorted.sort_by_key(|n| n.to_lowercase());
    assert_eq!(names, sorted);

    let page = client
        .items(
            &ItemsQuery::new()
                .include_types(&[ItemKind::Movie])
                .recursive(true)
                .sort_by(&[SortBy::CommunityRating])
                .order(SortOrder::Descending)
                .limit(3)
                .start_index(1),
        )
        .await
        .unwrap();
    assert_eq!(page.items.len(), 3);
    assert!(page.items[0].community_rating >= page.items[1].community_rating);
    assert_eq!(page.start_index, 1);
    assert_eq!(page.total_record_count, 12);

    let scifi = client
        .items(
            &ItemsQuery::new()
                .include_types(&[ItemKind::Movie])
                .recursive(true)
                .genres(&["Science-Fiction"]),
        )
        .await
        .unwrap();
    assert_eq!(scifi.items.len(), 3);
    assert!(
        scifi
            .items
            .iter()
            .all(|i| i.genres.iter().any(|g| g == "Science-Fiction"))
    );

    let search = client
        .items(&ItemsQuery::new().recursive(true).search_term("leucht"))
        .await
        .unwrap();
    assert_eq!(search.items.len(), 1);
    assert_eq!(search.items[0].name, "Der letzte Leuchtturm");

    let in_view = client
        .items(&ItemsQuery::new().parent_id(&views.items[1].id))
        .await
        .unwrap();
    assert!(in_view.items.iter().all(|i| i.kind == ItemKind::Series));
    assert_eq!(in_view.items.len(), 2);
}

#[tokio::test]
async fn item_series_and_lists() {
    let (client, _h) = logged_in().await;
    let id = sample_movie_id(1);
    let item = client
        .item(
            &id,
            &[
                "Overview",
                "People",
                "MediaSources",
                "Chapters",
                "Trickplay",
            ],
        )
        .await
        .unwrap();
    assert_eq!(item.name, "Sternenstaub-Archiv");
    assert_eq!(item.kind, ItemKind::Movie);
    assert_eq!(item.runtime_seconds(), Some(132.0 * 60.0));
    assert_eq!(item.people.len(), 2);
    assert_eq!(item.media_sources[0].media_streams.len(), 5);
    assert_eq!(item.chapters.len(), 2);
    assert!(item.image_tags.contains_key("Primary"));

    let tp = client.trickplay_info(&id).await.unwrap();
    let info = tp.values().next().unwrap().get("320").unwrap();
    assert_eq!((info.width, info.tile_width), (320, 10));

    assert!(matches!(
        client
            .item("00000000000000000000000000000000", &[])
            .await
            .unwrap_err(),
        ApiError::NotFound
    ));

    let series = sample_series_id(0);
    let seasons = client.seasons(&series, &[]).await.unwrap();
    assert_eq!(seasons.items.len(), 2);
    let all_eps = client.episodes(&series, None, &[]).await.unwrap();
    assert_eq!(all_eps.items.len(), 6);
    let s2 = client
        .episodes(&series, Some(&seasons.items[1].id), &[])
        .await
        .unwrap();
    assert_eq!(s2.items.len(), 3);
    assert_eq!(s2.items[0].parent_index_number, Some(2));

    let latest = client
        .latest(None, &[ItemKind::Movie], 5, &[])
        .await
        .unwrap();
    assert_eq!(latest.len(), 5);
    assert!(latest[0].date_created >= latest[1].date_created);

    let similar = client.similar(&id, 4, &[]).await.unwrap();
    assert_eq!(similar.items.len(), 4);
    assert!(similar.items.iter().all(|i| i.id != id));

    let genres = client.genres(None, &[ItemKind::Movie]).await.unwrap();
    assert!(genres.items.iter().any(|g| g.name == "Krimi"));
}

#[tokio::test]
async fn user_data_resume_and_next_up() {
    let (client, _h) = logged_in().await;
    let movie = sample_movie_id(3);
    let data = client.set_favorite(&movie, true).await.unwrap();
    assert!(data.is_favorite);
    let favs = client
        .items(&ItemsQuery::new().recursive(true).is_favorite(true))
        .await
        .unwrap();
    assert_eq!(favs.items.len(), 1);
    assert!(
        !client
            .set_favorite(&movie, false)
            .await
            .unwrap()
            .is_favorite
    );

    assert!(client.set_played(&movie, true).await.unwrap().played);
    let unplayed = client
        .items(
            &ItemsQuery::new()
                .include_types(&[ItemKind::Movie])
                .recursive(true)
                .filters(&[ItemFilter::IsUnplayed]),
        )
        .await
        .unwrap();
    assert_eq!(unplayed.items.len(), 11);
    assert!(!client.set_played(&movie, false).await.unwrap().played);

    assert_eq!(
        client.set_rating(&movie, Some(true)).await.unwrap().likes,
        Some(true)
    );
    assert_eq!(client.set_rating(&movie, None).await.unwrap().likes, None);

    // Weiterschauen und Als Nächstes folgen dem Wiedergabestand.
    assert!(
        client
            .resume(10, &["Video"], &[])
            .await
            .unwrap()
            .items
            .is_empty()
    );
    let series = sample_series_id(0);
    let eps = client.episodes(&series, None, &[]).await.unwrap().items;
    assert!(
        client
            .next_up(None, 10, &[])
            .await
            .unwrap()
            .items
            .is_empty()
    );
    client.set_played(&eps[0].id, true).await.unwrap();
    let next = client.next_up(None, 10, &[]).await.unwrap();
    assert_eq!(next.items.len(), 1);
    assert_eq!(next.items[0].id, eps[1].id);
    assert_eq!(
        client
            .next_up(Some(&sample_series_id(1)), 10, &[])
            .await
            .unwrap()
            .items
            .len(),
        0
    );

    client
        .report_progress(&PlaybackReport::direct(
            &movie,
            "ms",
            "play",
            10 * 60 * TICKS_PER_SECOND,
        ))
        .await
        .unwrap();
    let resume = client.resume(10, &[], &[]).await.unwrap();
    assert_eq!(resume.items.len(), 1);
    assert_eq!(
        resume.items[0]
            .user_data
            .as_ref()
            .unwrap()
            .playback_position_ticks,
        10 * 60 * TICKS_PER_SECOND
    );
}

#[tokio::test]
async fn playback_info_builds_direct_play_profile() {
    let (client, handle) = logged_in().await;
    let id = sample_movie_id(0);
    let user = client.user_id().unwrap();
    let response = client
        .playback_info(&id, &PlaybackInfoRequest::direct_play(user.clone()))
        .await
        .unwrap();
    assert!(response.play_session_id.is_some());
    let source = &response.media_sources[0];
    assert!(source.supports_direct_play);
    assert_eq!(
        source
            .media_streams
            .iter()
            .filter(|s| s.kind == "Audio")
            .count(),
        2
    );

    let Recorded::PlaybackInfo { item_id, body } = handle.recorded().remove(0) else {
        panic!("PlaybackInfo erwartet")
    };
    assert_eq!(item_id, id);
    assert_eq!(body["UserId"], user);
    assert_eq!(body["SubtitleStreamIndex"], -1);
    assert_eq!(body["EnableDirectPlay"], true);
    assert_eq!(body["EnableTranscoding"], false);
    assert!(body.get("AudioStreamIndex").is_none());
    let profile = &body["DeviceProfile"];
    assert_eq!(profile["DirectPlayProfiles"][0]["Type"], "Video");
    assert!(profile["DirectPlayProfiles"][0].get("Container").is_none());
    let subs = profile["SubtitleProfiles"].as_array().unwrap();
    assert!(subs.iter().all(|s| s["Method"] == "External"));
    assert!(
        subs.iter().any(|s| s["Format"] == "pgssub") && subs.iter().any(|s| s["Format"] == "ass")
    );
}

#[tokio::test]
async fn progress_reporting_is_recorded() {
    let (client, handle) = logged_in().await;
    let id = sample_movie_id(0);
    let ticks = 60 * TICKS_PER_SECOND;
    let mut report = PlaybackReport::direct(&id, "ms1", "sess1", 0);
    report.audio_stream_index = Some(1);
    client.report_playing(&report).await.unwrap();
    report.position_ticks = ticks;
    report.is_paused = true;
    client.report_progress(&report).await.unwrap();
    // Position nahe am Ende: gilt als gesehen.
    let stop = PlaybackStop {
        item_id: id.clone(),
        media_source_id: "ms1".into(),
        play_session_id: "sess1".into(),
        position_ticks: 118 * 60 * TICKS_PER_SECOND,
        failed: false,
    };
    client.report_stopped(&stop).await.unwrap();

    let rec = handle.recorded();
    assert_eq!(rec.len(), 3);
    let Recorded::Playing(p) = &rec[0] else {
        panic!()
    };
    assert_eq!(p["ItemId"], id);
    assert_eq!(p["PlayMethod"], "DirectPlay");
    assert_eq!(p["AudioStreamIndex"], 1);
    assert!(p.get("SubtitleStreamIndex").is_none());
    let Recorded::Progress(p) = &rec[1] else {
        panic!()
    };
    assert_eq!(p["PositionTicks"], ticks);
    assert_eq!(p["IsPaused"], true);
    let Recorded::Stopped(p) = &rec[2] else {
        panic!()
    };
    assert_eq!(p["PlaySessionId"], "sess1");
    assert_eq!(p["Failed"], false);

    let item = client.item(&id, &[]).await.unwrap();
    assert!(item.user_data.unwrap().played);
}

#[tokio::test]
async fn media_segments_and_images() {
    let (client, _h) = logged_in().await;
    let id = sample_movie_id(0);
    let segs = client.media_segments(&id).await.unwrap();
    assert_eq!(segs.items.len(), 2);
    assert_eq!(segs.items[0].kind, "Intro");
    assert_eq!(segs.items[0].start_ticks, 30 * TICKS_PER_SECOND);

    let url = client
        .image_url(
            &id,
            ImageType::Primary,
            Some("tag"),
            ImageOptions {
                max_width: Some(300),
                quality: Some(90),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(url.contains("/Items/") && url.contains("maxWidth=300") && url.contains("tag=tag"));
    let bytes = client.fetch_bytes(&url).await.unwrap();
    assert_eq!(&bytes[1..4], b"PNG");

    // Fremde Adressen bekommen nie ein Token.
    assert!(matches!(
        client
            .fetch_bytes("http://127.0.0.1:1/Items/x/Images/Primary")
            .await
            .unwrap_err(),
        ApiError::InvalidUrl(_)
    ));
    let stream = client.stream_url(&id, "ms1").unwrap();
    assert_eq!(
        &client.fetch_bytes(&stream).await.unwrap()[..],
        b"FAKE-VIDEO-BYTES"
    );
    assert!(
        client
            .trickplay_tile_url(&id, 320, 0, Some("ms1"))
            .unwrap()
            .contains("/Trickplay/320/0.jpg")
    );
}

#[tokio::test]
async fn retry_only_for_get() {
    let (client, handle) = logged_in().await;
    handle.clear_recorded();
    // GET: zwei Fehler, dritter Versuch klappt.
    handle.fail_next(2, 503);
    assert_eq!(client.views().await.unwrap().items.len(), 2);
    assert_eq!(
        handle
            .requests()
            .iter()
            .filter(|r| r.path == "/UserViews")
            .count(),
        3
    );

    // GET: dauerhafter Fehler wird nach 3 Versuchen gemeldet.
    handle.clear_recorded();
    handle.fail_next(10, 503);
    let err = client.views().await.unwrap_err();
    assert!(
        matches!(err, ApiError::Status { status: 503, .. }),
        "{err:?}"
    );
    assert_eq!(handle.requests().len(), 3);

    // POST: kein Wiederholen.
    handle.clear_recorded();
    handle.fail_next(1, 503);
    let report = PlaybackReport::direct(sample_movie_id(0), "ms", "s", 0);
    assert!(client.report_progress(&report).await.is_err());
    assert_eq!(handle.requests().len(), 1);

    // 404 wird nie wiederholt.
    handle.clear_recorded();
    handle.fail_next(0, 503);
    assert!(
        client
            .item("00000000000000000000000000000000", &[])
            .await
            .is_err()
    );
    assert_eq!(handle.requests().len(), 1);
}

#[tokio::test]
async fn timeout_is_reported_and_error_text_has_no_url() {
    let (addr, handle) = start().await;
    let options = ClientOptions {
        timeout: Duration::from_millis(100),
        retry: RetryPolicy {
            retries: 0,
            ..RetryPolicy::default()
        },
        ..fast_options()
    };
    let client = Client::with_options(&format!("http://{addr}"), info(), options).unwrap();
    handle.set_delay(Duration::from_millis(400));
    let err = client.public_info().await.unwrap_err();
    assert!(matches!(err, ApiError::Timeout), "{err:?}");

    handle.set_delay(Duration::ZERO);
    let dead = Client::with_options(
        "http://127.0.0.1:1",
        info(),
        ClientOptions {
            retry: RetryPolicy {
                retries: 0,
                ..RetryPolicy::default()
            },
            ..fast_options()
        },
    )
    .unwrap();
    let text = dead.public_info().await.unwrap_err().to_string();
    assert!(!text.contains("127.0.0.1"), "Adresse im Fehlertext: {text}");
}

#[tokio::test]
async fn discovery_against_local_responder() {
    use tokio::net::UdpSocket;
    let responder = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let target = responder.local_addr().unwrap();
    tokio::spawn(async move {
        let mut buf = [0u8; 64];
        let (len, from) = responder.recv_from(&mut buf).await.unwrap();
        assert_eq!(&buf[..len], b"who is JellyfinServer?");
        let reply = br#"{"Address":"http://192.0.2.10:8096","Id":"srv1","Name":"Beispiel","EndpointAddress":null}"#;
        // Doppelte Antwort und Müll: wird dedupliziert bzw. ignoriert.
        responder.send_to(reply, from).await.unwrap();
        responder.send_to(b"kein json", from).await.unwrap();
        responder.send_to(reply, from).await.unwrap();
    });
    let found = discover_at(target, Duration::from_millis(400))
        .await
        .unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "Beispiel");
    assert_eq!(found[0].address, "http://192.0.2.10:8096");
}
