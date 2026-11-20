//! Kern der Sitzungsverwaltung.

use crate::error::{Result, SessionError};
use crate::types::{ActiveSession, SessionEvent, SessionId, SessionOptions, UserChoice};
use lfx_api::{
    ApiError, AuthenticationResult, Client, ClientInfo, DiscoveredServer, Secret, Session,
};
use lfx_platform::paths::CONFIG_FILE;
use lfx_platform::{Config, SecretStore, ServerEntry, account_name};
use lfx_store::Store;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

/// Gerätename gegenüber dem Server (bewusst kein Rechnername).
const DEVICE_NAME: &str = "LENZFLIX Desktop";
/// Schlüssel im lokalen Speicher: war der Benutzer zuletzt Administrator (für den Offline-Wechsel)?
const ADMIN_KEY: &str = "session.is_admin";

pub(crate) struct ActiveState {
    pub(crate) session: ActiveSession,
    /// Eigener Client ohne Wiederholungen für Prüfungen (teilt das Token, nicht den Zustand).
    pub(crate) probe: Client,
}

pub(crate) struct Inner {
    pub(crate) config_path: PathBuf,
    pub(crate) config: Mutex<Config>,
    pub(crate) secrets: Arc<dyn SecretStore>,
    pub(crate) store: Store,
    pub(crate) options: SessionOptions,
    pub(crate) active: Mutex<Option<ActiveState>>,
    pub(crate) events: broadcast::Sender<SessionEvent>,
    pub(crate) online: AtomicBool,
    pub(crate) heartbeat: Mutex<Option<JoinHandle<()>>>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        if let Some(task) = self
            .heartbeat
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            task.abort();
        }
    }
}

/// Verwaltet Server, Benutzer und die aktive Sitzung. Klonen ist billig (geteilter Zustand).
#[derive(Clone)]
pub struct SessionManager {
    pub(crate) inner: Arc<Inner>,
}

impl std::fmt::Debug for SessionManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SessionManager")
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct PublicUser {
    id: String,
    name: String,
    #[serde(default)]
    has_password: bool,
    #[serde(default)]
    primary_image_tag: Option<String>,
}

pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl SessionManager {
    /// Öffnet die Verwaltung. Die Serverliste liegt in `config_dir/config.json`, Tokens im `secrets`-Speicher.
    /// Ein zuletzt aktives Konto mit gespeichertem Token wird ohne Netzwerkzugriff wiederhergestellt.
    pub fn new(config_dir: &Path, secrets: Arc<dyn SecretStore>, store: Store) -> Result<Self> {
        Self::with_options(config_dir, secrets, store, SessionOptions::default())
    }

    /// Wie [`Self::new`] mit eigenen Zeiten (Tests).
    pub fn with_options(
        config_dir: &Path,
        secrets: Arc<dyn SecretStore>,
        store: Store,
        options: SessionOptions,
    ) -> Result<Self> {
        let config_path = config_dir.join(CONFIG_FILE);
        let config = Config::load_or_create(&config_path)?;
        let (events, _) = broadcast::channel(64);
        let manager = Self {
            inner: Arc::new(Inner {
                config_path,
                config: Mutex::new(config),
                secrets,
                store,
                options,
                active: Mutex::new(None),
                events,
                online: AtomicBool::new(true),
                heartbeat: Mutex::new(None),
            }),
        };
        manager.restore_active();
        Ok(manager)
    }

    fn restore_active(&self) {
        let (server, user, user_name, device_id) = {
            let cfg = lock(&self.inner.config);
            let Some(sel) = cfg.active.clone() else {
                return;
            };
            let Some(server) = cfg.server(&sel.server_id).cloned() else {
                return;
            };
            let Some(user) = server.users.iter().find(|u| u.id == sel.user_id).cloned() else {
                return;
            };
            (
                server.clone(),
                sel.user_id,
                user.name,
                cfg.device_id.clone(),
            )
        };
        let token = match self.inner.secrets.get(&account_name(&server.id, &user)) {
            Ok(Some(t)) => t,
            _ => return,
        };
        let Ok(client) = self.build_client(&server.url, &device_id, false) else {
            return;
        };
        client.set_session(Some(Session {
            token,
            user_id: user.clone(),
        }));
        let is_admin = self.cached_admin(&server.id, &user);
        if self
            .install(client, &server, &user, &user_name, is_admin, false)
            .is_err()
        {
            tracing::warn!("Gespeicherte Sitzung konnte nicht wiederhergestellt werden");
        }
    }

    // ------------------------------------------------------------ Hilfen

    fn device_id(&self) -> String {
        lock(&self.inner.config).device_id.clone()
    }

    pub(crate) fn build_client(&self, url: &str, device_id: &str, fast: bool) -> Result<Client> {
        let options = if fast {
            self.inner.options.fast_client()
        } else {
            self.inner.options.client.clone()
        };
        Ok(Client::with_options(
            url,
            ClientInfo::new(DEVICE_NAME, device_id),
            options,
        )?)
    }

    pub(crate) fn new_client(&self, url: &str, fast: bool) -> Result<Client> {
        self.build_client(url, &self.device_id(), fast)
    }

    pub(crate) fn server_entry(&self, server_id: &str) -> Result<ServerEntry> {
        lock(&self.inner.config)
            .server(server_id)
            .cloned()
            .ok_or(SessionError::UnknownServer)
    }

    /// Ändert die Konfiguration und speichert sie atomar.
    fn update_config<T>(&self, f: impl FnOnce(&mut Config) -> T) -> Result<T> {
        let mut cfg = lock(&self.inner.config);
        let out = f(&mut cfg);
        cfg.save(&self.inner.config_path)?;
        Ok(out)
    }

    fn cached_admin(&self, server_id: &str, user_id: &str) -> bool {
        self.inner
            .store
            .get_setting_as::<bool>(server_id, user_id, ADMIN_KEY)
            .ok()
            .flatten()
            .unwrap_or(false)
    }

    pub(crate) fn emit(&self, event: SessionEvent) {
        // Ohne Empfänger ist das kein Fehler.
        let _ = self.inner.events.send(event);
    }

    pub(crate) fn set_online(&self, online: bool) {
        if self.inner.online.swap(online, Ordering::SeqCst) != online {
            self.emit(SessionEvent::Connectivity(online));
        }
    }

    /// Setzt die aktive Sitzung (ohne Ereignis, wenn `announce` falsch ist).
    fn install(
        &self,
        client: Client,
        server: &ServerEntry,
        user_id: &str,
        user_name: &str,
        is_admin: bool,
        announce: bool,
    ) -> Result<ActiveSession> {
        let probe = self.new_client(&server.url, true)?;
        probe.set_session(client.session());
        let session = ActiveSession {
            client,
            server_id: server.id.clone(),
            user_id: user_id.to_owned(),
            user_name: user_name.to_owned(),
            is_admin,
        };
        *lock(&self.inner.active) = Some(ActiveState {
            session: session.clone(),
            probe,
        });
        if announce {
            self.emit(SessionEvent::SessionChanged(Some(SessionId {
                server_id: session.server_id.clone(),
                user_id: session.user_id.clone(),
            })));
        }
        Ok(session)
    }

    fn remember_admin(&self, server_id: &str, user_id: &str, is_admin: bool) {
        if let Err(err) = self
            .inner
            .store
            .set_setting_as(server_id, user_id, ADMIN_KEY, &is_admin)
        {
            tracing::warn!(%err, "Administrator-Merkmal nicht gespeichert");
        }
    }

    /// Schließt eine erfolgreiche Anmeldung ab: Token in den Schlüsselbund, Benutzer in die Liste,
    /// Konto aktiv setzen.
    pub(crate) fn finish_login(
        &self,
        client: Client,
        server_id: &str,
        auth: &AuthenticationResult,
    ) -> Result<ActiveSession> {
        let server = self.server_entry(server_id)?;
        let user_id = auth.user.id.clone();
        let user_name = auth.user.name.clone();
        let is_admin = auth
            .user
            .policy
            .as_ref()
            .is_some_and(|p| p.is_administrator);
        self.inner
            .secrets
            .set(&account_name(server_id, &user_id), &auth.access_token)?;
        self.update_config(|cfg| {
            cfg.upsert_user(server_id, &user_id, &user_name);
            cfg.set_active(server_id, &user_id);
        })?;
        self.remember_admin(server_id, &user_id, is_admin);
        let session = self.install(client, &server, &user_id, &user_name, is_admin, true)?;
        self.set_online(true);
        Ok(session)
    }

    // ------------------------------------------------------------ Server

    /// Alle bekannten Server.
    pub fn servers(&self) -> Vec<ServerEntry> {
        lock(&self.inner.config).servers.clone()
    }

    /// Prüft die Adresse (`/System/Info/Public`), normalisiert sie und legt den Server an. Kennt die
    /// Liste die Server-Id schon, werden Name und Adresse aktualisiert (Benutzer bleiben).
    pub async fn add_server(&self, url: &str) -> Result<ServerEntry> {
        let base = lfx_api::normalize_base(url)?;
        let client = self.new_client(&base, true)?;
        let info = client.public_info().await.map_err(|e| match e {
            ApiError::Decode(_) | ApiError::NotFound => SessionError::InvalidServer(
                "Die Adresse gehört nicht zu einem Jellyfin-Server".into(),
            ),
            other => SessionError::Api(other),
        })?;
        if info.id.trim().is_empty() {
            return Err(SessionError::InvalidServer("Server-Id fehlt".into()));
        }
        let name = if info.server_name.trim().is_empty() {
            "Jellyfin".to_owned()
        } else {
            info.server_name.clone()
        };
        self.update_config(|cfg| cfg.upsert_server(&info.id, &name, &base).clone())
    }

    /// Entfernt einen Server samt gespeicherten Anmeldungen, lokalen Daten und Benutzern.
    pub fn remove_server(&self, server_id: &str) -> Result<()> {
        let server = self.server_entry(server_id)?;
        let mut first_error = None;
        for user in &server.users {
            if let Err(err) = self
                .inner
                .secrets
                .delete(&account_name(server_id, &user.id))
            {
                first_error.get_or_insert(SessionError::from(err));
            }
        }
        self.update_config(|cfg| cfg.remove_server(server_id))?;
        self.inner.store.purge_server(server_id)?;
        if self.active_is_server(server_id) {
            self.clear_active();
            self.emit(SessionEvent::SessionChanged(None));
        }
        first_error.map_or(Ok(()), Err)
    }

    /// Sucht Server im lokalen Netz (UDP-Broadcast).
    pub async fn discover_servers(&self, timeout: Duration) -> Result<Vec<DiscoveredServer>> {
        Ok(lfx_api::discover(timeout).await?)
    }

    // ------------------------------------------------------------ Benutzer

    /// Benutzer für „Wer schaut?“: die öffentliche Liste des Servers (mit Profilbild-Adresse), danach lokal
    /// bekannte Benutzer, die dort nicht stehen (versteckte Konten). Ist der Server nicht erreichbar, gibt es
    /// nur die lokal bekannten.
    pub async fn users_for(&self, server_id: &str) -> Result<Vec<UserChoice>> {
        let server = self.server_entry(server_id)?;
        let client = self.new_client(&server.url, true)?;
        let public = match fetch_public_users(&client).await {
            Ok(list) => list,
            Err(err) => {
                tracing::debug!(%err, "Öffentliche Benutzerliste nicht abrufbar");
                Vec::new()
            }
        };
        let signed_in = |id: &str| {
            matches!(
                self.inner.secrets.get(&account_name(server_id, id)),
                Ok(Some(_))
            )
        };
        let mut out: Vec<UserChoice> = public
            .iter()
            .map(|u| UserChoice {
                avatar_url: u.primary_image_tag.as_deref().and_then(|tag| {
                    client
                        .url(
                            &format!("/Users/{}/Images/Primary", u.id),
                            &[("tag".to_owned(), tag.to_owned())],
                        )
                        .ok()
                        .map(String::from)
                }),
                id: u.id.clone(),
                name: u.name.clone(),
                has_password: u.has_password,
                signed_in: signed_in(&u.id),
            })
            .collect();
        for known in &server.users {
            if !out.iter().any(|u| u.id == known.id) {
                out.push(UserChoice {
                    id: known.id.clone(),
                    name: known.name.clone(),
                    avatar_url: None,
                    has_password: true,
                    signed_in: signed_in(&known.id),
                });
            }
        }
        Ok(out)
    }

    /// Meldet mit Benutzername und Passwort an und macht das Konto aktiv.
    pub async fn login_password(
        &self,
        server_id: &str,
        username: &str,
        password: &str,
    ) -> Result<ActiveSession> {
        let server = self.server_entry(server_id)?;
        let client = self.new_client(&server.url, false)?;
        let auth = client
            .login(username, password)
            .await
            .map_err(|e| match e {
                ApiError::Unauthorized => SessionError::InvalidCredentials,
                other => SessionError::Api(other),
            })?;
        self.finish_login(client, server_id, &auth)
    }

    /// Wechselt zu einem Benutzer mit gespeicherter Anmeldung, ohne Passwort. Das Token wird kurz geprüft:
    /// lehnt der Server es ab, wird es verworfen (`AuthExpired`). Ist der Server nicht erreichbar, wechselt
    /// die App trotzdem (Offline-Betrieb) und meldet `Connectivity(false)`.
    pub async fn switch_user(&self, server_id: &str, user_id: &str) -> Result<ActiveSession> {
        let server = self.server_entry(server_id)?;
        let user = server
            .users
            .iter()
            .find(|u| u.id == user_id)
            .cloned()
            .ok_or(SessionError::UnknownUser)?;
        let account = account_name(server_id, user_id);
        let token: Secret = self
            .inner
            .secrets
            .get(&account)?
            .ok_or(SessionError::NoStoredToken)?;
        let session = Session {
            token,
            user_id: user_id.to_owned(),
        };
        let check = self.new_client(&server.url, true)?;
        check.set_session(Some(session.clone()));
        let mut name = user.name.clone();
        let mut is_admin = self.cached_admin(server_id, user_id);
        let mut online = true;
        match check.me().await {
            Ok(me) => {
                if !me.name.is_empty() {
                    name = me.name;
                }
                is_admin = me.policy.as_ref().is_some_and(|p| p.is_administrator);
                self.remember_admin(server_id, user_id, is_admin);
            }
            Err(ApiError::Unauthorized) => {
                let _ = self.inner.secrets.delete(&account);
                self.emit(SessionEvent::AuthExpired(SessionId {
                    server_id: server_id.to_owned(),
                    user_id: user_id.to_owned(),
                }));
                return Err(SessionError::AuthExpired);
            }
            Err(err) if is_offline_error(&err) => online = false,
            Err(err) => return Err(err.into()),
        }
        let client = self.new_client(&server.url, false)?;
        client.set_session(Some(session));
        self.update_config(|cfg| {
            cfg.upsert_user(server_id, user_id, &name);
            cfg.set_active(server_id, user_id);
        })?;
        let active = self.install(client, &server, user_id, &name, is_admin, true)?;
        self.set_online(online);
        Ok(active)
    }

    /// Meldet das aktive Konto ab: Server-Abmeldung (best effort, kurzes Zeitlimit), Token aus dem
    /// Schlüsselbund löschen, keine aktive Sitzung mehr. Der Benutzer bleibt in der Liste.
    pub async fn logout(&self) -> Result<()> {
        let Some((session, probe)) = self.active_snapshot() else {
            return Ok(());
        };
        let limit = self.inner.options.probe_timeout;
        // Der Prüf-Client hat kein Wiederholen und ein kurzes Zeitlimit.
        match tokio::time::timeout(limit, probe.logout()).await {
            Ok(Err(err)) => tracing::debug!(%err, "Abmeldung am Server nicht bestätigt"),
            Err(_) => tracing::debug!("Abmeldung am Server: Zeitüberschreitung"),
            Ok(Ok(())) => {}
        }
        let secret_result = self
            .inner
            .secrets
            .delete(&account_name(&session.server_id, &session.user_id));
        session.client.set_session(None);
        self.clear_active();
        self.update_config(|cfg| cfg.active = None)?;
        self.emit(SessionEvent::SessionChanged(None));
        Ok(secret_result?)
    }

    /// Vergisst einen Benutzer vollständig (Token, Eintrag, lokale Daten). Ist er aktiv, wird er abgemeldet.
    pub fn remove_user(&self, server_id: &str, user_id: &str) -> Result<()> {
        self.server_entry(server_id)?;
        let secret_result = self.inner.secrets.delete(&account_name(server_id, user_id));
        self.update_config(|cfg| cfg.remove_user(server_id, user_id))?;
        self.inner.store.purge_user(server_id, user_id)?;
        let was_active = lock(&self.inner.active)
            .as_ref()
            .is_some_and(|a| a.session.server_id == server_id && a.session.user_id == user_id);
        if was_active {
            self.clear_active();
            self.emit(SessionEvent::SessionChanged(None));
        }
        Ok(secret_result?)
    }

    // ------------------------------------------------------------ Zustand

    /// Aktive Sitzung, falls vorhanden.
    pub fn active(&self) -> Option<ActiveSession> {
        lock(&self.inner.active).as_ref().map(|a| a.session.clone())
    }

    /// Zuletzt gewähltes Konto (auch nach Abmeldung/Ablauf leer oder nicht mehr aktiv).
    pub fn last_selection(&self) -> Option<SessionId> {
        lock(&self.inner.config).active.as_ref().map(|a| SessionId {
            server_id: a.server_id.clone(),
            user_id: a.user_id.clone(),
        })
    }

    /// Ist der aktive Server (nach letzter Prüfung) erreichbar? Ohne Prüfung `true`.
    pub fn is_online(&self) -> bool {
        self.inner.online.load(Ordering::SeqCst)
    }

    /// Neuer Empfänger für den Ereignisbus.
    pub fn subscribe(&self) -> broadcast::Receiver<SessionEvent> {
        self.inner.events.subscribe()
    }

    /// Meldet einen API-Fehler der App an die Verwaltung: bei `Unauthorized` gilt die Anmeldung als
    /// abgelaufen (Token verwerfen, Ereignis `AuthExpired`). Gibt `true` zurück, wenn das der Fall war.
    pub fn report_api_error(&self, err: &ApiError) -> bool {
        if matches!(err, ApiError::Unauthorized) {
            return self.expire_active();
        }
        false
    }

    pub(crate) fn expire_active(&self) -> bool {
        let Some(taken) = lock(&self.inner.active).take() else {
            return false;
        };
        let id = SessionId {
            server_id: taken.session.server_id.clone(),
            user_id: taken.session.user_id.clone(),
        };
        let _ = self
            .inner
            .secrets
            .delete(&account_name(&id.server_id, &id.user_id));
        taken.session.client.set_session(None);
        self.emit(SessionEvent::AuthExpired(id));
        self.emit(SessionEvent::SessionChanged(None));
        true
    }

    pub(crate) fn active_snapshot(&self) -> Option<(ActiveSession, Client)> {
        lock(&self.inner.active)
            .as_ref()
            .map(|a| (a.session.clone(), a.probe.clone()))
    }

    fn active_is_server(&self, server_id: &str) -> bool {
        lock(&self.inner.active)
            .as_ref()
            .is_some_and(|a| a.session.server_id == server_id)
    }

    fn clear_active(&self) {
        if let Some(old) = lock(&self.inner.active).take() {
            old.session.client.set_session(None);
        }
    }
}

/// Zählt der Fehler als „Server nicht erreichbar“?
pub(crate) fn is_offline_error(err: &ApiError) -> bool {
    matches!(err, ApiError::Network(_) | ApiError::Timeout)
        || matches!(err, ApiError::Status { status, .. } if *status >= 500)
}

async fn fetch_public_users(client: &Client) -> std::result::Result<Vec<PublicUser>, ApiError> {
    let bytes = client
        .fetch_bytes(&format!("{}/Users/Public", client.base_url()))
        .await?;
    serde_json::from_slice(&bytes).map_err(|e| ApiError::Decode(e.to_string()))
}
