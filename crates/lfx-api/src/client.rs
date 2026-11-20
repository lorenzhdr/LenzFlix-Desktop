//! Kern des Clients: Konfiguration, Sitzung, Anfrage-Pipeline (Auth-Header, Retry, Logging).

use crate::error::{ApiError, Result};
use crate::redact::redact_url;
use crate::secret::Secret;
use reqwest::{Method, StatusCode, Url};
use serde::{Serialize, de::DeserializeOwned};
use std::fmt;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

/// Produktname im Auth-Header.
pub const CLIENT_NAME: &str = "LENZFLIX";

/// Identität dieses Clients (Auth-Header `Client`, `Device`, `DeviceId`, `Version`).
#[derive(Debug, Clone)]
pub struct ClientInfo {
    /// Gerätename, wie ihn der Server in der Geräteliste zeigt.
    pub device_name: String,
    /// Stabile Geräte-Id (UUID), pro Installation einmal erzeugt.
    pub device_id: String,
    /// Version der App (Standard: Version dieser Crate).
    pub version: String,
}

impl ClientInfo {
    /// Neue Identität mit der Crate-Version.
    pub fn new(device_name: impl Into<String>, device_id: impl Into<String>) -> Self {
        Self {
            device_name: device_name.into(),
            device_id: device_id.into(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
        }
    }
}

/// Angemeldete Sitzung (zum Wiederherstellen aus dem Schlüsselbund).
#[derive(Clone)]
pub struct Session {
    /// Zugriffstoken.
    pub token: Secret,
    /// Benutzer-Id.
    pub user_id: String,
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field("token", &self.token)
            .field("user_id", &"***")
            .finish()
    }
}

/// Wiederholungsregeln (gelten nur für idempotente GET-Anfragen).
#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    /// Zusätzliche Versuche nach dem ersten.
    pub retries: u32,
    /// Wartezeit vor dem ersten Wiederholen, verdoppelt sich je Versuch.
    pub base_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            retries: 2,
            base_delay: Duration::from_millis(250),
        }
    }
}

/// Einstellungen für [`Client::with_options`].
#[derive(Debug, Clone)]
pub struct ClientOptions {
    /// Zeitlimit je Anfrage.
    pub timeout: Duration,
    /// Zeitlimit für den Verbindungsaufbau.
    pub connect_timeout: Duration,
    /// Wiederholungsregeln.
    pub retry: RetryPolicy,
}

impl Default for ClientOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(20),
            connect_timeout: Duration::from_secs(8),
            retry: RetryPolicy::default(),
        }
    }
}

struct Inner {
    http: reqwest::Client,
    base: String,
    info: ClientInfo,
    retry: RetryPolicy,
    session: RwLock<Option<Session>>,
}

/// Jellyfin-API-Client. Klonen ist billig (geteilter Zustand).
#[derive(Clone)]
pub struct Client {
    inner: Arc<Inner>,
}

impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client")
            .field("server", &"<server>")
            .field("device_name", &self.inner.info.device_name)
            .field("logged_in", &self.is_logged_in())
            .finish()
    }
}

/// Wertet einen Auth-Header-Wert header-sicher aus: Anführungszeichen, Komma, Backslash, Prozent,
/// Steuer- und Nicht-ASCII-Zeichen werden prozentkodiert.
fn escape_header_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        match b {
            b'"' | b',' | b'\\' | b'%' | 0..=0x1f | 0x7f..=0xff => {
                out.push_str(&format!("%{b:02X}"))
            }
            _ => out.push(b as char),
        }
    }
    out
}

impl Client {
    /// Neuer Client für `server_url` (z. B. `http://192.168.1.10:8096`, auch mit Basispfad).
    pub fn new(server_url: &str, info: ClientInfo) -> Result<Self> {
        Self::with_options(server_url, info, ClientOptions::default())
    }

    /// Neuer Client mit eigenen Zeitlimits/Wiederholungsregeln.
    pub fn with_options(
        server_url: &str,
        info: ClientInfo,
        options: ClientOptions,
    ) -> Result<Self> {
        let base = normalize_base(server_url)?;
        let http = reqwest::Client::builder()
            .timeout(options.timeout)
            .connect_timeout(options.connect_timeout)
            .user_agent(format!("{CLIENT_NAME}/{}", info.version))
            .build()
            .map_err(ApiError::from_reqwest)?;
        Ok(Self {
            inner: Arc::new(Inner {
                http,
                base,
                info,
                retry: options.retry,
                session: RwLock::new(None),
            }),
        })
    }

    /// Normalisierte Serveradresse (ohne abschließenden Schrägstrich). Nur für Anzeige/Speicherung, nie loggen.
    pub fn base_url(&self) -> &str {
        &self.inner.base
    }

    /// Identität dieses Clients.
    pub fn info(&self) -> &ClientInfo {
        &self.inner.info
    }

    /// Aktuelle Sitzung, falls angemeldet.
    pub fn session(&self) -> Option<Session> {
        self.inner
            .session
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Sitzung setzen (z. B. aus dem Schlüsselbund wiederhergestellt) oder löschen.
    pub fn set_session(&self, session: Option<Session>) {
        *self
            .inner
            .session
            .write()
            .unwrap_or_else(|e| e.into_inner()) = session;
    }

    /// Ist eine Sitzung gesetzt?
    pub fn is_logged_in(&self) -> bool {
        self.session().is_some()
    }

    /// Benutzer-Id der Sitzung oder [`ApiError::NotLoggedIn`].
    pub fn user_id(&self) -> Result<String> {
        self.session()
            .map(|s| s.user_id)
            .ok_or(ApiError::NotLoggedIn)
    }

    /// Wert des `Authorization`-Headers:
    /// `MediaBrowser Client="LENZFLIX", Device="…", DeviceId="…", Version="…"[, Token="…"]`.
    /// Enthält das Token: nie loggen. Wird auch für mpv (`http-header-fields`) und Bildladen gebraucht.
    pub fn auth_header_value(&self) -> String {
        let info = &self.inner.info;
        let mut value = format!(
            "MediaBrowser Client=\"{CLIENT_NAME}\", Device=\"{}\", DeviceId=\"{}\", Version=\"{}\"",
            escape_header_value(&info.device_name),
            escape_header_value(&info.device_id),
            escape_header_value(&info.version),
        );
        if let Some(session) = self.session()
            && !session.token.is_empty()
        {
            value.push_str(&format!(
                ", Token=\"{}\"",
                escape_header_value(session.token.expose())
            ));
        }
        value
    }

    /// Absolute URL aus Pfad und Query (Basispfad des Servers bleibt erhalten).
    pub fn url(&self, path: &str, query: &[(String, String)]) -> Result<Url> {
        let mut url = Url::parse(&format!(
            "{}{}{}",
            self.inner.base,
            if path.starts_with('/') { "" } else { "/" },
            path
        ))
        .map_err(|_| ApiError::InvalidUrl("Pfad".into()))?;
        if !query.is_empty() {
            url.query_pairs_mut()
                .extend_pairs(query.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        }
        Ok(url)
    }

    /// Eine Anfrage ohne Wiederholung. Prüft den Status und mappt Fehler.
    async fn send_once(
        &self,
        method: &Method,
        url: &Url,
        body: Option<&serde_json::Value>,
    ) -> Result<reqwest::Response> {
        let started = Instant::now();
        let mut req = self
            .inner
            .http
            .request(method.clone(), url.clone())
            .header(reqwest::header::AUTHORIZATION, self.auth_header_value())
            .header(reqwest::header::ACCEPT, "application/json");
        if let Some(body) = body {
            req = req.json(body);
        }
        let result = req.send().await;
        let redacted = redact_url(url.as_str());
        match result {
            Err(err) => {
                let err = ApiError::from_reqwest(err);
                tracing::debug!(%method, url = %redacted, error = %err, "Anfrage fehlgeschlagen");
                Err(err)
            }
            Ok(resp) => {
                let status = resp.status();
                tracing::debug!(%method, url = %redacted, status = status.as_u16(), ms = started.elapsed().as_millis() as u64, "Antwort");
                if status.is_success() {
                    Ok(resp)
                } else {
                    Err(match status {
                        StatusCode::UNAUTHORIZED => ApiError::Unauthorized,
                        StatusCode::FORBIDDEN => ApiError::Forbidden,
                        StatusCode::NOT_FOUND => ApiError::NotFound,
                        s => ApiError::Status {
                            status: s.as_u16(),
                            path: url.path().to_owned(),
                        },
                    })
                }
            }
        }
    }

    /// Anfrage mit Wiederholung, aber nur für GET.
    pub(crate) async fn send(
        &self,
        method: Method,
        path: &str,
        query: &[(String, String)],
        body: Option<serde_json::Value>,
    ) -> Result<reqwest::Response> {
        let url = self.url(path, query)?;
        let retry = if method == Method::GET {
            self.inner.retry
        } else {
            RetryPolicy {
                retries: 0,
                ..self.inner.retry
            }
        };
        let mut attempt = 0u32;
        loop {
            match self.send_once(&method, &url, body.as_ref()).await {
                Err(err) if err.is_retryable() && attempt < retry.retries => {
                    let delay = retry.base_delay * 2u32.saturating_pow(attempt);
                    attempt += 1;
                    tracing::debug!(attempt, "Wiederholung nach Fehler");
                    tokio::time::sleep(delay).await;
                }
                other => return other,
            }
        }
    }

    async fn decode<T: DeserializeOwned>(resp: reqwest::Response) -> Result<T> {
        let bytes = resp.bytes().await.map_err(ApiError::from_reqwest)?;
        serde_json::from_slice(&bytes).map_err(|e| ApiError::Decode(e.to_string()))
    }

    /// GET mit JSON-Antwort.
    pub(crate) async fn get_json<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(String, String)],
    ) -> Result<T> {
        Self::decode(self.send(Method::GET, path, query, None).await?).await
    }

    /// POST/DELETE mit optionalem JSON-Body und JSON-Antwort.
    pub(crate) async fn call_json<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        query: &[(String, String)],
        body: Option<&impl Serialize>,
    ) -> Result<T> {
        let body = body
            .map(serde_json::to_value)
            .transpose()
            .map_err(|e| ApiError::Decode(e.to_string()))?;
        Self::decode(self.send(method, path, query, body).await?).await
    }

    /// POST/DELETE ohne Antwortauswertung (204 oder beliebiger Inhalt).
    pub(crate) async fn call_empty(
        &self,
        method: Method,
        path: &str,
        query: &[(String, String)],
        body: Option<&impl Serialize>,
    ) -> Result<()> {
        let body = body
            .map(serde_json::to_value)
            .transpose()
            .map_err(|e| ApiError::Decode(e.to_string()))?;
        self.send(method, path, query, body).await.map(|_| ())
    }

    /// Lädt Bytes (z. B. ein Bild) von einer URL **dieses Servers** mit Auth-Header.
    /// Fremde Hosts werden abgelehnt, damit das Token nie dorthin geht.
    pub async fn fetch_bytes(&self, url: &str) -> Result<bytes::Bytes> {
        let target = Url::parse(url).map_err(|_| ApiError::InvalidUrl("URL".into()))?;
        if !target
            .as_str()
            .starts_with(&format!("{}/", self.inner.base))
        {
            return Err(ApiError::InvalidUrl("fremder Server".into()));
        }
        let retry = self.inner.retry;
        let mut attempt = 0u32;
        loop {
            let res = async {
                let resp = self.send_once(&Method::GET, &target, None).await?;
                resp.bytes().await.map_err(ApiError::from_reqwest)
            }
            .await;
            match res {
                Err(err) if err.is_retryable() && attempt < retry.retries => {
                    let delay = retry.base_delay * 2u32.saturating_pow(attempt);
                    attempt += 1;
                    tokio::time::sleep(delay).await;
                }
                other => return other,
            }
        }
    }
}

/// Prüft und normalisiert eine Serveradresse. Ohne Schema wird `http://` ergänzt.
pub fn normalize_base(input: &str) -> Result<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(ApiError::InvalidUrl("leer".into()));
    }
    let with_scheme = if trimmed.contains("://") {
        trimmed.to_owned()
    } else {
        format!("http://{trimmed}")
    };
    let url = Url::parse(&with_scheme).map_err(|_| ApiError::InvalidUrl("Format".into()))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(ApiError::InvalidUrl("Schema".into()));
    }
    let mut base = format!("{}://{}", url.scheme(), url.host_str().unwrap_or_default());
    if let Some(port) = url.port() {
        base.push_str(&format!(":{port}"));
    }
    base.push_str(url.path().trim_end_matches('/'));
    Ok(base)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_base() {
        assert_eq!(
            normalize_base("192.168.1.2:8096/").unwrap(),
            "http://192.168.1.2:8096"
        );
        assert_eq!(
            normalize_base("https://x.invalid/jellyfin//").unwrap(),
            "https://x.invalid/jellyfin"
        );
        assert!(normalize_base("ftp://x").is_err());
        assert!(normalize_base("  ").is_err());
    }

    #[test]
    fn escapes_header_values() {
        assert_eq!(escape_header_value("Büro \"PC\""), "B%C3%BCro %22PC%22");
    }
}
