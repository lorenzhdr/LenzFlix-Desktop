//! LAN-Suche: UDP-Broadcast `who is JellyfinServer?` auf Port 7359, Antwort ist JSON.

use crate::error::{ApiError, Result};
use serde::Deserialize;
use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;
use tokio::net::UdpSocket;

/// UDP-Port der Server-Erkennung.
pub const DISCOVERY_PORT: u16 = 7359;
/// Anfragetext.
pub const DISCOVERY_QUERY: &[u8] = b"who is JellyfinServer?";

/// Gefundener Server.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DiscoveredServer {
    /// Server-Id.
    pub id: String,
    /// Anzeigename.
    pub name: String,
    /// Adresse (`http://host:port`), vom Server gemeldet.
    pub address: String,
    /// Adresse, unter der der Server die Anfrage sah (fehlt bei älteren Servern).
    #[serde(default)]
    pub endpoint_address: Option<String>,
}

/// Wertet eine Antwort aus. Ungültige Pakete ergeben `None`.
pub fn parse_discovery_response(data: &[u8]) -> Option<DiscoveredServer> {
    let server: DiscoveredServer = serde_json::from_slice(data).ok()?;
    (!server.id.is_empty() && !server.address.is_empty()).then_some(server)
}

/// Sucht im LAN per Broadcast (255.255.255.255) und sammelt Antworten bis zum Ablauf von `timeout`.
pub async fn discover(timeout: Duration) -> Result<Vec<DiscoveredServer>> {
    discover_at(
        SocketAddr::from((Ipv4Addr::BROADCAST, DISCOVERY_PORT)),
        timeout,
    )
    .await
}

/// Wie [`discover`], aber mit frei wählbarem Ziel (für Tests mit einem lokalen Antwortgeber).
pub async fn discover_at(target: SocketAddr, timeout: Duration) -> Result<Vec<DiscoveredServer>> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await?;
    socket.set_broadcast(true)?;
    socket.send_to(DISCOVERY_QUERY, target).await?;
    let mut found: Vec<DiscoveredServer> = Vec::new();
    let mut buf = vec![0u8; 4096];
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        match tokio::time::timeout_at(deadline, socket.recv_from(&mut buf)).await {
            Err(_) => break, // Zeitlimit: fertig
            Ok(Err(err)) => return Err(ApiError::Io(err)),
            Ok(Ok((len, _from))) => {
                if let Some(server) = parse_discovery_response(&buf[..len])
                    && !found.iter().any(|s| s.id == server.id)
                {
                    found.push(server);
                }
            }
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_response() {
        let json = br#"{"Address":"http://192.168.0.5:8096","Id":"abc123","Name":"Testserver","EndpointAddress":"192.168.0.9"}"#;
        let s = parse_discovery_response(json).unwrap();
        assert_eq!(s.id, "abc123");
        assert_eq!(s.name, "Testserver");
        assert_eq!(s.address, "http://192.168.0.5:8096");
        assert_eq!(s.endpoint_address.as_deref(), Some("192.168.0.9"));
    }

    #[test]
    fn parses_without_endpoint() {
        let json = br#"{"Address":"http://h:8096","Id":"x","Name":"N"}"#;
        assert!(
            parse_discovery_response(json)
                .unwrap()
                .endpoint_address
                .is_none()
        );
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_discovery_response(b"hallo").is_none());
        assert!(parse_discovery_response(br#"{"Address":"","Id":"","Name":"x"}"#).is_none());
    }
}
