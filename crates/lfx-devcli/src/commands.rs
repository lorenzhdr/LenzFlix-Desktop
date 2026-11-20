//! Befehle `login`, `whoami`, `servers`, `logout`, `discover`.
//!
//! Ausgaben enthalten nie das Token und nie die Serveradresse (nur Namen; Adressen als `<server>`).

use anyhow::{Context as _, Result, anyhow, bail};
use lfx_api::{Client, ClientInfo, ClientOptions, Secret, normalize_base};
use lfx_platform::{AppDirs, Config, SecretStore, account_name};
use std::io::Write;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Gerätename, den der Server in der Geräteliste zeigt.
pub const DEVICE_NAME: &str = "LENZFLIX Dev-CLI";

/// Umgebung der Befehle (in Tests ersetzbar).
pub struct Ctx {
    /// Ordner der App.
    pub dirs: AppDirs,
    /// Ablage der Tokens.
    pub secrets: Arc<dyn SecretStore>,
    /// Abstand der Quick-Connect-Abfragen.
    pub qc_poll_interval: Duration,
    /// Höchstdauer des Wartens auf die Bestätigung.
    pub qc_timeout: Duration,
    /// Zeitlimits und Wiederholungen des API-Clients.
    pub client_options: ClientOptions,
}

impl Ctx {
    /// Standardwerte: Abfrage alle 2 s, Zeitlimit 5 min.
    pub fn new(dirs: AppDirs, secrets: Arc<dyn SecretStore>) -> Self {
        Self {
            dirs,
            secrets,
            qc_poll_interval: Duration::from_secs(2),
            qc_timeout: Duration::from_secs(300),
            client_options: ClientOptions::default(),
        }
    }

    fn config(&self) -> Result<Config> {
        Config::load_or_create(&self.dirs.config_file()).context("Konfiguration laden")
    }

    fn save(&self, config: &Config) -> Result<()> {
        config
            .save(&self.dirs.config_file())
            .context("Konfiguration speichern")
    }

    fn client(&self, url: &str, device_id: &str) -> Result<Client> {
        Client::with_options(
            url,
            ClientInfo::new(DEVICE_NAME, device_id),
            self.client_options.clone(),
        )
        .map_err(|e| anyhow!("{e}"))
    }
}

/// Angemeldete Sitzung aus Konfiguration und Schlüsselbund.
pub struct OpenSession {
    /// Client mit gesetzter Sitzung.
    pub client: Client,
    /// Name des Servers.
    pub server_name: String,
    /// Name des Benutzers.
    pub user_name: String,
}

/// Stellt die zuletzt benutzte Sitzung wieder her (oder die einzige vorhandene).
pub fn open_session(ctx: &Ctx) -> Result<OpenSession> {
    let config = ctx.config()?;
    let (server_id, user_id) = match &config.active {
        Some(a) => (a.server_id.clone(), a.user_id.clone()),
        None => {
            let mut all = config
                .servers
                .iter()
                .flat_map(|s| s.users.iter().map(move |u| (s.id.clone(), u.id.clone())));
            match (all.next(), all.next()) {
                (Some(one), None) => one,
                _ => bail!(
                    "Kein aktives Konto. Bitte zuerst `lfx-dev login --server <url>` ausführen."
                ),
            }
        }
    };
    let server = config
        .server(&server_id)
        .ok_or_else(|| anyhow!("Aktiver Server nicht in der Konfiguration"))?;
    let user = server
        .users
        .iter()
        .find(|u| u.id == user_id)
        .ok_or_else(|| anyhow!("Aktiver Benutzer nicht in der Konfiguration"))?;
    let token = ctx
        .secrets
        .get(&account_name(&server_id, &user_id))
        .map_err(|e| anyhow!("{e}"))?
        .ok_or_else(|| anyhow!("Kein Token im Schlüsselbund. Bitte neu anmelden."))?;
    let client = ctx.client(&server.url, &config.device_id)?;
    client.set_session(Some(lfx_api::Session {
        token,
        user_id: user_id.clone(),
    }));
    Ok(OpenSession {
        client,
        server_name: server.name.clone(),
        user_name: user.name.clone(),
    })
}

/// `login --server <url>`: Quick Connect (Code anzeigen, auf Bestätigung warten, Token ablegen).
pub async fn login(ctx: &Ctx, server_url: &str, out: &mut dyn Write) -> Result<()> {
    let mut config = ctx.config()?;
    let base = normalize_base(server_url).map_err(|e| anyhow!("{e}"))?;
    let client = ctx.client(&base, &config.device_id)?;
    let info = client
        .public_info()
        .await
        .map_err(|e| anyhow!("Server nicht erreichbar: {e}"))?;
    writeln!(
        out,
        "Server: {} (Version {})",
        info.server_name, info.version
    )?;
    if !client.quick_connect_enabled().await.unwrap_or(false) {
        bail!("Quick Connect ist auf dem Server nicht aktiviert");
    }
    let started = client
        .quick_connect_initiate()
        .await
        .map_err(|e| anyhow!("{e}"))?;
    writeln!(out, "Quick-Connect-Code: {}", started.code)?;
    writeln!(
        out,
        "Bitte im Jellyfin-Web unter Benutzermenü > Quick Connect bestätigen (Wartezeit höchstens {} s).",
        ctx.qc_timeout.as_secs()
    )?;
    out.flush()?;

    let deadline = Instant::now() + ctx.qc_timeout;
    loop {
        let state = client
            .quick_connect_poll(&started.secret)
            .await
            .map_err(|e| anyhow!("Abfrage fehlgeschlagen: {e}"))?;
        if state.authenticated {
            break;
        }
        if Instant::now() + ctx.qc_poll_interval > deadline {
            bail!("Zeitüberschreitung: Quick Connect wurde nicht bestätigt");
        }
        tokio::time::sleep(ctx.qc_poll_interval).await;
    }

    let auth = client
        .quick_connect_login(&started.secret)
        .await
        .map_err(|e| anyhow!("Anmeldung fehlgeschlagen: {e}"))?;
    let (server_id, user_id) = (info.id.as_str(), auth.user.id.as_str());
    ctx.secrets
        .set(
            &account_name(server_id, user_id),
            &Secret::new(auth.access_token.expose()),
        )
        .map_err(|e| anyhow!("{e}"))?;
    config.upsert_server(server_id, &info.server_name, &base);
    config.upsert_user(server_id, user_id, &auth.user.name);
    config.set_active(server_id, user_id);
    ctx.save(&config)?;
    writeln!(
        out,
        "Angemeldet als {} auf {}. Token liegt im Schlüsselbund.",
        auth.user.name, info.server_name
    )?;
    Ok(())
}

/// `whoami`: fragt `/Users/Me` ab.
pub async fn whoami(ctx: &Ctx, out: &mut dyn Write) -> Result<()> {
    let session = open_session(ctx)?;
    let me = session
        .client
        .me()
        .await
        .map_err(|e| anyhow!("Abfrage fehlgeschlagen: {e}"))?;
    let admin = me.policy.as_ref().is_some_and(|p| p.is_administrator);
    writeln!(
        out,
        "Benutzer: {}{}\nServer: {}",
        me.name,
        if admin { " (Administrator)" } else { "" },
        session.server_name
    )?;
    Ok(())
}

/// `servers`: Liste der gespeicherten Server und Benutzer.
pub fn servers(ctx: &Ctx, out: &mut dyn Write) -> Result<()> {
    let config = ctx.config()?;
    if config.servers.is_empty() {
        writeln!(out, "Keine Server gespeichert.")?;
        return Ok(());
    }
    for (n, s) in config.servers.iter().enumerate() {
        writeln!(out, "{}. {} (<server>)", n + 1, s.name)?;
        for u in &s.users {
            let active = config
                .active
                .as_ref()
                .is_some_and(|a| a.server_id == s.id && a.user_id == u.id);
            let has_token = ctx
                .secrets
                .get(&account_name(&s.id, &u.id))
                .map(|t| t.is_some())
                .unwrap_or(false);
            writeln!(
                out,
                "   - {}{}{}",
                u.name,
                if active { " [aktiv]" } else { "" },
                if has_token { "" } else { " (kein Token)" }
            )?;
        }
    }
    Ok(())
}

/// `logout`: meldet die aktive Sitzung ab, löscht Token und Konfigurationseintrag.
pub async fn logout(ctx: &Ctx, out: &mut dyn Write) -> Result<()> {
    let mut config = ctx.config()?;
    let Some(active) = config.active.clone() else {
        bail!("Kein aktives Konto");
    };
    // Server-Abmeldung nur versuchen; lokal wird in jedem Fall aufgeräumt.
    let remote = match open_session(ctx) {
        Ok(s) => s.client.logout().await.is_ok(),
        Err(_) => false,
    };
    ctx.secrets
        .delete(&account_name(&active.server_id, &active.user_id))
        .map_err(|e| anyhow!("{e}"))?;
    config.remove_user(&active.server_id, &active.user_id);
    ctx.save(&config)?;
    writeln!(
        out,
        "Abgemeldet{}. Token aus dem Schlüsselbund gelöscht.",
        if remote {
            ""
        } else {
            " (Server nicht erreichbar, nur lokal)"
        }
    )?;
    Ok(())
}

/// `discover`: Suche im lokalen Netz (UDP-Broadcast). Die gefundenen Adressen werden bewusst
/// angezeigt, weil man sie für `login --server` braucht.
pub async fn discover(seconds: u64, out: &mut dyn Write) -> Result<()> {
    let found = lfx_api::discover(Duration::from_secs(seconds))
        .await
        .map_err(|e| anyhow!("{e}"))?;
    if found.is_empty() {
        writeln!(out, "Keine Server gefunden.")?;
    }
    for s in found {
        writeln!(out, "{} - {}", s.name, s.address)?;
    }
    Ok(())
}
