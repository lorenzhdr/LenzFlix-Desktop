//! Mock-Integrationstests der erweiterten Methoden (kein echtes Netz, nur 127.0.0.1).

use lfx_api::{
    ApiError, AudioStreamOptions, ChartsClient, Client, ClientInfo, ClientOptions, ItemKind,
    ItemsQuery, PasswordChange, PlaybackInfoRequest, PlaystateCommand, ProgramsQuery, RetryPolicy,
    SearchHintOptions, StatsRange, UserDataUpdate, UserPolicy,
};
use lfx_mock_jellyfin::{
    DEMO_PASSWORD, DEMO_USER, MockHandle, VIEW_BOOKS, VIEW_PHOTOS, album_id, artist_id, book_id,
    channel_id, make_png, recording_id, sample_movie_id, sample_series_id, start, track_id,
};
use std::net::SocketAddr;
use std::time::Duration;

fn client_for(addr: SocketAddr) -> Client {
    Client::with_options(
        &format!("http://{addr}"),
        ClientInfo::new("Test-Rechner", "00000000-0000-4000-8000-000000000000"),
        ClientOptions {
            timeout: Duration::from_secs(5),
            connect_timeout: Duration::from_secs(2),
            retry: RetryPolicy {
                retries: 0,
                base_delay: Duration::from_millis(5),
            },
        },
    )
    .unwrap()
}

async fn logged_in() -> (Client, MockHandle) {
    let (addr, handle) = start().await;
    let client = client_for(addr);
    client.login(DEMO_USER, DEMO_PASSWORD).await.unwrap();
    (client, handle)
}

async fn admin() -> (Client, MockHandle) {
    let (client, handle) = logged_in().await;
    handle.set_demo_admin(true);
    (client, handle)
}

// ---------------------------------------------------------------- Server, Bilder, Branding

#[tokio::test]
async fn public_users_ping_and_png() {
    let (addr, handle) = start().await;
    let client = client_for(addr);
    // Ohne Anmeldung erreichbar.
    client.ping().await.unwrap();
    let users = client.public_users().await.unwrap();
    assert_eq!(users.len(), 2);
    assert_eq!(users[0].name, DEMO_USER);
    assert!(users[0].primary_image_tag.is_some());
    assert!(users[1].primary_image_tag.is_none());
    drop(handle);

    // Bildantwort ist ein gültiges PNG (Signatur, Blöcke, Prüfsummen).
    let (client, _h) = logged_in().await;
    let id = sample_movie_id(0);
    let url = client
        .image_url(&id, lfx_api::ImageType::Primary, None, Default::default())
        .unwrap();
    let png = client.fetch_bytes(&url).await.unwrap();
    assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
    assert_eq!(&png[png.len() - 8..png.len() - 4], b"IEND");
    assert_eq!(png.to_vec(), make_png(24, 36));
}

#[tokio::test]
async fn delete_active_encodings_sends_device_and_session() {
    let (client, handle) = logged_in().await;
    client.delete_active_encodings("play-xyz").await.unwrap();
    let got = handle.encodings_deleted();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].0, "00000000-0000-4000-8000-000000000000");
    assert_eq!(got[0].1, "play-xyz");
}

#[tokio::test]
async fn branding_is_parsed_from_css() {
    let (addr, handle) = start().await;
    let client = client_for(addr);
    let b = client.branding().await.unwrap();
    assert_eq!(b.charts_url.as_deref(), Some(&*format!("http://{addr}/cw")));
    assert_eq!(b.stats_url.as_deref(), Some("/stats"));
    assert_eq!(b.tv_channels.get(&5).map(String::as_str), Some("Action"));
    assert!(
        client
            .branding_css()
            .await
            .unwrap()
            .contains("Mock-Branding")
    );
    assert!(
        client
            .branding_options()
            .await
            .unwrap()
            .login_disclaimer
            .is_some()
    );
    handle.set_branding_css(None);
    assert_eq!(client.branding().await.unwrap(), Default::default());
}

// ---------------------------------------------------------------- Charts-Worker

#[tokio::test]
async fn charts_client_charts_and_stats() {
    let (client, handle) = logged_in().await;
    let b = client.branding().await.unwrap();
    let charts = ChartsClient::from_branding(&client, &b).unwrap().unwrap();
    let c = charts.charts().await.unwrap();
    assert_eq!(c.window_days, 30);
    assert_eq!(c.movies.len(), 3);
    assert_eq!(c.movies[0].seconds, 10_800);
    assert_eq!(c.all[0].kind.as_deref(), Some("Movie"));

    let s = charts.my_stats(StatsRange::Year).await.unwrap();
    assert_eq!(s.range, "year");
    assert_eq!(s.timeline.unit, "week");
    assert_eq!(s.totals.active_days, 2);
    assert_eq!(s.top_series[0].id, sample_series_id(0));
    assert_eq!(s.methods.direct_play.plays, 2);
    assert_eq!(s.compare.unwrap().of, 4);
    assert_eq!(
        charts
            .my_stats(StatsRange::All)
            .await
            .unwrap()
            .timeline
            .unit,
        "month"
    );

    // Falsches Token → 401; ausgeschalteter Worker → 404.
    let bad = ChartsClient::new(&b.charts_url.clone().unwrap(), "falsch".into()).unwrap();
    assert!(matches!(
        bad.charts().await.unwrap_err(),
        ApiError::Unauthorized
    ));
    handle.set_charts_enabled(false);
    assert!(matches!(
        charts.charts().await.unwrap_err(),
        ApiError::NotFound
    ));

    // Ohne Charts-URL oder Sitzung gibt es keinen Client.
    assert!(ChartsClient::from_branding(&client, &Default::default()).is_none());
}

#[tokio::test]
async fn charts_token_goes_only_in_header() {
    let (client, handle) = logged_in().await;
    let b = client.branding().await.unwrap();
    let charts = ChartsClient::from_branding(&client, &b).unwrap().unwrap();
    charts.my_stats(StatsRange::Days7).await.unwrap();
    let log = handle.requests();
    let last = log.last().unwrap();
    assert_eq!(last.path, "/cw/me/stats");
    let text = format!("{charts:?} {last:?}");
    for t in handle.issued_tokens() {
        assert!(!text.contains(&t));
    }
}

// ---------------------------------------------------------------- Suche, Personen, Filter

#[tokio::test]
async fn search_hints_cover_media_people_genres() {
    let (client, _h) = logged_in().await;
    let hits = client
        .search_hints(
            "a",
            &SearchHintOptions {
                limit: 100,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let kinds: Vec<&str> = hits.search_hints.iter().map(|h| h.kind.as_str()).collect();
    for k in [
        "Movie",
        "Series",
        "Person",
        "Genre",
        "Studio",
        "MusicArtist",
    ] {
        assert!(kinds.contains(&k), "{k} fehlt in {kinds:?}");
    }
    assert_eq!(hits.total_record_count, hits.search_hints.len() as i64);

    let only_people = client
        .search_hints(
            "mira",
            &SearchHintOptions {
                include_media: false,
                include_genres: false,
                include_studios: false,
                include_artists: false,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(only_people.search_hints.len(), 1);
    assert_eq!(only_people.search_hints[0].name, "Mira Kessler");

    let limited = client
        .search_hints(
            "",
            &SearchHintOptions {
                limit: 3,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(limited.search_hints.len(), 3);
}

#[tokio::test]
async fn persons_genres_studios_filters_counts() {
    let (client, _h) = logged_in().await;
    let p = client
        .persons(&ItemsQuery::new().search_term("voigt"))
        .await
        .unwrap();
    assert_eq!(p.items.len(), 1);
    assert_eq!(p.items[0].kind, ItemKind::Person);
    let all = client
        .persons(&ItemsQuery::new().name_starts_with("J"))
        .await
        .unwrap();
    assert_eq!(all.items[0].name, "Jonas Albrecht");

    let genres = client
        .genres_filtered(&ItemsQuery::new().parent_id("x"))
        .await
        .unwrap();
    assert!(genres.items.iter().any(|g| g.name == "Drama"));
    let studios = client.studios(&ItemsQuery::new()).await.unwrap();
    assert_eq!(studios.items.len(), 2);
    assert_eq!(studios.items[0].kind, ItemKind::Studio);
    let mg = client.music_genres(&ItemsQuery::new()).await.unwrap();
    assert_eq!(mg.items[0].kind, ItemKind::MusicGenre);

    let f = client
        .item_filters(None, &[ItemKind::Movie], true)
        .await
        .unwrap();
    assert_eq!(f.genres.len(), 3);
    assert_eq!(f.audio_languages, ["ger", "eng"]);

    let counts = client.item_counts(false).await.unwrap();
    assert_eq!(counts.movie_count, 12);
    assert_eq!(counts.series_count, 2);
    assert_eq!(counts.artist_count, 2);
    assert_eq!(counts.song_count, 10);
    assert_eq!(counts.album_count, 3);
    assert_eq!(counts.book_count, 3);
}

#[tokio::test]
async fn extras_trailers_intros() {
    let (client, _h) = logged_in().await;
    let features = client.special_features(&sample_movie_id(0)).await.unwrap();
    assert_eq!(features.len(), 1);
    assert!(
        client
            .special_features(&sample_series_id(0))
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        client
            .local_trailers(&sample_movie_id(0))
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        client
            .intros(&sample_movie_id(0))
            .await
            .unwrap()
            .items
            .is_empty()
    );
}

#[tokio::test]
async fn user_data_update_and_latest_per_library() {
    let (client, _h) = logged_in().await;
    let id = sample_movie_id(1);
    let d = client
        .update_user_data(
            &id,
            &UserDataUpdate {
                playback_position_ticks: Some(1_200_000_000),
                is_favorite: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(d.playback_position_ticks, 1_200_000_000);
    assert!(d.is_favorite);
    // „Latest je Bibliothek“: vorhandene Methode mit parent_id.
    let latest = client
        .latest(Some("00000000000000000000000000500001"), &[], 5, &[])
        .await
        .unwrap();
    assert!(!latest.is_empty());
    assert!(latest.iter().all(|i| i.kind == ItemKind::Movie));
}

// ---------------------------------------------------------------- Musik

#[tokio::test]
async fn music_artists_albums_tracks_mix_and_stream() {
    let (client, handle) = logged_in().await;
    handle.set_media_views(true);
    let views = client.views().await.unwrap();
    let names: Vec<&str> = views.items.iter().map(|v| v.name.as_str()).collect();
    for n in ["Musik", "Fotos", "Bücher", "Live-TV"] {
        assert!(names.contains(&n), "{n} fehlt in {names:?}");
    }
    assert_eq!(
        views
            .items
            .iter()
            .find(|v| v.name == "Musik")
            .unwrap()
            .collection_type
            .as_deref(),
        Some("music")
    );

    let artists = client.artists(&ItemsQuery::new()).await.unwrap();
    assert_eq!(artists.items.len(), 2);
    assert_eq!(artists.items[0].kind, ItemKind::MusicArtist);
    assert_eq!(artists.items[0].album_count, Some(2));
    let aa = client
        .album_artists(&ItemsQuery::new().search_term("nord"))
        .await
        .unwrap();
    assert_eq!(aa.items.len(), 1);

    let all_albums = client.albums(None, None, 50, 0).await.unwrap();
    assert_eq!(all_albums.total_record_count, 3);
    let of_artist = client
        .albums(Some(&artist_id(0)), None, 50, 0)
        .await
        .unwrap();
    assert_eq!(of_artist.items.len(), 2);
    assert!(
        of_artist
            .items
            .iter()
            .all(|a| a.album_artist.as_deref() == Some("Nora Tiefenbach"))
    );
    assert_eq!(of_artist.items[0].album_artists[0].id, artist_id(0));

    let tracks = client.album_tracks(&album_id(0)).await.unwrap();
    assert_eq!(tracks.items.len(), 4);
    assert_eq!(
        tracks
            .items
            .iter()
            .map(|t| t.index_number)
            .collect::<Vec<_>>(),
        [Some(1), Some(2), Some(3), Some(4)]
    );
    assert_eq!(tracks.items[0].album.as_deref(), Some("Leuchtfeuer"));
    assert_eq!(tracks.items[0].artists, ["Nora Tiefenbach"]);
    assert_eq!(
        tracks.items[0].media_sources[0].media_streams[0]
            .codec
            .as_deref(),
        Some("flac")
    );

    let searched = client
        .tracks(&ItemsQuery::new().search_term("flut"))
        .await
        .unwrap();
    assert_eq!(searched.items.len(), 1);

    let mix = client.instant_mix(&album_id(0), 5).await.unwrap();
    assert_eq!(mix.items.len(), 5);
    assert_eq!(mix.items[0].kind, ItemKind::Audio);

    let url = client
        .audio_stream_url(&track_id(0), &AudioStreamOptions::default())
        .unwrap();
    assert!(url.contains("/Audio/") && url.contains("/universal?"));
    assert!(url.contains("userId=") && url.contains("deviceId="));
    for t in handle.issued_tokens() {
        assert!(!url.contains(&t));
    }
    assert!(!url.to_lowercase().contains("api_key") && !url.contains("ApiKey"));
    assert_eq!(
        &client.fetch_bytes(&url).await.unwrap()[..],
        b"FAKE-AUDIO-BYTES"
    );
}

// ---------------------------------------------------------------- Playlists

#[tokio::test]
async fn playlists_create_add_remove_move() {
    let (client, handle) = logged_in().await;
    let t: Vec<String> = (0..4).map(track_id).collect();
    let created = client
        .create_playlist("Meine Liste", &[&t[0], &t[1]], Some("Audio"))
        .await
        .unwrap();
    assert_eq!(created.id.len(), 32);

    client
        .playlist_add(&created.id, &[&t[2], &t[3]], None)
        .await
        .unwrap();
    let items = client
        .playlist_items(&created.id, 50, 0, &[])
        .await
        .unwrap();
    assert_eq!(items.items.len(), 4);
    assert_eq!(items.items[2].name, "Nachtfähre");
    let entry: Vec<String> = items
        .items
        .iter()
        .map(|i| i.playlist_item_id.clone().unwrap())
        .collect();

    // Letzten Eintrag nach vorn schieben.
    client
        .playlist_move(&created.id, &entry[3], 0)
        .await
        .unwrap();
    assert_eq!(
        handle.playlists()[0].1,
        [t[3].clone(), t[0].clone(), t[1].clone(), t[2].clone()]
    );
    // Eintrag per PlaylistItemId entfernen (nicht per Element-Id).
    client
        .playlist_remove(&created.id, &[&entry[0]])
        .await
        .unwrap();
    assert_eq!(
        handle.playlists()[0].1,
        [t[3].clone(), t[1].clone(), t[2].clone()]
    );
    // Einfügen an Position.
    client
        .playlist_add(&created.id, &[&t[0]], Some(1))
        .await
        .unwrap();
    assert_eq!(handle.playlists()[0].1[1], t[0]);

    let dto = client.playlist(&created.id).await.unwrap();
    assert_eq!(dto.item_ids.len(), 4);
    client
        .rename_playlist(&created.id, "Umbenannt")
        .await
        .unwrap();
    assert_eq!(handle.playlists()[0].0, "Umbenannt");

    // Wiedergabelisten stehen auch unter /Items.
    let lists = client
        .items(
            &ItemsQuery::new()
                .include_types(&[ItemKind::Playlist])
                .recursive(true),
        )
        .await
        .unwrap();
    assert_eq!(lists.items.len(), 1);
    assert_eq!(lists.items[0].name, "Umbenannt");

    client.delete_item(&created.id).await.unwrap();
    assert!(handle.playlists().is_empty());
    assert!(matches!(
        client.delete_item(&created.id).await.unwrap_err(),
        ApiError::NotFound
    ));
}

// ---------------------------------------------------------------- Fotos, Bücher

#[tokio::test]
async fn photos_and_books() {
    let (client, _h) = logged_in().await;
    let root = client.photo_items(Some(VIEW_PHOTOS), 50, 0).await.unwrap();
    // Album zuerst, dann die zwei losen Fotos.
    assert_eq!(root.items[0].kind, ItemKind::PhotoAlbum);
    assert_eq!(root.items.len(), 3);
    let album = client
        .photo_items(Some(&root.items[0].id), 50, 0)
        .await
        .unwrap();
    assert_eq!(album.items.len(), 3);
    assert!(
        album
            .items
            .iter()
            .all(|p| p.kind == ItemKind::Photo && p.width == Some(4000))
    );
    let everything = client.photo_items(None, 50, 0).await.unwrap();
    assert_eq!(everything.items.len(), 6);

    let photo_url = client.download_url(&album.items[0].id).unwrap();
    assert!(photo_url.ends_with("/Download"));
    let bytes = client.fetch_bytes(&photo_url).await.unwrap();
    assert_eq!(&bytes[1..4], b"PNG");

    let books = client.books(Some(VIEW_BOOKS), 50, 0).await.unwrap();
    assert_eq!(books.items.len(), 3);
    assert_eq!(books.items[0].kind, ItemKind::Book);
    assert_eq!(books.items[0].container.as_deref(), Some("epub"));
    let url = client.download_url(&book_id(0)).unwrap();
    assert_eq!(
        &client.fetch_bytes(&url).await.unwrap()[..],
        b"FAKE-BOOK-BYTES"
    );

    // Lesefortschritt.
    let d = client
        .update_user_data(
            &book_id(0),
            &UserDataUpdate {
                playback_position_ticks: Some(42),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(d.playback_position_ticks, 42);
    let again = client.item(&book_id(0), &[]).await.unwrap();
    assert_eq!(again.user_data.unwrap().playback_position_ticks, 42);
}

// ---------------------------------------------------------------- Live-TV

#[tokio::test]
async fn live_tv_channels_guide_timers_recordings_stream() {
    let (client, handle) = logged_in().await;
    let ch = client.live_channels(true, 50, 0).await.unwrap();
    assert_eq!(ch.items.len(), 4);
    assert_eq!(ch.items[0].name, "Kanal 1");
    assert_eq!(ch.items[0].kind, ItemKind::TvChannel);
    let current = ch.items[0].current_program.as_ref().unwrap();
    assert!(!current.name.is_empty());
    assert_eq!(current.channel_id.as_deref(), Some(channel_id(0).as_str()));
    let plain = client.live_channels(false, 50, 0).await.unwrap();
    assert!(plain.items[0].current_program.is_none());

    // Sendeplan mit Zeitraum: nur Kanal 2, nur laufende Sendungen.
    let airing = client
        .live_programs(&ProgramsQuery {
            channel_ids: vec![channel_id(1)],
            is_airing: Some(true),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(airing.items.len(), 1);
    assert_eq!(airing.items[0].channel_name.as_deref(), Some("Kanal 2"));
    let guide = client
        .live_programs(&ProgramsQuery {
            channel_ids: vec![channel_id(0), channel_id(2)],
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(guide.items.len() >= 20);
    assert!(
        guide
            .items
            .windows(2)
            .all(|w| w[0].start_date <= w[1].start_date)
    );
    let start = guide.items[5].start_date.clone().unwrap();
    let narrowed = client
        .live_programs(&ProgramsQuery {
            channel_ids: vec![channel_id(0)],
            max_start_date: Some(start.clone()),
            limit: Some(100),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(
        narrowed
            .items
            .iter()
            .all(|p| p.start_date.as_deref() <= Some(start.as_str()))
    );

    // Timer anlegen und löschen.
    let program = airing.items[0].id.clone();
    let defaults = client.timer_defaults(&program).await.unwrap();
    assert_eq!(defaults["ProgramId"], program);
    client.record_program(&program).await.unwrap();
    let timers = client.timers().await.unwrap();
    assert_eq!(timers.items.len(), 1);
    assert_eq!(
        timers.items[0].program_id.as_deref(),
        Some(program.as_str())
    );
    assert_eq!(timers.items[0].status.as_deref(), Some("New"));
    assert_eq!(timers.items[0].pre_padding_seconds, 60);
    assert_eq!(handle.timers().len(), 1);
    client.cancel_timer(&timers.items[0].id).await.unwrap();
    assert!(client.timers().await.unwrap().items.is_empty());
    assert!(matches!(
        client.cancel_timer("nix").await.unwrap_err(),
        ApiError::NotFound
    ));

    // Aufnahmen.
    let rec = client.recordings(50, 0).await.unwrap();
    assert_eq!(rec.items.len(), 2);
    assert_eq!(rec.items[0].kind, ItemKind::Recording);
    client.delete_recording(&recording_id(0)).await.unwrap();
    assert_eq!(client.recordings(50, 0).await.unwrap().items.len(), 1);

    // Live-Stream öffnen und schließen.
    let request = PlaybackInfoRequest::direct_play(client.user_id().unwrap());
    let info = client
        .open_live_stream(&channel_id(0), &request)
        .await
        .unwrap();
    let source = &info.media_sources[0];
    assert!(source.is_infinite_stream && source.requires_closing);
    let live = source.live_stream_id.clone().unwrap();
    assert_eq!(
        handle.open_live_streams().as_slice(),
        std::slice::from_ref(&live)
    );
    client.close_live_stream(&live).await.unwrap();
    assert!(handle.open_live_streams().is_empty());
}

// ---------------------------------------------------------------- Benutzer-Einstellungen

#[tokio::test]
async fn user_configuration_roundtrip_keeps_unknown_fields() {
    let (client, _h) = logged_in().await;
    let mut cfg = client.user_configuration().await.unwrap();
    assert!(cfg.play_default_audio_track);
    cfg.audio_language_preference = Some("ger".into());
    cfg.subtitle_language_preference = Some("eng".into());
    cfg.subtitle_mode = Some("OnlyForced".into());
    cfg.extra
        .insert("GroupedFolders".into(), serde_json::json!(["a"]));
    client.update_user_configuration(&cfg).await.unwrap();
    let back = client.user_configuration().await.unwrap();
    assert_eq!(back.audio_language_preference.as_deref(), Some("ger"));
    assert_eq!(back.subtitle_mode.as_deref(), Some("OnlyForced"));
    assert_eq!(back.extra["GroupedFolders"], serde_json::json!(["a"]));
}

#[tokio::test]
async fn password_change_and_profile_image() {
    let (client, handle) = logged_in().await;
    let err = client
        .change_password(&PasswordChange {
            current_pw: "falsch".into(),
            new_pw: "neu".into(),
        })
        .await
        .unwrap_err();
    assert!(matches!(err, ApiError::Forbidden));
    client
        .change_password(&PasswordChange {
            current_pw: DEMO_PASSWORD.into(),
            new_pw: "neu".into(),
        })
        .await
        .unwrap();
    // Zweiter Versuch mit dem alten Passwort scheitert jetzt.
    assert!(
        client
            .change_password(&PasswordChange {
                current_pw: DEMO_PASSWORD.into(),
                new_pw: "x".into(),
            })
            .await
            .is_err()
    );

    // Kein Bild → Fehler vor dem Senden; PNG → Base64 mit Bild-Inhaltstyp.
    assert!(client.upload_profile_image(b"kein bild").await.is_err());
    let png = make_png(8, 8);
    client.upload_profile_image(&png).await.unwrap();
    let (ctype, len) = handle.avatar().unwrap();
    assert_eq!(ctype, "image/png");
    assert_eq!(len, png.len().div_ceil(3) * 4);
    client.delete_profile_image().await.unwrap();
    assert!(handle.avatar().is_none());
    let url = client.profile_image_url("abc", Some("tag")).unwrap();
    assert!(url.contains("/UserImage?userId=abc&tag=tag"));
}

#[tokio::test]
async fn quick_connect_authorize_for_other_device() {
    let (client, handle) = logged_in().await;
    let other = Client::new(
        client.base_url(),
        ClientInfo::new("Fernseher", "11111111-1111-4111-8111-111111111111"),
    )
    .unwrap();
    let qc = other.quick_connect_initiate().await.unwrap();
    assert!(client.quick_connect_authorize(&qc.code).await.unwrap());
    assert!(
        other
            .quick_connect_poll(&qc.secret)
            .await
            .unwrap()
            .authenticated
    );
    drop(handle);
}

// ---------------------------------------------------------------- Verwaltung

#[tokio::test]
async fn admin_endpoints_need_admin() {
    let (client, _h) = logged_in().await;
    assert!(matches!(
        client.system_info().await.unwrap_err(),
        ApiError::Forbidden
    ));
    assert!(matches!(
        client.sessions(None).await.unwrap_err(),
        ApiError::Forbidden
    ));
    assert!(matches!(
        client.users().await.unwrap_err(),
        ApiError::Forbidden
    ));
    assert!(matches!(
        client.refresh_library().await.unwrap_err(),
        ApiError::Forbidden
    ));
    assert!(matches!(
        client.restart_server().await.unwrap_err(),
        ApiError::Forbidden
    ));
    assert!(!client.me().await.unwrap().policy.unwrap().is_administrator);
}

#[tokio::test]
async fn admin_system_sessions_tasks_libraries() {
    let (client, handle) = admin().await;
    assert!(client.me().await.unwrap().policy.unwrap().is_administrator);
    let info = client.system_info().await.unwrap();
    assert_eq!(info.version, "12.4.0");
    assert!(info.can_self_restart);

    let sessions = client.sessions(Some(960)).await.unwrap();
    assert_eq!(sessions.len(), 2);
    let tv = sessions.iter().find(|s| s.id == "sess-tv").unwrap();
    assert_eq!(tv.now_playing_item.as_ref().unwrap().kind, ItemKind::Movie);
    assert_eq!(tv.play_state.as_ref().unwrap().volume_level, Some(40));
    client
        .session_message("sess-tv", "Hallo", "Kurze Nachricht", 3000)
        .await
        .unwrap();
    client
        .session_playstate("sess-tv", PlaystateCommand::Pause, None)
        .await
        .unwrap();
    client
        .session_playstate("sess-tv", PlaystateCommand::Seek, Some(99))
        .await
        .unwrap();
    client.session_command("sess-tv", "GoHome").await.unwrap();
    assert!(matches!(
        client
            .session_message("nix", "a", "b", 1)
            .await
            .unwrap_err(),
        ApiError::NotFound
    ));
    assert_eq!(
        handle.admin_events(),
        [
            "Nachricht:sess-tv:Kurze Nachricht",
            "Steuerung:sess-tv:Pause",
            "Steuerung:sess-tv:Seek:99",
            "Befehl:sess-tv:GoHome"
        ]
    );

    let tasks = client.scheduled_tasks(false).await.unwrap();
    assert_eq!(tasks.len(), 3);
    assert!(tasks.iter().all(|t| t.state == "Idle"));
    client.start_task("task-scan").await.unwrap();
    let running = client.scheduled_tasks(true).await.unwrap();
    let scan = running.iter().find(|t| t.id == "task-scan").unwrap();
    assert_eq!(scan.state, "Running");
    assert_eq!(scan.current_progress_percentage, Some(42.0));
    client.stop_task("task-scan").await.unwrap();
    assert!(
        client
            .scheduled_tasks(true)
            .await
            .unwrap()
            .iter()
            .all(|t| t.state == "Idle")
    );
    assert!(matches!(
        client.start_task("nix").await.unwrap_err(),
        ApiError::NotFound
    ));

    client.refresh_library().await.unwrap();
    let folders = client.virtual_folders().await.unwrap();
    assert_eq!(folders.len(), 2);
    client
        .add_virtual_folder("Musik", "music", &["/mock/musik"], true)
        .await
        .unwrap();
    let folders = client.virtual_folders().await.unwrap();
    assert_eq!(folders[2].name, "Musik");
    assert_eq!(folders[2].locations, ["/mock/musik"]);
    assert!(matches!(
        client
            .add_virtual_folder("Musik", "music", &[], false)
            .await
            .unwrap_err(),
        ApiError::Status { status: 409, .. }
    ));
    client.remove_virtual_folder("Musik", false).await.unwrap();
    assert_eq!(client.virtual_folders().await.unwrap().len(), 2);

    client.restart_server().await.unwrap();
    client.shutdown_server().await.unwrap();
    let events = handle.admin_events();
    assert!(
        events.contains(&"Neustart".to_owned()) && events.contains(&"Herunterfahren".to_owned())
    );
}

#[tokio::test]
async fn admin_users_policy_logs_devices() {
    let (client, _h) = admin().await;
    let users = client.users().await.unwrap();
    assert_eq!(users.len(), 2);
    let created = client.create_user("neuling", "geheim").await.unwrap();
    assert_eq!(created.name, "neuling");
    assert!(client.create_user("neuling", "x").await.is_err());

    let mut user = client.user(&created.id).await.unwrap();
    let mut policy: UserPolicy = user.policy.take().unwrap();
    assert!(!policy.is_disabled && policy.enable_media_playback);
    policy.is_disabled = true;
    policy.enable_live_tv_management = true;
    client
        .update_user_policy(&created.id, &policy)
        .await
        .unwrap();
    let again = client.user(&created.id).await.unwrap().policy.unwrap();
    assert!(again.is_disabled && again.enable_live_tv_management);
    // Unbekannte Felder der Richtlinie bleiben beim Rückschreiben erhalten.
    assert_eq!(again.extra["AuthenticationProviderId"], "Mock.Auth");

    client.delete_user(&created.id).await.unwrap();
    assert_eq!(client.users().await.unwrap().len(), 2);
    assert!(matches!(
        client.user(&created.id).await.unwrap_err(),
        ApiError::NotFound
    ));

    let log = client.activity_log(5, 0, None).await.unwrap();
    assert_eq!(log.items.len(), 5);
    assert_eq!(log.total_record_count, 12);
    assert_eq!(log.items[0].id, 12);
    let page2 = client.activity_log(5, 5, None).await.unwrap();
    assert_eq!(page2.items[0].id, 7);

    let files = client.logs().await.unwrap();
    assert_eq!(files.len(), 2);
    let text = client.log_file(&files[0].name).await.unwrap();
    assert!(text.contains("Erfundener Logeintrag"));
    assert!(matches!(
        client.log_file("../x").await.unwrap_err(),
        ApiError::NotFound
    ));

    let devices = client.devices().await.unwrap();
    assert_eq!(devices.items.len(), 2);
    client.delete_device("dev-tv").await.unwrap();
    assert_eq!(client.devices().await.unwrap().items.len(), 1);
}
