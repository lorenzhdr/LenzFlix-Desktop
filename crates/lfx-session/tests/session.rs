//! Tests der Sitzungsverwaltung gegen den Mock-Server (nur 127.0.0.1).

use lfx_mock_jellyfin::catalog::{SERVER_ID, USER_ID};
use lfx_mock_jellyfin::{
    DEMO_PASSWORD, DEMO_USER, GUEST_ID, GUEST_PASSWORD, GUEST_USER, MockConfig, MockHandle,
};
use lfx_platform::{MemorySecretStore, SecretStore, account_name};
use lfx_session::{QuickConnectState, SessionError, SessionEvent, SessionManager, SessionOptions};
use lfx_store::Store;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast::Receiver;

struct Env {
    dir: tempfile::TempDir,
    mgr: SessionManager,
    mock: MockHandle,
    url: String,
    secrets: Arc<MemorySecretStore>,
}

fn fast_options() -> SessionOptions {
    SessionOptions {
        heartbeat_interval: Duration::from_millis(30),
        backoff_min: Duration::from_millis(20),
        backoff_max: Duration::from_millis(80),
        quick_connect_poll: Duration::from_millis(20),
        quick_connect_timeout: Duration::from_secs(5),
        probe_timeout: Duration::from_secs(2),
        ..SessionOptions::default()
    }
}

async fn setup_with(config: MockConfig) -> Env {
    let (addr, mock) = lfx_mock_jellyfin::start_with(config).await;
    let dir = tempfile::tempdir().unwrap();
    let secrets = Arc::new(MemorySecretStore::new());
    let mgr = SessionManager::with_options(
        dir.path(),
        secrets.clone() as Arc<dyn SecretStore>,
        Store::open_in_memory().unwrap(),
        fast_options(),
    )
    .unwrap();
    Env {
        dir,
        mgr,
        mock,
        url: format!("http://{addr}"),
        secrets,
    }
}

async fn setup() -> Env {
    setup_with(MockConfig::default()).await
}

/// Wartet auf ein passendes Ereignis (andere werden übersprungen).
async fn wait_for(
    rx: &mut Receiver<SessionEvent>,
    pred: impl Fn(&SessionEvent) -> bool,
) -> SessionEvent {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let ev = rx.recv().await.expect("Ereignisbus offen");
            if pred(&ev) {
                return ev;
            }
        }
    })
    .await
    .expect("Ereignis kam nicht rechtzeitig")
}

#[tokio::test]
async fn add_server_normalizes_and_persists() {
    let env = setup().await;
    let entry = env.mgr.add_server(&format!("{}/", env.url)).await.unwrap();
    assert_eq!(entry.id, SERVER_ID);
    assert_eq!(entry.name, "Mock-Videothek");
    assert_eq!(entry.url, env.url);
    // Ohne Schema und mit Schrägstrich: gleicher Eintrag, keine Dopplung.
    let again = env
        .mgr
        .add_server(&format!("{}/", env.url.trim_start_matches("http://")))
        .await
        .unwrap();
    assert_eq!(again.url, env.url);
    assert_eq!(env.mgr.servers().len(), 1);
    // Neustart liest die Liste wieder.
    let mgr2 = SessionManager::new(
        env.dir.path(),
        env.secrets.clone() as Arc<dyn SecretStore>,
        Store::open_in_memory().unwrap(),
    )
    .unwrap();
    assert_eq!(mgr2.servers().len(), 1);
}

#[tokio::test]
async fn add_server_rejects_bad_addresses() {
    let env = setup().await;
    assert!(matches!(
        env.mgr.add_server("ftp://irgendwas").await,
        Err(SessionError::Api(_))
    ));
    // Nicht erreichbar: Port 1 auf Loopback ist geschlossen.
    assert!(env.mgr.add_server("127.0.0.1:1").await.is_err());
    assert!(env.mgr.servers().is_empty());
}

#[tokio::test]
async fn password_login_stores_token_in_keychain_only() {
    let env = setup().await;
    env.mgr.add_server(&env.url).await.unwrap();
    let mut rx = env.mgr.subscribe();
    let active = env
        .mgr
        .login_password(SERVER_ID, DEMO_USER, DEMO_PASSWORD)
        .await
        .unwrap();
    assert_eq!(active.user_id, USER_ID);
    assert_eq!(active.user_name, DEMO_USER);
    assert!(!active.is_admin);
    assert!(active.client.is_logged_in());
    assert!(active.client.me().await.is_ok());
    let ev = wait_for(&mut rx, |e| matches!(e, SessionEvent::SessionChanged(_))).await;
    assert!(matches!(ev, SessionEvent::SessionChanged(Some(id)) if id.user_id == USER_ID));
    let stored = env
        .secrets
        .get(&account_name(SERVER_ID, USER_ID))
        .unwrap()
        .unwrap();
    // Token steht weder in der Konfiguration noch in Debug-Ausgaben.
    let config_text = std::fs::read_to_string(env.dir.path().join("config.json")).unwrap();
    assert!(!config_text.contains(stored.expose()));
    let debug = format!("{active:?} {:?}", env.mgr);
    assert!(!debug.contains(stored.expose()));
    assert_eq!(env.mgr.last_selection().unwrap().user_id, USER_ID);
}

#[tokio::test]
async fn wrong_password_is_invalid_credentials() {
    let env = setup().await;
    env.mgr.add_server(&env.url).await.unwrap();
    let err = env
        .mgr
        .login_password(SERVER_ID, DEMO_USER, "falsch")
        .await
        .unwrap_err();
    assert!(matches!(err, SessionError::InvalidCredentials), "{err:?}");
    assert!(env.mgr.active().is_none());
    assert!(env.secrets.is_empty());
    assert!(matches!(
        env.mgr.login_password("gibt-es-nicht", "a", "b").await,
        Err(SessionError::UnknownServer)
    ));
}

#[tokio::test]
async fn users_for_lists_public_users_with_avatar() {
    let env = setup().await;
    env.mgr.add_server(&env.url).await.unwrap();
    let before = env.mgr.users_for(SERVER_ID).await.unwrap();
    assert_eq!(before.len(), 2);
    assert!(before.iter().all(|u| !u.signed_in));
    let demo = before.iter().find(|u| u.id == USER_ID).unwrap();
    let url = demo.avatar_url.as_deref().unwrap();
    assert!(url.contains(&format!("/Users/{USER_ID}/Images/Primary")));
    assert!(!url.contains("api_key") && !url.contains("ApiKey") && !url.contains("Token"));
    assert!(
        before
            .iter()
            .find(|u| u.id == GUEST_ID)
            .unwrap()
            .avatar_url
            .is_none()
    );
    env.mgr
        .login_password(SERVER_ID, DEMO_USER, DEMO_PASSWORD)
        .await
        .unwrap();
    let after = env.mgr.users_for(SERVER_ID).await.unwrap();
    assert!(after.iter().find(|u| u.id == USER_ID).unwrap().signed_in);
    assert!(!after.iter().find(|u| u.id == GUEST_ID).unwrap().signed_in);
    // Server weg: nur lokal bekannte Benutzer.
    env.mock.fail_next(u32::MAX, 503);
    let offline = env.mgr.users_for(SERVER_ID).await.unwrap();
    assert_eq!(offline.len(), 1);
    assert_eq!(offline[0].id, USER_ID);
}

#[tokio::test]
async fn quick_connect_flow() {
    let env = setup().await;
    env.mgr.add_server(&env.url).await.unwrap();
    let mut handle = env.mgr.login_quick_connect(SERVER_ID).unwrap();
    let Some(QuickConnectState::Code(code)) = handle.next().await else {
        panic!("Code erwartet");
    };
    assert_eq!(code.len(), 6);
    // Noch nicht bestätigt: Warten.
    assert_eq!(handle.next().await, Some(QuickConnectState::Waiting));
    assert!(env.mgr.active().is_none());
    assert_eq!(env.mock.authorize_quick_connect(), 1);
    assert_eq!(handle.next().await, Some(QuickConnectState::Authorized));
    assert_eq!(handle.next().await, None);
    let active = env.mgr.active().unwrap();
    assert_eq!(active.user_id, USER_ID);
    assert!(
        env.secrets
            .get(&account_name(SERVER_ID, USER_ID))
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn quick_connect_disabled_fails() {
    let env = setup_with(MockConfig {
        quick_connect_enabled: false,
        ..MockConfig::default()
    })
    .await;
    env.mgr.add_server(&env.url).await.unwrap();
    let mut handle = env.mgr.login_quick_connect(SERVER_ID).unwrap();
    assert!(matches!(
        handle.next().await,
        Some(QuickConnectState::Failed(text)) if text.contains("Quick Connect")
    ));
    assert_eq!(handle.next().await, None);
    assert!(env.mgr.active().is_none());
}

#[tokio::test]
async fn quick_connect_timeout_and_cancel() {
    let (addr, _mock) = lfx_mock_jellyfin::start().await;
    let dir = tempfile::tempdir().unwrap();
    let mgr = SessionManager::with_options(
        dir.path(),
        Arc::new(MemorySecretStore::new()),
        Store::open_in_memory().unwrap(),
        SessionOptions {
            quick_connect_timeout: Duration::from_millis(150),
            ..fast_options()
        },
    )
    .unwrap();
    mgr.add_server(&format!("http://{addr}")).await.unwrap();
    let mut handle = mgr.login_quick_connect(SERVER_ID).unwrap();
    let mut last = None;
    while let Some(state) = handle.next().await {
        last = Some(state);
    }
    assert!(matches!(last, Some(QuickConnectState::Failed(_))));
    // Abbrechen beendet den Strom ohne weiteren Zustand.
    let mut handle = mgr.login_quick_connect(SERVER_ID).unwrap();
    assert!(matches!(
        handle.next().await,
        Some(QuickConnectState::Code(_))
    ));
    handle.cancel();
    while handle.next().await.is_some() {}
    assert!(mgr.active().is_none());
}

#[tokio::test]
async fn switch_user_without_password() {
    let env = setup().await;
    env.mgr.add_server(&env.url).await.unwrap();
    env.mgr
        .login_password(SERVER_ID, DEMO_USER, DEMO_PASSWORD)
        .await
        .unwrap();
    env.mgr
        .login_password(SERVER_ID, GUEST_USER, GUEST_PASSWORD)
        .await
        .unwrap();
    assert_eq!(env.mgr.active().unwrap().user_id, GUEST_ID);
    let logins = env
        .mock
        .requests()
        .iter()
        .filter(|r| r.path == "/Users/AuthenticateByName")
        .count();
    let mut rx = env.mgr.subscribe();
    let active = env.mgr.switch_user(SERVER_ID, USER_ID).await.unwrap();
    assert_eq!(active.user_id, USER_ID);
    assert!(active.client.me().await.is_ok());
    assert!(matches!(
        wait_for(&mut rx, |e| matches!(e, SessionEvent::SessionChanged(_))).await,
        SessionEvent::SessionChanged(Some(id)) if id.user_id == USER_ID
    ));
    // Kein erneutes Login nötig.
    let logins_after = env
        .mock
        .requests()
        .iter()
        .filter(|r| r.path == "/Users/AuthenticateByName")
        .count();
    assert_eq!(logins, logins_after);
    assert_eq!(env.mgr.last_selection().unwrap().user_id, USER_ID);
}

#[tokio::test]
async fn switch_user_errors() {
    let env = setup().await;
    env.mgr.add_server(&env.url).await.unwrap();
    env.mgr
        .login_password(SERVER_ID, DEMO_USER, DEMO_PASSWORD)
        .await
        .unwrap();
    assert!(matches!(
        env.mgr.switch_user(SERVER_ID, "unbekannt").await,
        Err(SessionError::UnknownUser)
    ));
    // Token aus dem Schlüsselbund entfernt: neu anmelden.
    env.secrets
        .delete(&account_name(SERVER_ID, USER_ID))
        .unwrap();
    assert!(matches!(
        env.mgr.switch_user(SERVER_ID, USER_ID).await,
        Err(SessionError::NoStoredToken)
    ));
}

#[tokio::test]
async fn switch_user_with_expired_token() {
    let env = setup().await;
    env.mgr.add_server(&env.url).await.unwrap();
    env.mgr
        .login_password(SERVER_ID, DEMO_USER, DEMO_PASSWORD)
        .await
        .unwrap();
    env.mgr.logout().await.ok();
    // Token liegt wieder im Schlüsselbund, der Server kennt es aber nicht mehr.
    env.secrets
        .set(
            &account_name(SERVER_ID, USER_ID),
            &lfx_api::Secret::new("abgelaufen"),
        )
        .unwrap();
    let mut rx = env.mgr.subscribe();
    let err = env.mgr.switch_user(SERVER_ID, USER_ID).await.unwrap_err();
    assert!(matches!(err, SessionError::AuthExpired), "{err:?}");
    assert!(matches!(
        wait_for(&mut rx, |e| matches!(e, SessionEvent::AuthExpired(_))).await,
        SessionEvent::AuthExpired(id) if id.user_id == USER_ID
    ));
    assert!(
        env.secrets
            .get(&account_name(SERVER_ID, USER_ID))
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn switch_user_while_offline_still_switches() {
    let env = setup().await;
    env.mgr.add_server(&env.url).await.unwrap();
    env.mgr
        .login_password(SERVER_ID, DEMO_USER, DEMO_PASSWORD)
        .await
        .unwrap();
    env.mock.fail_next(u32::MAX, 503);
    let mut rx = env.mgr.subscribe();
    let active = env.mgr.switch_user(SERVER_ID, USER_ID).await.unwrap();
    assert_eq!(active.user_id, USER_ID);
    assert!(!env.mgr.is_online());
    wait_for(&mut rx, |e| *e == SessionEvent::Connectivity(false)).await;
}

#[tokio::test]
async fn logout_removes_token_everywhere() {
    let env = setup().await;
    env.mgr.add_server(&env.url).await.unwrap();
    env.mgr
        .login_password(SERVER_ID, DEMO_USER, DEMO_PASSWORD)
        .await
        .unwrap();
    assert_eq!(env.mock.issued_tokens().len(), 1);
    let mut rx = env.mgr.subscribe();
    env.mgr.logout().await.unwrap();
    assert!(env.mgr.active().is_none());
    assert!(env.secrets.is_empty());
    // Der Server hat das Token ebenfalls verworfen.
    assert!(env.mock.issued_tokens().is_empty());
    assert!(env.mgr.last_selection().is_none());
    assert_eq!(
        wait_for(&mut rx, |e| matches!(e, SessionEvent::SessionChanged(_))).await,
        SessionEvent::SessionChanged(None)
    );
    // Benutzer bleibt für „Wer schaut?“ bekannt, aber ohne Anmeldung.
    let users = env.mgr.users_for(SERVER_ID).await.unwrap();
    assert!(users.iter().all(|u| !u.signed_in));
    // Zweites Abmelden ist harmlos.
    env.mgr.logout().await.unwrap();
}

#[tokio::test]
async fn offline_detection_and_reconnect() {
    let env = setup().await;
    env.mgr.add_server(&env.url).await.unwrap();
    env.mgr
        .login_password(SERVER_ID, DEMO_USER, DEMO_PASSWORD)
        .await
        .unwrap();
    let mut rx = env.mgr.subscribe();
    assert!(env.mgr.check_connectivity().await);
    env.mock.fail_next(u32::MAX, 503);
    assert!(!env.mgr.check_connectivity().await);
    assert!(!env.mgr.is_online());
    assert_eq!(
        wait_for(&mut rx, |e| matches!(e, SessionEvent::Connectivity(_))).await,
        SessionEvent::Connectivity(false)
    );
    // Wiederholte Prüfung ändert nichts, kein zweites Ereignis.
    assert!(!env.mgr.check_connectivity().await);
    env.mock.fail_next(0, 503);
    assert!(env.mgr.check_connectivity().await);
    assert_eq!(
        wait_for(&mut rx, |e| matches!(e, SessionEvent::Connectivity(_))).await,
        SessionEvent::Connectivity(true)
    );
    // Ein Ausfall ist kein abgelaufenes Token.
    assert!(env.mgr.active().is_some());
}

#[tokio::test]
async fn offline_when_server_is_gone() {
    let env = setup().await;
    env.mgr.add_server(&env.url).await.unwrap();
    env.mgr
        .login_password(SERVER_ID, DEMO_USER, DEMO_PASSWORD)
        .await
        .unwrap();
    env.mock.shutdown().await;
    assert!(!env.mgr.check_connectivity().await);
    assert!(env.mgr.active().is_some());
}

#[tokio::test]
async fn heartbeat_loop_detects_outage_and_recovery() {
    let env = setup().await;
    env.mgr.add_server(&env.url).await.unwrap();
    env.mgr
        .login_password(SERVER_ID, DEMO_USER, DEMO_PASSWORD)
        .await
        .unwrap();
    let mut rx = env.mgr.subscribe();
    env.mgr.start_heartbeat();
    env.mock.fail_next(u32::MAX, 503);
    wait_for(&mut rx, |e| *e == SessionEvent::Connectivity(false)).await;
    env.mock.fail_next(0, 503);
    wait_for(&mut rx, |e| *e == SessionEvent::Connectivity(true)).await;
    env.mgr.stop_heartbeat();
}

#[tokio::test]
async fn heartbeat_backs_off_while_offline() {
    let env = setup().await;
    env.mgr.add_server(&env.url).await.unwrap();
    env.mgr
        .login_password(SERVER_ID, DEMO_USER, DEMO_PASSWORD)
        .await
        .unwrap();
    env.mock.fail_next(u32::MAX, 503);
    env.mock.clear_recorded();
    env.mgr.start_heartbeat();
    tokio::time::sleep(Duration::from_millis(600)).await;
    env.mgr.stop_heartbeat();
    let pings = env
        .mock
        .requests()
        .iter()
        .filter(|r| r.path == "/System/Ping")
        .count();
    // Wartezeiten 20, 40, 80, 80 … ms: deutlich weniger Anfragen als bei festem Abstand von 20 ms (30).
    assert!((3..=12).contains(&pings), "{pings} Pings");
}

#[tokio::test]
async fn unauthorized_becomes_auth_expired() {
    let env = setup().await;
    env.mgr.add_server(&env.url).await.unwrap();
    env.mgr
        .login_password(SERVER_ID, DEMO_USER, DEMO_PASSWORD)
        .await
        .unwrap();
    let mut rx = env.mgr.subscribe();
    env.mock.revoke_tokens();
    assert!(env.mgr.check_connectivity().await); // Server erreichbar, nur Token ungültig
    assert!(matches!(
        wait_for(&mut rx, |e| matches!(e, SessionEvent::AuthExpired(_))).await,
        SessionEvent::AuthExpired(id) if id.user_id == USER_ID
    ));
    assert!(env.mgr.active().is_none());
    assert!(env.secrets.is_empty());
}

#[tokio::test]
async fn api_error_401_reported_by_app_expires_session() {
    let env = setup().await;
    env.mgr.add_server(&env.url).await.unwrap();
    let active = env
        .mgr
        .login_password(SERVER_ID, DEMO_USER, DEMO_PASSWORD)
        .await
        .unwrap();
    env.mock.revoke_tokens();
    let err = active.client.me().await.unwrap_err();
    let mut rx = env.mgr.subscribe();
    assert!(!env.mgr.report_api_error(&lfx_api::ApiError::NotFound));
    assert!(env.mgr.active().is_some());
    assert!(env.mgr.report_api_error(&err));
    assert!(matches!(
        wait_for(&mut rx, |e| matches!(e, SessionEvent::AuthExpired(_))).await,
        SessionEvent::AuthExpired(_)
    ));
    assert!(env.mgr.active().is_none());
    // Zweite Meldung desselben Fehlers: nichts mehr zu tun.
    assert!(!env.mgr.report_api_error(&err));
}

#[tokio::test]
async fn active_session_restored_on_restart() {
    let env = setup().await;
    env.mgr.add_server(&env.url).await.unwrap();
    env.mgr
        .login_password(SERVER_ID, DEMO_USER, DEMO_PASSWORD)
        .await
        .unwrap();
    let store = Store::open_in_memory().unwrap();
    let mgr2 = SessionManager::new(
        env.dir.path(),
        env.secrets.clone() as Arc<dyn SecretStore>,
        store,
    )
    .unwrap();
    let active = mgr2.active().expect("Sitzung wiederhergestellt");
    assert_eq!(active.user_id, USER_ID);
    assert!(active.client.me().await.is_ok());
    // Ohne Token im Schlüsselbund bleibt die App abgemeldet.
    env.secrets
        .delete(&account_name(SERVER_ID, USER_ID))
        .unwrap();
    let mgr3 = SessionManager::new(
        env.dir.path(),
        env.secrets.clone() as Arc<dyn SecretStore>,
        Store::open_in_memory().unwrap(),
    )
    .unwrap();
    assert!(mgr3.active().is_none());
}

#[tokio::test]
async fn remove_server_and_user_clean_up() {
    let env = setup().await;
    env.mgr.add_server(&env.url).await.unwrap();
    env.mgr
        .login_password(SERVER_ID, DEMO_USER, DEMO_PASSWORD)
        .await
        .unwrap();
    env.mgr
        .login_password(SERVER_ID, GUEST_USER, GUEST_PASSWORD)
        .await
        .unwrap();
    env.mgr.remove_user(SERVER_ID, USER_ID).unwrap();
    assert!(
        env.secrets
            .get(&account_name(SERVER_ID, USER_ID))
            .unwrap()
            .is_none()
    );
    assert!(env.mgr.active().is_some()); // Gast ist noch aktiv
    let mut rx = env.mgr.subscribe();
    env.mgr.remove_server(SERVER_ID).unwrap();
    assert!(env.mgr.servers().is_empty());
    assert!(env.mgr.active().is_none());
    assert!(env.secrets.is_empty());
    assert_eq!(
        wait_for(&mut rx, |e| matches!(e, SessionEvent::SessionChanged(_))).await,
        SessionEvent::SessionChanged(None)
    );
    assert!(matches!(
        env.mgr.remove_server(SERVER_ID),
        Err(SessionError::UnknownServer)
    ));
}
