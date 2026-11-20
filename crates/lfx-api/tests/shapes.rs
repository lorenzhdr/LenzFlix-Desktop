#![recursion_limit = "512"]
//! Deserialisierung echter Antwortformen: selbst geschriebene Beispiele mit erfundenen Werten,
//! aufgebaut nach den Schemata der OpenAPI-Spec (Jellyfin 12.4).

use lfx_api::{
    ActivityLogEntry, BaseItem, ItemCounts, ItemKind, LogFile, MediaStream, QueryFilters,
    QueryResult, SearchHintResult, SessionInfo, SystemInfo, TaskInfo, TimerInfo, UserConfiguration,
    UserDto, UserPolicy, VirtualFolder,
};
use serde_json::json;

fn movie_json() -> serde_json::Value {
    json!({
        "Name": "Beispielfilm", "OriginalTitle": "Example Film", "ServerId": "s1", "Id": "m1",
        "Etag": "e", "DateCreated": "2026-01-01T00:00:00.0000000Z", "CanDelete": false, "CanDownload": true,
        "HasSubtitles": true, "Container": "mkv", "SortName": "beispielfilm",
        "PremiereDate": "2020-05-01T00:00:00.0000000Z", "CriticRating": 71.0,
        "OfficialRating": "FSK 12", "Overview": "Erfundene Handlung.", "Taglines": ["Ein Satz"],
        "Genres": ["Drama"], "CommunityRating": 7.5, "RunTimeTicks": 72_000_000_000i64,
        "ProductionYear": 2020, "IsFolder": false, "ParentId": "p1", "Type": "Movie",
        "People": [{ "Name": "Erika Muster", "Id": "pe1", "Role": "Hauptrolle", "Type": "Actor",
                     "PrimaryImageTag": "t1" }],
        "Studios": [{ "Name": "Studio Nordlicht", "Id": "st1" }],
        "GenreItems": [{ "Name": "Drama", "Id": "g1" }],
        "ParentLogoItemId": "p1", "ParentBackdropItemId": "p1", "ParentBackdropImageTags": ["b1"],
        "LocalTrailerCount": 1, "SpecialFeatureCount": 2,
        "RemoteTrailers": [{ "Url": "https://example.invalid/trailer", "Name": "Trailer" }],
        "UserData": { "PlaybackPositionTicks": 100, "PlayCount": 2, "IsFavorite": true, "Played": false,
                      "Key": "k", "ItemId": "m1", "LastPlayedDate": "2026-01-02T00:00:00.0000000Z" },
        "ChildCount": 0, "PrimaryImageAspectRatio": 0.6667,
        "ImageTags": { "Primary": "a", "Logo": "b" }, "BackdropImageTags": ["c"],
        "ImageBlurHashes": { "Primary": { "a": "LKO2?U%2Tw=w]~RBVZRi};RPxuwH" } },
        "MediaType": "Video", "Width": 1920, "Height": 1080,
        "MediaSources": [{
            "Protocol": "File", "Id": "ms1", "Type": "Default", "Container": "mkv", "Size": 5_000_000_000i64,
            "Name": "Beispielfilm", "IsRemote": false, "RunTimeTicks": 72_000_000_000i64,
            "SupportsTranscoding": true, "SupportsDirectStream": true, "SupportsDirectPlay": true,
            "RequiresOpening": false, "RequiresClosing": false, "VideoType": "VideoFile",
            "RequiredHttpHeaders": {}, "Bitrate": 9_000_000, "DefaultAudioStreamIndex": 1,
            "MediaStreams": [
                { "Codec": "hevc", "CodecTag": "hvc1", "ColorSpace": "bt2020nc", "ColorTransfer": "smpte2084",
                  "ColorPrimaries": "bt2020", "ColorRange": "tv", "VideoRange": "HDR", "VideoRangeType": "HDR10",
                  "DvProfile": 8, "DvLevel": 6, "DvBlSignalCompatibilityId": 1, "Hdr10PlusPresentFlag": false,
                  "BitDepth": 10, "IsInterlaced": false, "Height": 2160, "Width": 3840, "AverageFrameRate": 23.976,
                  "PixelFormat": "yuv420p10le", "Level": 153.0, "Type": "Video", "Index": 0, "IsDefault": true,
                  "AspectRatio": "16:9", "Profile": "Main 10" },
                { "Codec": "eac3", "Language": "ger", "ChannelLayout": "5.1", "Channels": 6, "SampleRate": 48000,
                  "BitRate": 640000, "IsDefault": true, "Type": "Audio", "Index": 1, "DisplayTitle": "Deutsch" },
                { "Codec": "srt", "Language": "eng", "IsExternal": true, "IsTextSubtitleStream": true,
                  "DeliveryMethod": "External", "DeliveryUrl": "/Videos/m1/ms1/Subtitles/2/0/Stream.srt",
                  "SupportsExternalStream": true, "IsHearingImpaired": true, "Type": "Subtitle", "Index": 2 }
            ]
        }],
        "Chapters": [{ "StartPositionTicks": 0, "Name": "Anfang", "ImageTag": "ct" }],
        "Trickplay": { "ms1": { "320": { "Width": 320, "Height": 180, "TileWidth": 10, "TileHeight": 10,
                                          "ThumbnailCount": 400, "Interval": 10000, "Bandwidth": 100000 } } },
        "LocationType": "FileSystem", "ProviderIds": { "Tmdb": "1" }, "Tags": ["t"],
        "UnbekanntesNeuesFeld": { "x": 1 }
    })
}

#[test]
fn movie_with_all_fields() {
    let item: BaseItem = serde_json::from_value(movie_json()).unwrap();
    assert_eq!(item.kind, ItemKind::Movie);
    assert_eq!(item.taglines, ["Ein Satz"]);
    assert_eq!(item.genre_items[0].name, "Drama");
    assert_eq!(item.studios[0].name, "Studio Nordlicht");
    assert_eq!(item.people[0].role.as_deref(), Some("Hauptrolle"));
    assert_eq!(item.people[0].primary_image_tag.as_deref(), Some("t1"));
    assert_eq!(item.local_trailer_count, 1);
    assert_eq!(item.special_feature_count, Some(2));
    assert_eq!(item.remote_trailers[0].name.as_deref(), Some("Trailer"));
    assert_eq!(item.critic_rating, Some(71.0));
    assert_eq!(item.image_blur_hashes["Primary"]["a"].len(), 28);
    assert_eq!(item.parent_backdrop_image_tags, ["b1"]);
    assert_eq!(item.user_data.as_ref().unwrap().play_count, 2);
    assert_eq!(item.chapters[0].image_tag.as_deref(), Some("ct"));
    assert_eq!(item.trickplay.unwrap()["ms1"]["320"].thumbnail_count, 400);

    let src = &item.media_sources[0];
    assert_eq!(src.kind.as_deref(), Some("Default"));
    let video = &src.media_streams[0];
    assert!(video.is_hdr());
    assert_eq!(video.video_range_type.as_deref(), Some("HDR10"));
    assert_eq!(video.color_primaries.as_deref(), Some("bt2020"));
    assert_eq!(video.color_transfer.as_deref(), Some("smpte2084"));
    assert_eq!(video.color_space.as_deref(), Some("bt2020nc"));
    assert_eq!(video.dv_profile, Some(8));
    assert_eq!(video.pixel_format.as_deref(), Some("yuv420p10le"));
    let audio = &src.media_streams[1];
    assert_eq!(audio.channel_layout.as_deref(), Some("5.1"));
    assert!(!audio.is_hdr());
    let sub = &src.media_streams[2];
    assert!(sub.is_external && sub.is_hearing_impaired);
    assert_eq!(sub.delivery_method.as_deref(), Some("External"));
    assert!(sub.delivery_url.as_deref().unwrap().ends_with("Stream.srt"));
}

#[test]
fn sdr_video_is_not_hdr() {
    let s: MediaStream = serde_json::from_value(
        json!({ "Type": "Video", "VideoRange": "SDR", "VideoRangeType": "SDR", "Index": 0 }),
    )
    .unwrap();
    assert!(!s.is_hdr());
    let dovi: MediaStream = serde_json::from_value(
        json!({ "Type": "Video", "VideoRange": "HDR", "VideoRangeType": "DOVIWithHDR10", "Index": 0 }),
    )
    .unwrap();
    assert!(dovi.is_hdr());
}

#[test]
fn unknown_kind_falls_back() {
    let item: BaseItem =
        serde_json::from_value(json!({ "Id": "x", "Name": "n", "Type": "ZukunftsTyp" })).unwrap();
    assert_eq!(item.kind, ItemKind::Unknown);
    for (name, kind) in [
        ("MusicAlbum", ItemKind::MusicAlbum),
        ("PhotoAlbum", ItemKind::PhotoAlbum),
        ("Photo", ItemKind::Photo),
        ("Book", ItemKind::Book),
        ("TvChannel", ItemKind::TvChannel),
        ("Program", ItemKind::Program),
        ("Recording", ItemKind::Recording),
        ("Person", ItemKind::Person),
        ("Studio", ItemKind::Studio),
        ("Playlist", ItemKind::Playlist),
    ] {
        let k: ItemKind = serde_json::from_value(json!(name)).unwrap();
        assert_eq!(k, kind);
        assert_eq!(kind.as_str(), name);
    }
}

#[test]
fn music_track_and_album() {
    let track: BaseItem = serde_json::from_value(json!({
        "Id": "t1", "Name": "Erste Welle", "Type": "Audio", "MediaType": "Audio",
        "Album": "Leuchtfeuer", "AlbumId": "a1", "AlbumPrimaryImageTag": "tag",
        "AlbumArtist": "Nora Tiefenbach", "AlbumArtists": [{ "Name": "Nora Tiefenbach", "Id": "ar1" }],
        "Artists": ["Nora Tiefenbach"], "ArtistItems": [{ "Name": "Nora Tiefenbach", "Id": "ar1" }],
        "IndexNumber": 1, "ParentIndexNumber": 1, "RunTimeTicks": 1_800_000_000i64, "HasLyrics": true,
        "NormalizationGain": -6.5
    }))
    .unwrap();
    assert_eq!(track.kind, ItemKind::Audio);
    assert_eq!(track.album_artists[0].id, "ar1");
    assert_eq!(track.artist_items[0].name, "Nora Tiefenbach");
    assert!(track.has_lyrics);
    let album: BaseItem = serde_json::from_value(json!({
        "Id": "a1", "Name": "Leuchtfeuer", "Type": "MusicAlbum", "IsFolder": true,
        "ChildCount": 4, "SongCount": 4, "RecursiveItemCount": 4, "CumulativeRunTimeTicks": 7_200_000_000i64
    }))
    .unwrap();
    assert_eq!(album.song_count, Some(4));
    assert_eq!(album.cumulative_run_time_ticks, Some(7_200_000_000));
}

#[test]
fn live_tv_channel_program_recording() {
    let channel: BaseItem = serde_json::from_value(json!({
        "Id": "c1", "Name": "Kanal 1", "Type": "TvChannel", "ChannelNumber": "1", "MediaType": "Video",
        "CurrentProgram": { "Id": "p1", "Name": "Morgenjournal", "Type": "Program", "ChannelId": "c1",
                            "ChannelName": "Kanal 1", "StartDate": "2026-01-01T06:00:00.0000000Z",
                            "EndDate": "2026-01-01T07:00:00.0000000Z", "IsLive": true, "IsRepeat": false,
                            "EpisodeTitle": "Folge", "TimerId": "t1", "SeriesTimerId": "st1" }
    }))
    .unwrap();
    assert_eq!(channel.kind, ItemKind::TvChannel);
    let program = channel.current_program.unwrap();
    assert_eq!(program.kind, ItemKind::Program);
    assert!(program.is_live);
    assert_eq!(program.timer_id.as_deref(), Some("t1"));
    assert_eq!(
        program.start_date.as_deref(),
        Some("2026-01-01T06:00:00.0000000Z")
    );

    let rec: BaseItem = serde_json::from_value(json!({
        "Id": "r1", "Name": "Sendung", "Type": "Recording", "ChannelName": "Kanal 1",
        "CompletionPercentage": 100.0, "EndDate": "2026-01-01T07:00:00.0000000Z"
    }))
    .unwrap();
    assert_eq!(rec.kind, ItemKind::Recording);
    assert_eq!(rec.completion_percentage, Some(100.0));

    let timers: QueryResult<TimerInfo> = serde_json::from_value(json!({
        "Items": [{ "Id": "t1", "Type": "Timer", "ChannelId": "c1", "ProgramId": "p1", "Name": "Sendung",
                    "StartDate": "2026-01-01T06:00:00.0000000Z", "EndDate": "2026-01-01T07:00:00.0000000Z",
                    "PrePaddingSeconds": 60, "PostPaddingSeconds": 120, "Status": "New" }],
        "TotalRecordCount": 1, "StartIndex": 0
    }))
    .unwrap();
    assert_eq!(timers.items[0].post_padding_seconds, 120);
}

#[test]
fn search_filters_counts() {
    let hits: SearchHintResult = serde_json::from_value(json!({
        "SearchHints": [
            { "ItemId": "m1", "Id": "m1", "Name": "Beispielfilm", "MatchedTerm": "beispiel", "Type": "Movie",
              "ProductionYear": 2020, "PrimaryImageTag": "t", "PrimaryImageAspectRatio": 0.6667,
              "IsFolder": false, "RunTimeTicks": 100, "MediaType": "Video" },
            { "ItemId": "e1", "Name": "Folge", "Type": "Episode", "Series": "Serie", "IndexNumber": 1 },
            { "ItemId": "s1", "Name": "Lied", "Type": "Audio", "Album": "A", "AlbumArtist": "K", "Artists": ["K"] }
        ],
        "TotalRecordCount": 3
    }))
    .unwrap();
    assert_eq!(hits.search_hints.len(), 3);
    assert_eq!(hits.search_hints[1].series.as_deref(), Some("Serie"));
    assert_eq!(hits.search_hints[2].artists, ["K"]);

    let f: QueryFilters = serde_json::from_value(json!({
        "Genres": [{ "Name": "Drama", "Id": "g" }], "Tags": ["x"],
        "AudioLanguages": ["ger"], "SubtitleLanguages": ["eng"]
    }))
    .unwrap();
    assert_eq!(f.genres[0].id, "g");
    let c: ItemCounts = serde_json::from_value(json!({
        "MovieCount": 1, "SeriesCount": 2, "EpisodeCount": 3, "ArtistCount": 4, "ProgramCount": 5,
        "TrailerCount": 6, "SongCount": 7, "AlbumCount": 8, "MusicVideoCount": 9, "BoxSetCount": 10,
        "BookCount": 11, "ItemCount": 12
    }))
    .unwrap();
    assert_eq!((c.song_count, c.book_count, c.item_count), (7, 11, 12));
}

#[test]
fn admin_shapes() {
    let info: SystemInfo = serde_json::from_value(json!({
        "LocalAddress": "http://127.0.0.1:8096", "ServerName": "Beispiel", "Version": "12.4.0",
        "ProductName": "Jellyfin Server", "OperatingSystem": "", "Id": "i", "StartupWizardCompleted": true,
        "OperatingSystemDisplayName": "Testsystem", "HasPendingRestart": false, "IsShuttingDown": false,
        "SupportsLibraryMonitor": true, "WebSocketPortNumber": 8096, "CompletedInstallations": [],
        "CanSelfRestart": true, "CanLaunchWebBrowser": false, "HasUpdateAvailable": false,
        "SystemArchitecture": "Arm64"
    }))
    .unwrap();
    assert_eq!(info.version, "12.4.0");
    assert!(info.can_self_restart);

    let sessions: Vec<SessionInfo> = serde_json::from_value(json!([{
        "PlayState": { "PositionTicks": 5, "CanSeek": true, "IsPaused": true, "IsMuted": false,
                       "VolumeLevel": 80, "PlayMethod": "DirectPlay", "RepeatMode": "RepeatNone" },
        "AdditionalUsers": [], "Capabilities": { "PlayableMediaTypes": ["Video"] },
        "RemoteEndPoint": "192.0.2.1", "PlayableMediaTypes": ["Video"], "Id": "sess",
        "UserId": "u", "UserName": "demo", "Client": "Test", "LastActivityDate": "2026-01-01T00:00:00Z",
        "DeviceName": "PC", "DeviceId": "d", "ApplicationVersion": "1", "IsActive": true,
        "SupportsMediaControl": true, "SupportsRemoteControl": true, "HasCustomDeviceName": false,
        "NowPlayingItem": { "Id": "m1", "Name": "Film", "Type": "Movie" }, "ServerId": "s"
    }]))
    .unwrap();
    assert!(sessions[0].play_state.as_ref().unwrap().is_paused);
    assert_eq!(sessions[0].now_playing_item.as_ref().unwrap().name, "Film");

    let tasks: Vec<TaskInfo> = serde_json::from_value(json!([{
        "Name": "Scan", "State": "Running", "CurrentProgressPercentage": 12.5, "Id": "t",
        "LastExecutionResult": { "StartTimeUtc": "2026-01-01T00:00:00Z", "EndTimeUtc": "2026-01-01T00:01:00Z",
                                 "Status": "Completed", "Name": "Scan", "Key": "K", "Id": "x" },
        "Triggers": [{ "Type": "IntervalTrigger", "IntervalTicks": 1 }],
        "Description": "d", "Category": "Bibliothek", "IsHidden": false, "Key": "K"
    }]))
    .unwrap();
    assert_eq!(tasks[0].current_progress_percentage, Some(12.5));
    assert_eq!(
        tasks[0]
            .last_execution_result
            .as_ref()
            .unwrap()
            .status
            .as_deref(),
        Some("Completed")
    );

    let folders: Vec<VirtualFolder> = serde_json::from_value(json!([{
        "Name": "Filme", "Locations": ["/daten/filme"], "CollectionType": "movies",
        "LibraryOptions": { "EnableRealtimeMonitor": true }, "ItemId": "i", "RefreshProgress": 50.0,
        "RefreshStatus": "Active"
    }]))
    .unwrap();
    assert_eq!(folders[0].locations, ["/daten/filme"]);

    let log: QueryResult<ActivityLogEntry> = serde_json::from_value(json!({
        "Items": [{ "Id": 7, "Name": "Anmeldung", "Overview": "o", "ShortOverview": "s", "Type": "SessionStarted",
                    "Date": "2026-01-01T00:00:00Z", "UserId": "u", "Severity": "Information" }],
        "TotalRecordCount": 1, "StartIndex": 0
    }))
    .unwrap();
    assert_eq!(log.items[0].id, 7);
    let files: Vec<LogFile> = serde_json::from_value(json!([
        { "DateCreated": "2026-01-01T00:00:00Z", "DateModified": "2026-01-01T00:00:00Z", "Size": 5, "Name": "a.log" }
    ]))
    .unwrap();
    assert_eq!(files[0].size, 5);
}

#[test]
fn policy_and_configuration_roundtrip_unchanged() {
    let policy_json = json!({
        "IsAdministrator": false, "IsHidden": true, "IsDisabled": false, "BlockedTags": ["x"],
        "EnableUserPreferenceAccess": true, "AccessSchedules": [], "BlockUnratedItems": [],
        "EnableRemoteControlOfOtherUsers": false, "EnableSharedDeviceControl": true,
        "EnableRemoteAccess": true, "EnableLiveTvManagement": true, "EnableLiveTvAccess": true,
        "EnableMediaPlayback": true, "EnableAudioPlaybackTranscoding": true,
        "EnableVideoPlaybackTranscoding": true, "EnablePlaybackRemuxing": true,
        "ForceRemoteSourceTranscoding": false, "EnableContentDeletion": false,
        "EnableContentDeletionFromFolders": [], "EnableContentDownloading": true,
        "EnableSyncTranscoding": true, "EnableMediaConversion": true, "EnabledDevices": [],
        "EnableAllDevices": true, "EnabledChannels": [], "EnableAllChannels": false,
        "EnabledFolders": ["f1"], "EnableAllFolders": false, "InvalidLoginAttemptCount": 0,
        "LoginAttemptsBeforeLockout": -1, "MaxActiveSessions": 0, "EnablePublicSharing": true,
        "BlockedMediaFolders": [], "BlockedChannels": [], "RemoteClientBitrateLimit": 0,
        "AuthenticationProviderId": "A", "PasswordResetProviderId": "P", "SyncPlayAccess": "CreateAndJoinGroups"
    });
    let policy: UserPolicy = serde_json::from_value(policy_json.clone()).unwrap();
    assert!(policy.is_hidden && policy.enable_live_tv_management);
    assert_eq!(policy.enabled_folders, ["f1"]);
    let back = serde_json::to_value(&policy).unwrap();
    for (k, v) in policy_json.as_object().unwrap() {
        assert_eq!(&back[k], v, "Feld {k} verändert");
    }

    let cfg_json = json!({
        "AudioLanguagePreference": "ger", "PlayDefaultAudioTrack": true, "SubtitleLanguagePreference": "eng",
        "DisplayMissingEpisodes": false, "GroupedFolders": [], "SubtitleMode": "Smart",
        "DisplayCollectionsView": false, "EnableLocalPassword": false, "OrderedViews": ["a"],
        "LatestItemsExcludes": [], "MyMediaExcludes": [], "HidePlayedInLatest": true,
        "RememberAudioSelections": true, "RememberSubtitleSelections": true, "EnableNextEpisodeAutoPlay": true
    });
    let cfg: UserConfiguration = serde_json::from_value(cfg_json.clone()).unwrap();
    assert_eq!(cfg.ordered_views, ["a"]);
    let back = serde_json::to_value(&cfg).unwrap();
    for (k, v) in cfg_json.as_object().unwrap() {
        assert_eq!(&back[k], v, "Feld {k} verändert");
    }
}

#[test]
fn user_dto_with_login_dates() {
    let u: UserDto = serde_json::from_value(json!({
        "Name": "demo", "ServerId": "s", "Id": "u", "PrimaryImageTag": "t", "HasPassword": true,
        "HasConfiguredPassword": true, "EnableAutoLogin": false,
        "LastLoginDate": "2026-01-01T00:00:00Z", "LastActivityDate": "2026-01-02T00:00:00Z"
    }))
    .unwrap();
    assert_eq!(u.last_login_date.as_deref(), Some("2026-01-01T00:00:00Z"));
}
