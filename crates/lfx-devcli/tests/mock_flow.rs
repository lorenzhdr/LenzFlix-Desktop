//! Integrationstests von `lfx-dev` gegen den Mock-Server (nur 127.0.0.1, kein echtes Netz,
//! Schlüsselbund im Arbeitsspeicher).

use lfx_api::{Client, ClientInfo, ItemKind, ItemsQuery, PlaybackReport, TICKS_PER_SECOND};
use lfx_devcli::{Ctx, commands, g4};
use lfx_mock_jellyfin::{DEMO_PASSWORD, DEMO_USER, MockConfig, MockHandle, start_with};
use lfx_platform::{AppDirs, Config, MemorySecretStore, SecretStore};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

struct Env {
    addr: SocketAddr,
    mock: MockHandle,
    ctx: Ctx,
    secrets: Arc<MemorySecretStore>,
    _tmp: tempfile::TempDir,
}

fn text(buf: &[u8]) -> String {
    String::from_utf8_lossy(buf).into_owned()
}

async fn env_with(config: MockConfig) -> Env {
    let (addr, mock) = start_with(config).await;
    let tmp = tempfile::tempdir().unwrap();
    let secrets = Arc::new(MemorySecretStore::new());
    let mut ctx = Ctx::new(AppDirs::from_root(tmp.path()), secrets.clone());
    ctx.qc_poll_interval = Duration::from_millis(10);
    ctx.qc_timeout = Duration::from_secs(5);
    ctx.client_options.retry.base_delay = Duration::from_millis(5);
    Env {
        addr,
        mock,
        ctx,
        secrets,
        _tmp: tmp,
    }
}

async fn env() -> Env {
    env_with(MockConfig {
        quick_connect_auto_after_polls: Some(1),
        ..MockConfig::default()
    })
    .await
}

async fn logged_in() -> Env {
    let e = env().await;
    let mut out = Vec::new();
    commands::login(&e.ctx, &format!("http://{}", e.addr), &mut out)
        .await
        .unwrap();
    e
}

/// Client mit eigener Passwort-Anmeldung, um Zustand des Mocks vorzubereiten/zu prüfen.
async fn admin_client(addr: SocketAddr) -> Client {
    let c = Client::new(
        &format!("http://{addr}"),
        ClientInfo::new("Test", "00000000-0000-4000-8000-000000000001"),
    )
    .unwrap();
    c.login(DEMO_USER, DEMO_PASSWORD).await.unwrap();
    c
}

async fn movie_ids(c: &Client) -> Vec<String> {
    let q = ItemsQuery::new()
        .include_types(&[ItemKind::Movie])
        .recursive(true);
    c.items(&q)
        .await
        .unwrap()
        .items
        .into_iter()
        .map(|i| i.id)
        .collect()
}

fn assert_no_leaks(env: &Env, output: &str) {
    for token in env.mock.issued_tokens() {
        assert!(!output.contains(&token), "Token in der Ausgabe");
    }
    assert!(
        !output.contains(&env.addr.to_string()) && !output.contains("127.0.0.1"),
        "Server-Host in der Ausgabe:\n{output}"
    );
}

#[tokio::test]
async fn login_stores_token_and_config() {
    let e = env().await;
    let mut out = Vec::new();
    commands::login(&e.ctx, &format!("http://{}", e.addr), &mut out)
        .await
        .unwrap();
    let out = text(&out);
    assert!(out.contains("Quick-Connect-Code: 1234"), "{out}");
    assert!(out.contains("Angemeldet als demo"), "{out}");
    assert_eq!(e.secrets.len(), 1);
    let cfg = Config::load_or_create(&e.ctx.dirs.config_file()).unwrap();
    assert_eq!(cfg.servers.len(), 1);
    assert_eq!(cfg.servers[0].users[0].name, DEMO_USER);
    assert!(cfg.active.is_some());
    // Das Token liegt im Schlüsselbund, nicht in der Datei.
    let raw = std::fs::read_to_string(e.ctx.dirs.config_file()).unwrap();
    for token in e.mock.issued_tokens() {
        assert!(!raw.contains(&token));
        assert!(!out.contains(&token));
    }
    assert!(!out.contains("127.0.0.1"));
}

#[tokio::test]
async fn login_waits_for_manual_confirmation() {
    let e = env_with(MockConfig::default()).await;
    let url = format!("http://{}", e.addr);
    let confirm = async {
        // Warten, bis ein Quick-Connect-Antrag existiert, dann wie ein Nutzer bestätigen.
        loop {
            tokio::time::sleep(Duration::from_millis(20)).await;
            if e.mock.authorize_quick_connect() > 0 {
                break;
            }
        }
    };
    let mut out = Vec::new();
    let (res, ()) = tokio::join!(commands::login(&e.ctx, &url, &mut out), confirm);
    res.unwrap();
    assert_eq!(e.secrets.len(), 1);
}

#[tokio::test]
async fn login_times_out_without_confirmation() {
    let mut e = env_with(MockConfig::default()).await;
    e.ctx.qc_timeout = Duration::from_millis(150);
    let mut out = Vec::new();
    let err = commands::login(&e.ctx, &format!("http://{}", e.addr), &mut out)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("Zeitüberschreitung"), "{err}");
    assert!(e.secrets.is_empty());
}

#[tokio::test]
async fn login_fails_when_quick_connect_disabled() {
    let e = env().await;
    e.mock.set_quick_connect_enabled(false);
    let mut out = Vec::new();
    let err = commands::login(&e.ctx, &format!("http://{}", e.addr), &mut out)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("Quick Connect"), "{err}");
}

#[tokio::test]
async fn whoami_servers_logout() {
    let e = logged_in().await;
    let mut out = Vec::new();
    commands::whoami(&e.ctx, &mut out).await.unwrap();
    let who = text(&out);
    assert!(who.contains("Benutzer: demo"), "{who}");
    assert_no_leaks(&e, &who);

    let mut out = Vec::new();
    commands::servers(&e.ctx, &mut out).unwrap();
    let list = text(&out);
    assert!(
        list.contains("Mock-Videothek") && list.contains("demo [aktiv]"),
        "{list}"
    );
    assert_no_leaks(&e, &list);

    let mut out = Vec::new();
    commands::logout(&e.ctx, &mut out).await.unwrap();
    assert!(e.secrets.is_empty());
    assert!(
        e.mock.issued_tokens().is_empty(),
        "Token serverseitig nicht abgemeldet"
    );
    let err = commands::whoami(&e.ctx, &mut Vec::new()).await.unwrap_err();
    assert!(err.to_string().contains("Kein aktives Konto"), "{err}");
}

#[tokio::test]
async fn whoami_without_login_explains() {
    let e = env().await;
    let err = commands::whoami(&e.ctx, &mut Vec::new()).await.unwrap_err();
    assert!(err.to_string().contains("login"), "{err}");
}

#[tokio::test]
async fn g4_passes_three_times_and_leaks_nothing() {
    let e = logged_in().await;
    for run in 1..=3 {
        let mut out = Vec::new();
        let summary = g4::run(&e.ctx, &mut out).await.unwrap();
        let out = text(&out);
        assert!(summary.passed(), "Lauf {run}:\n{out}");
        assert_eq!(summary.skipped, 0, "Lauf {run}:\n{out}");
        for name in [
            "system_info_public",
            "users_me",
            "user_views",
            "items_movies",
            "resume",
            "next_up",
            "playback_info",
            "progress_roundtrip",
            "restore_userdata",
            "branding_css",
            "charts",
            "me_stats",
        ] {
            assert!(out.contains(&format!("] {name}")), "{name} fehlt:\n{out}");
        }
        assert!(!out.contains("[FAIL]"), "{out}");
        assert!(out.contains("<server>/cw/charts"), "{out}");
        assert_no_leaks(&e, &out);
    }
    // Nach drei Läufen steht nichts unter „Weiterschauen“.
    let c = admin_client(e.addr).await;
    assert!(c.resume(10, &[], &[]).await.unwrap().items.is_empty());
}

#[tokio::test]
async fn g4_restores_played_state() {
    let e = logged_in().await;
    let c = admin_client(e.addr).await;
    let ids = movie_ids(&c).await;
    for id in &ids {
        c.set_played(id, true).await.unwrap();
    }
    let summary = g4::run(&e.ctx, &mut Vec::new()).await.unwrap();
    assert!(summary.passed());
    for id in &ids {
        let ud = c.item(id, &[]).await.unwrap().user_data.unwrap();
        assert!(ud.played, "gesehen-Status verloren");
        assert_eq!(ud.playback_position_ticks, 0);
    }
}

#[tokio::test]
async fn g4_restores_resume_position() {
    let e = logged_in().await;
    let c = admin_client(e.addr).await;
    let ids = movie_ids(&c).await;
    let before = 100 * TICKS_PER_SECOND;
    for id in &ids {
        let item = c.item(id, &["MediaSources"]).await.unwrap();
        let src = item.media_sources[0].id.clone();
        let report = PlaybackReport::direct(id, src, "vorbereitung", before);
        c.report_progress(&report).await.unwrap();
    }
    let summary = g4::run(&e.ctx, &mut Vec::new()).await.unwrap();
    assert!(summary.passed());
    for id in &ids {
        let ud = c.item(id, &[]).await.unwrap().user_data.unwrap();
        assert_eq!(
            ud.playback_position_ticks, before,
            "Position nicht zurückgesetzt"
        );
        assert!(!ud.played);
    }
}

#[tokio::test]
async fn g4_fails_when_charts_worker_rejects() {
    let e = logged_in().await;
    e.mock.set_charts_enabled(false);
    let mut out = Vec::new();
    let summary = g4::run(&e.ctx, &mut out).await.unwrap();
    let out = text(&out);
    assert!(!summary.passed(), "{out}");
    assert_eq!(summary.fail, 2, "{out}");
    assert!(out.contains("HTTP 404"), "{out}");
    assert_no_leaks(&e, &out);
}

#[tokio::test]
async fn g4_skips_charts_without_branding() {
    let e = logged_in().await;
    e.mock.set_branding_css(None);
    let mut out = Vec::new();
    let summary = g4::run(&e.ctx, &mut out).await.unwrap();
    let out = text(&out);
    // Ohne Branding-CSS schlägt `branding_css` fehl (leer), Charts werden übersprungen.
    assert_eq!(summary.skipped, 2, "{out}");
    assert_eq!(summary.fail, 1, "{out}");
}

#[tokio::test]
async fn g4_without_login_is_an_error() {
    let e = env().await;
    assert!(g4::run(&e.ctx, &mut Vec::new()).await.is_err());
}

#[tokio::test]
async fn g4_reports_fail_for_expired_token() {
    let e = logged_in().await;
    // Token serverseitig entwerten.
    let c = admin_client(e.addr).await;
    let _ = c; // eigener Login erzeugt nur ein weiteres Token
    let acc = e.secrets.len();
    assert_eq!(acc, 1);
    let cfg = Config::load_or_create(&e.ctx.dirs.config_file()).unwrap();
    let a = cfg.active.unwrap();
    e.secrets
        .set(
            &lfx_platform::account_name(&a.server_id, &a.user_id),
            &lfx_api::Secret::new("ungueltig"),
        )
        .unwrap();
    let mut out = Vec::new();
    let summary = g4::run(&e.ctx, &mut out).await.unwrap();
    assert!(!summary.passed());
    assert!(text(&out).contains("[FAIL]"));
}
