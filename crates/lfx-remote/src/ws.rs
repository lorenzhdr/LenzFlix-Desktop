//! Verbindungsaufbau zum Session-WebSocket `/socket`.
//!
//! Die OpenAPI-Spezifikation beschreibt `/socket` nicht. Der Server wertet für den Upgrade-Request dieselbe
//! Authentifizierung aus wie für REST, also den `Authorization`-Header. Deshalb ist [`WsAuth::Header`] der
//! Standard. Nur wenn ein Aufbau mit Header nicht möglich ist, gibt es [`WsAuth::QueryApiKey`]
//! (`?api_key=…&deviceId=…`); dann steht das Token in der URL und die URL darf nie geloggt werden.

use lfx_api::Client;
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{HeaderValue, header};
use tokio_tungstenite::tungstenite::{self, Error as WsProtoError};
use tokio_tungstenite::{
    Connector, MaybeTlsStream, WebSocketStream, connect_async_tls_with_config,
};

/// Verbindungsstrom.
pub type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Art der Anmeldung am WebSocket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WsAuth {
    /// `Authorization`-Header (Standard).
    #[default]
    Header,
    /// Token als `api_key` in der URL (nur wenn Header nicht möglich).
    QueryApiKey,
}

/// Fehler beim Verbinden (ohne URL und Token).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WsError {
    /// Server lehnt die Anmeldung ab (401/403): nicht erneut versuchen.
    #[error("Anmeldung am Server abgelehnt (HTTP {0})")]
    Rejected(u16),
    /// Anderer Fehler (Netz, TLS, Protokoll, HTTP-Status).
    #[error("Verbindung fehlgeschlagen: {0}")]
    Failed(String),
}

impl WsError {
    /// Lohnt sich ein neuer Versuch?
    pub fn is_retryable(&self) -> bool {
        !matches!(self, Self::Rejected(_))
    }
}

/// Baut die WebSocket-URL: Schema `ws`/`wss`, Pfad `/socket` unter dem Basispfad des Servers.
pub fn socket_url(client: &Client, auth: WsAuth) -> Result<String, WsError> {
    let mut query = Vec::new();
    if auth == WsAuth::QueryApiKey {
        let token = client
            .session()
            .map(|s| s.token.expose().to_owned())
            .unwrap_or_default();
        query.push(("api_key".to_owned(), token));
        query.push(("deviceId".to_owned(), client.info().device_id.clone()));
    }
    let mut url = client
        .url("/socket", &query)
        .map_err(|_| WsError::Failed("Adresse".into()))?;
    let scheme = if url.scheme() == "https" { "wss" } else { "ws" };
    url.set_scheme(scheme)
        .map_err(|()| WsError::Failed("Schema".into()))?;
    Ok(url.to_string())
}

fn tls_connector() -> Result<Connector, WsError> {
    let mut roots = rustls::RootCertStore::empty();
    for cert in rustls_native_certs::load_native_certs().certs {
        let _ = roots.add(cert);
    }
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|_| WsError::Failed("TLS".into()))?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(Connector::Rustls(Arc::new(config)))
}

/// Öffnet den WebSocket.
pub async fn connect(client: &Client, auth: WsAuth) -> Result<WsStream, WsError> {
    let url = socket_url(client, auth)?;
    let secure = url.starts_with("wss://");
    let mut request = url
        .into_client_request()
        .map_err(|_| WsError::Failed("Anfrage".into()))?;
    if auth == WsAuth::Header {
        let value = HeaderValue::from_str(&client.auth_header_value())
            .map_err(|_| WsError::Failed("Auth-Header".into()))?;
        request.headers_mut().insert(header::AUTHORIZATION, value);
    }
    let connector = if secure { Some(tls_connector()?) } else { None };
    match connect_async_tls_with_config(request, None, false, connector).await {
        Ok((stream, _)) => Ok(stream),
        Err(WsProtoError::Http(resp)) => {
            let status = resp.status().as_u16();
            if status == 401 || status == 403 {
                Err(WsError::Rejected(status))
            } else {
                Err(WsError::Failed(format!("HTTP {status}")))
            }
        }
        // Fehlertexte der Bibliothek können die URL enthalten: nur die Art melden.
        Err(WsProtoError::Io(e)) => Err(WsError::Failed(format!("E/A ({:?})", e.kind()))),
        Err(WsProtoError::Tls(_)) => Err(WsError::Failed("TLS".into())),
        Err(WsProtoError::ConnectionClosed | WsProtoError::AlreadyClosed) => {
            Err(WsError::Failed("geschlossen".into()))
        }
        Err(tungstenite::Error::Url(_)) => Err(WsError::Failed("Adresse".into())),
        Err(_) => Err(WsError::Failed("Protokoll".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lfx_api::{ClientInfo, Secret, Session};

    fn client(base: &str) -> Client {
        let c = Client::new(base, ClientInfo::new("Testgerät", "dev-1")).unwrap();
        c.set_session(Some(Session {
            token: Secret::new("geheim-token"),
            user_id: "u".into(),
        }));
        c
    }

    #[test]
    fn url_schemes_and_base_path() {
        let u = socket_url(&client("http://192.0.2.1:8096"), WsAuth::Header).unwrap();
        assert_eq!(u, "ws://192.0.2.1:8096/socket");
        let u = socket_url(&client("https://media.example.test/jf"), WsAuth::Header).unwrap();
        assert_eq!(u, "wss://media.example.test/jf/socket");
    }

    #[test]
    fn header_mode_keeps_token_out_of_url() {
        let u = socket_url(&client("http://192.0.2.1:8096"), WsAuth::Header).unwrap();
        assert!(!u.contains("geheim-token") && !u.contains("api_key"));
    }

    #[test]
    fn query_mode_only_on_request() {
        let u = socket_url(&client("http://192.0.2.1:8096"), WsAuth::QueryApiKey).unwrap();
        assert!(u.contains("api_key=geheim-token") && u.contains("deviceId=dev-1"));
    }

    #[test]
    fn rejected_is_not_retryable() {
        assert!(!WsError::Rejected(401).is_retryable());
        assert!(WsError::Failed("x".into()).is_retryable());
    }
}
