//! Player ohne Bild und Ton gegen lavfi-Quellen und kleine lokale Dateien.

mod common;

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{headless_player, wait_event, wait_state, write_srt, write_wav};
use lfx_player::{EndKind, ExternalSubtitle, PlayRequest, PlayerEvent, TrackChoice, TrackKind};

fn request_for(path: &std::path::Path) -> PlayRequest {
    PlayRequest::new("item", "source", path.to_string_lossy().into_owned())
}

#[test]
fn plays_pauses_seeks_and_stops() {
    let Some(p) = headless_player() else { return };
    let mut ev = p.subscribe_events();
    let wav = write_wav(30);
    p.open(request_for(&wav));
    wait_event(&mut ev, "FileLoaded", |e| {
        matches!(e, PlayerEvent::FileLoaded { .. })
    });
    let s = wait_state(&p, "Position läuft", |s| s.position > 0.3);
    assert!(s.loaded && !s.paused);
    assert!((s.duration.unwrap() - 30.0).abs() < 0.5, "{:?}", s.duration);
    assert!(s.audio_tracks().count() == 1);
    assert!(s.selected(TrackKind::Audio).is_some());

    p.pause();
    wait_event(&mut ev, "Pause", |e| {
        matches!(e, PlayerEvent::PauseChanged { paused: true, .. })
    });
    let held = wait_state(&p, "pausiert", |s| s.paused).position;
    std::thread::sleep(Duration::from_millis(400));
    assert!((p.state().position - held).abs() < 0.2);

    p.seek_to(20.0);
    wait_event(&mut ev, "Seeked", |e| {
        matches!(e, PlayerEvent::Seeked { .. })
    });
    let pos = p.state().position;
    assert!((pos - 20.0).abs() < 1.0, "nach Seek: {pos}");
    p.seek_by(-5.0);
    wait_event(&mut ev, "Seeked 2", |e| {
        matches!(e, PlayerEvent::Seeked { .. })
    });
    let pos = p.state().position;
    assert!((pos - 15.0).abs() < 1.0, "nach relativem Seek: {pos}");

    p.toggle_pause();
    wait_state(&p, "läuft wieder", |s| !s.paused);
    p.set_volume(40.0);
    p.set_muted(true);
    p.set_speed(1.5);
    let s = wait_state(&p, "Lautstärke/Tempo", |s| {
        s.volume == 40.0 && s.muted && s.speed == 1.5
    });
    assert!(s.loaded);

    p.stop();
    let e = wait_event(&mut ev, "Ended", |e| matches!(e, PlayerEvent::Ended { .. }));
    assert!(matches!(
        e,
        PlayerEvent::Ended {
            kind: EndKind::Stop,
            ..
        }
    ));
    assert!(!p.state().loaded);
    let _ = std::fs::remove_file(wav);
}

#[test]
fn external_subtitle_tracks_and_selection() {
    let Some(p) = headless_player() else { return };
    let mut ev = p.subscribe_events();
    let (wav, srt) = (write_wav(20), write_srt());
    let mut req = request_for(&wav);
    req.external_subtitles.push(ExternalSubtitle {
        url: srt.to_string_lossy().into_owned(),
        lang: Some("deu".into()),
        title: Some("Beispiel".into()),
    });
    p.open(req);
    wait_event(&mut ev, "FileLoaded", |e| {
        matches!(e, PlayerEvent::FileLoaded { .. })
    });
    let s = wait_state(&p, "Untertitelspur", |s| s.subtitle_tracks().count() == 1);
    let sub = s.subtitle_tracks().next().unwrap().clone();
    assert!(sub.external);
    assert_eq!(sub.lang.as_deref(), Some("deu"));
    assert_eq!(sub.title.as_deref(), Some("Beispiel"));

    p.set_subtitle_track(TrackChoice::Id(sub.id));
    wait_state(&p, "Untertitel an", |s| {
        s.selected(TrackKind::Subtitle).is_some()
    });
    p.set_subtitle_track(TrackChoice::Off);
    wait_event(&mut ev, "TrackChanged", |e| {
        matches!(e, PlayerEvent::TrackChanged { .. })
    });
    wait_state(&p, "Untertitel aus", |s| {
        s.selected(TrackKind::Subtitle).is_none()
    });
    let _ = (std::fs::remove_file(wav), std::fs::remove_file(srt));
}

#[test]
fn subtitle_off_from_request_wins_over_auto_select() {
    let Some(p) = headless_player() else { return };
    let mut ev = p.subscribe_events();
    let (wav, srt) = (write_wav(20), write_srt());
    let mut req = request_for(&wav);
    req.subtitle_stream_index = Some(-1);
    req.external_subtitles.push(ExternalSubtitle {
        url: srt.to_string_lossy().into_owned(),
        lang: None,
        title: None,
    });
    p.open(req);
    wait_event(&mut ev, "FileLoaded", |e| {
        matches!(e, PlayerEvent::FileLoaded { .. })
    });
    let s = wait_state(&p, "Spur da", |s| s.subtitle_tracks().count() == 1);
    assert!(s.selected(TrackKind::Subtitle).is_none());
    let _ = (std::fs::remove_file(wav), std::fs::remove_file(srt));
}

#[test]
fn eof_is_reported_with_keep_open() {
    let Some(p) = headless_player() else { return };
    let mut ev = p.subscribe_events();
    let wav = write_wav(2);
    p.open(request_for(&wav));
    wait_event(&mut ev, "Eof", |e| matches!(e, PlayerEvent::Eof { .. }));
    let s = p.state();
    assert!(s.eof);
    assert!((s.position - 2.0).abs() < 0.2, "{}", s.position);
    let _ = std::fs::remove_file(wav);
}

#[test]
fn video_params_and_hwdec_for_lavfi_clip() {
    let Some(p) = headless_player() else { return };
    let mut ev = p.subscribe_events();
    let mut req = PlayRequest::new("item", "source", "av://lavfi:testsrc2=size=160x120:rate=25");
    req.start_seconds = 0.0;
    p.open(req);
    wait_event(&mut ev, "FileLoaded", |e| {
        matches!(e, PlayerEvent::FileLoaded { .. })
    });
    let s = wait_state(&p, "Videoparameter", |s| s.video.width > 0);
    assert_eq!((s.video.width, s.video.height), (160, 120));
    assert!(!s.video.is_hdr());
    assert!(s.tracks.iter().any(|t| t.kind == TrackKind::Video));
    assert!(s.hwdec.is_none());
}

#[test]
fn missing_file_ends_with_error() {
    let Some(p) = headless_player() else { return };
    let mut ev = p.subscribe_events();
    p.open(PlayRequest::new("item", "source", "Z:/gibt/es/nicht.mkv"));
    let e = wait_event(&mut ev, "Ended", |e| matches!(e, PlayerEvent::Ended { .. }));
    assert!(matches!(
        e,
        PlayerEvent::Ended {
            kind: EndKind::Error(_),
            ..
        }
    ));
    assert!(p.state().error.is_some());
}

#[test]
fn reopen_ends_old_playback_and_bumps_generation() {
    let Some(p) = headless_player() else { return };
    let mut ev = p.subscribe_events();
    let wav = write_wav(30);
    p.open(request_for(&wav));
    wait_event(&mut ev, "FileLoaded 1", |e| {
        matches!(e, PlayerEvent::FileLoaded { generation: 1 })
    });
    p.open(request_for(&wav));
    let e = wait_event(&mut ev, "Ended 1", |e| {
        matches!(e, PlayerEvent::Ended { .. })
    });
    assert!(matches!(
        e,
        PlayerEvent::Ended {
            generation: 1,
            kind: EndKind::Stop
        }
    ));
    wait_event(&mut ev, "FileLoaded 2", |e| {
        matches!(e, PlayerEvent::FileLoaded { generation: 2 })
    });
    assert_eq!(p.state().generation, 2);
    // Der verschluckte EndFile der ersten Wiedergabe darf die zweite nicht beenden.
    std::thread::sleep(Duration::from_millis(300));
    assert!(p.state().loaded);
    let _ = std::fs::remove_file(wav);
}

#[test]
fn authorization_header_is_sent_to_the_server_but_not_in_the_url() {
    let Some(p) = headless_player() else { return };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen: Arc<Mutex<String>> = Arc::default();
    let seen2 = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { break };
            let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
            let mut buf = [0u8; 4096];
            let n = s.read(&mut buf).unwrap_or(0);
            let text = String::from_utf8_lossy(&buf[..n]).into_owned();
            {
                let mut g = seen2.lock().unwrap();
                if g.is_empty() {
                    *g = text;
                }
            }
            let _ = s.write_all(
                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
        }
    });
    let mut ev = p.subscribe_events();
    let mut req = PlayRequest::new(
        "item",
        "source",
        format!("http://127.0.0.1:{port}/Videos/x/stream?static=true"),
    );
    // Komma im Wert: prüft die Längenpräfix-Schreibweise der mpv-Liste.
    req.http_headers
        .push("Authorization: MediaBrowser Client=\"LENZFLIX\", Token=\"abc123\"".into());
    p.open(req);
    wait_event(&mut ev, "Ended", |e| matches!(e, PlayerEvent::Ended { .. }));
    let request = seen.lock().unwrap().clone();
    assert!(
        request.starts_with("GET /Videos/x/stream?static=true"),
        "{request}"
    );
    assert!(
        request
            .to_ascii_lowercase()
            .contains("authorization: mediabrowser client=\"lenzflix\", token=\"abc123\""),
        "{request}"
    );
    assert!(!request.lines().next().unwrap().contains("abc123"));
}
