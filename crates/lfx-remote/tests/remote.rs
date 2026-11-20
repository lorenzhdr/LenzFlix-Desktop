//! Integrationstests gegen den Mock (nur 127.0.0.1, kein echtes Netz).

use lfx_api::{Client, ClientInfo, ClientOptions, RetryPolicy};
use lfx_mock_jellyfin::{DEMO_PASSWORD, DEMO_USER, MockHandle, start};
use lfx_remote::{
    Backoff, ConnectionState, DisconnectReason, PlayerAction, PlayerSnapshot, RecordingPlayer,
    RemoteConfig, RemoteEvent, RemoteHandle, RemoteSession, SyncPlayEvent, TimeSyncPolicy,
};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::broadcast::Receiver;

struct Fixture {
    mock: MockHandle,
    player: Arc<RecordingPlayer>,
    handle: RemoteHandle,
    events: Receiver<RemoteEvent>,
}

fn client_for(addr: std::net::SocketAddr) -> Client {
    Client::with_options(
        &format!("http://{addr}"),
        ClientInfo::new("Test-Rechner", "00000000-0000-4000-8000-000000000001"),
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

fn config(client: Client, player: Arc<RecordingPlayer>) -> RemoteConfig {
    let mut cfg = RemoteConfig::new(client, player);
    cfg.backoff = Backoff {
        initial: Duration::from_millis(30),
        max: Duration::from_millis(200),
        factor: 2.0,
        reset_after: Duration::from_secs(5),
    };
    cfg.tick_interval = Duration::from_millis(30);
    cfg.time_policy = TimeSyncPolicy {
        greedy_count: 3,
        greedy_interval: Duration::from_millis(50),
        steady_interval: Duration::from_secs(30),
    };
    cfg
}

async fn fixture_with(prepare: impl FnOnce(&MockHandle)) -> Fixture {
    let (addr, mock) = start().await;
    let client = client_for(addr);
    client.login(DEMO_USER, DEMO_PASSWORD).await.unwrap();
    prepare(&mock);
    let player = Arc::new(RecordingPlayer::new());
    let handle = RemoteSession::start(config(client, player.clone()));
    let events = handle.subscribe();
    Fixture {
        mock,
        player,
        handle,
        events,
    }
}

async fn fixture() -> Fixture {
    fixture_with(|_| {}).await
}

/// Wartet auf ein Ereignis, das `pick` annimmt (höchstens 5 s).
async fn wait_event<T>(
    rx: &mut Receiver<RemoteEvent>,
    mut pick: impl FnMut(&RemoteEvent) -> Option<T>,
) -> T {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match tokio::time::timeout(left, rx.recv()).await {
            Ok(Ok(e)) => {
                if let Some(v) = pick(&e) {
                    return v;
                }
            }
            Ok(Err(_)) => continue,
            Err(_) => panic!("Ereignis nicht rechtzeitig eingetroffen"),
        }
    }
}

async fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !cond() {
        assert!(Instant::now() < deadline, "Zeitüberschreitung: {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn wait_connected(f: &mut Fixture) {
    wait_event(&mut f.events, |e| {
        matches!(e, RemoteEvent::Connection(ConnectionState::Connected)).then_some(())
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn connects_and_reports_capabilities() {
    let mut f = fixture().await;
    wait_connected(&mut f).await;
    wait_until("Fähigkeiten", || !f.mock.capabilities_posts().is_empty()).await;
    let caps = &f.mock.capabilities_posts()[0];
    assert_eq!(
        caps["PlayableMediaTypes"],
        serde_json::json!(["Video", "Audio"])
    );
    assert_eq!(caps["SupportsMediaControl"], true);
    assert!(
        caps["SupportedCommands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c == "SetVolume")
    );
    assert_eq!(f.mock.ws_connections_open(), 1);
    assert_eq!(f.handle.connection(), ConnectionState::Connected);
    f.handle.stop().await;
    wait_until("Verbindung zu", || f.mock.ws_connections_open() == 0).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn remote_messages_reach_the_player() {
    let mut f = fixture().await;
    wait_connected(&mut f).await;
    f.mock.ws_push(r#"{"MessageType":"Play","Data":{"ItemIds":["abc"],"StartPositionTicks":90000000,"PlayCommand":"PlayNow"}}"#);
    f.mock.ws_push(
        r#"{"MessageType":"Playstate","Data":{"Command":"Seek","SeekPositionTicks":123}}"#,
    );
    f.mock
        .ws_push(r#"{"MessageType":"Playstate","Data":{"Command":"Pause"}}"#);
    f.mock.ws_push(r#"{"MessageType":"GeneralCommand","Data":{"Name":"SetVolume","Arguments":{"Volume":"42"}}}"#);
    f.mock.ws_push(r#"{"MessageType":"GeneralCommand","Data":{"Name":"DisplayMessage","Arguments":{"Header":"Hi","Text":"Test"}}}"#);
    f.mock
        .ws_push(r#"{"MessageType":"TimerCreated","Data":{}}"#);
    f.mock.ws_push("kaputt");
    let player = f.player.clone();
    wait_until("Aktionen", || player.actions().len() >= 5).await;
    let a = f.player.actions();
    assert!(
        matches!(&a[0], PlayerAction::Play { item_ids, start_position_ticks: Some(90_000_000), .. } if item_ids == &["abc"])
    );
    assert_eq!(a[1], PlayerAction::SeekTicks(123));
    assert_eq!(a[2], PlayerAction::Pause);
    assert_eq!(a[3], PlayerAction::SetVolume(42));
    assert!(matches!(&a[4], PlayerAction::DisplayMessage { text, .. } if text == "Test"));
    // Unbekannte Typen kommen als Ereignis an, kaputte als ParseError.
    wait_event(&mut f.events, |e| {
        matches!(e, RemoteEvent::ParseError(_)).then_some(())
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn sends_keepalive_after_force_keepalive() {
    let mut f = fixture_with(|m| m.set_keepalive_secs(2)).await;
    wait_connected(&mut f).await;
    let mock = &f.mock;
    wait_until("KeepAlive", || {
        mock.ws_received()
            .iter()
            .any(|t| t == r#"{"MessageType":"KeepAlive"}"#)
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn reconnects_with_backoff_and_reports_again() {
    let mut f = fixture().await;
    wait_connected(&mut f).await;
    wait_until("erste Meldung", || f.mock.capabilities_posts().len() == 1).await;
    f.mock.ws_drop_all();
    let retry = wait_event(&mut f.events, |e| match e {
        RemoteEvent::Connection(ConnectionState::Disconnected { reason, retry_in }) => {
            Some((reason.clone(), *retry_in))
        }
        _ => None,
    })
    .await;
    assert_eq!(retry.0, DisconnectReason::Lost);
    assert_eq!(retry.1, Some(Duration::from_millis(30)));
    wait_connected(&mut f).await;
    assert_eq!(f.mock.ws_connections_total(), 2);
    wait_until("zweite Meldung", || f.mock.capabilities_posts().len() == 2).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn rejected_login_stops_without_retry() {
    let (addr, mock) = start().await;
    let client = client_for(addr);
    client.login(DEMO_USER, DEMO_PASSWORD).await.unwrap();
    mock.fail_next(1, 401);
    let player = Arc::new(RecordingPlayer::new());
    let handle = RemoteSession::start(config(client, player));
    let mut events = handle.subscribe();
    let state = wait_event(&mut events, |e| match e {
        RemoteEvent::Connection(s @ ConnectionState::Disconnected { .. }) => Some(s.clone()),
        _ => None,
    })
    .await;
    assert_eq!(
        state,
        ConnectionState::Disconnected {
            reason: DisconnectReason::Unauthorized,
            retry_in: None
        }
    );
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(mock.ws_connections_total(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn syncplay_group_lifecycle_and_scheduled_command() {
    // Serveruhr 5 s voraus: ohne Zeitabgleich käme der Befehl 5 s zu spät.
    let mut f = fixture_with(|m| m.set_utc_offset_ms(5000)).await;
    wait_connected(&mut f).await;
    let offset = wait_event(&mut f.events, |e| match e {
        RemoteEvent::SyncPlay(SyncPlayEvent::TimeSynced { offset_micros, .. }) => {
            Some(*offset_micros)
        }
        _ => None,
    })
    .await;
    assert!((offset - 5_000_000).abs() < 100_000, "Versatz {offset}");

    f.mock.add_syncplay_group("Filmabend", &["Eva", "Tom"]);
    let api = f.handle.syncplay();
    let groups = api.list_groups().await.unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].group_name, "Filmabend");
    api.join(&groups[0].group_id).await.unwrap();
    let info = wait_event(&mut f.events, |e| match e {
        RemoteEvent::SyncPlay(SyncPlayEvent::Joined(i)) => Some(i.clone()),
        _ => None,
    })
    .await;
    assert_eq!(info.participants, ["Eva", "Tom", "demo"]);
    assert_eq!(f.handle.group().unwrap().group_id, groups[0].group_id);

    // Spielt und steht bei 0: Unpause in 200 ms (Serverzeit) darf nicht sofort und nicht 5 s später laufen.
    f.player.set_state(PlayerSnapshot {
        item_id: Some("i1".into()),
        position_ticks: 0,
        playing: false,
        buffering: false,
        rate: 1.0,
    });
    f.player.clear();
    let sent = Instant::now();
    api.unpause().await.unwrap();
    let player = f.player.clone();
    wait_until("Unpause", || {
        player.actions().contains(&PlayerAction::Unpause)
    })
    .await;
    let waited = sent.elapsed();
    assert!(waited >= Duration::from_millis(120), "zu früh: {waited:?}");
    assert!(waited < Duration::from_millis(1500), "zu spät: {waited:?}");
    assert!(f.mock.syncplay_calls().iter().any(|(a, _)| a == "Unpause"));

    api.leave().await.unwrap();
    wait_event(&mut f.events, |e| {
        matches!(e, RemoteEvent::SyncPlay(SyncPlayEvent::Left)).then_some(())
    })
    .await;
    assert!(f.handle.group().is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn syncplay_reports_buffering_and_ready() {
    let mut f = fixture().await;
    wait_connected(&mut f).await;
    let api = f.handle.syncplay();
    api.new_group("Testgruppe").await.unwrap();
    wait_event(&mut f.events, |e| {
        matches!(e, RemoteEvent::SyncPlay(SyncPlayEvent::Joined(_))).then_some(())
    })
    .await;
    let mut snap = PlayerSnapshot {
        item_id: Some("i1".into()),
        position_ticks: 70_000_000,
        playing: true,
        buffering: true,
        rate: 1.0,
    };
    f.player.set_state(snap.clone());
    let mock = &f.mock;
    wait_until("Buffering", || {
        mock.syncplay_calls().iter().any(|(a, _)| a == "Buffering")
    })
    .await;
    snap.buffering = false;
    f.player.set_state(snap);
    wait_until("Ready", || {
        mock.syncplay_calls().iter().any(|(a, _)| a == "Ready")
    })
    .await;
    let calls = mock.syncplay_calls();
    let ready = calls.iter().find(|(a, _)| a == "Ready").unwrap();
    assert_eq!(ready.1["PositionTicks"], 70_000_000);
    assert!(ready.1["When"].as_str().unwrap().ends_with('Z'));
}
