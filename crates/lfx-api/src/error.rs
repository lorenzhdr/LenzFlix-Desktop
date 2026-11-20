//! Fehlertyp des API-Clients. Enthält nie URLs mit Query, Tokens oder Hostnamen.

use thiserror::Error;

/// Alle Fehler des Clients.
#[derive(Debug, Error)]
pub enum ApiError {
    /// Verbindungs-/Transportfehler (ohne URL).
    #[error("Netzwerkfehler: {0}")]
    Network(String),
    /// Zeitüberschreitung.
    #[error("Zeitüberschreitung")]
    Timeout,
    /// HTTP 401: Anmeldung fehlt oder Token ungültig.
    #[error("Nicht angemeldet oder Anmeldung abgelaufen")]
    Unauthorized,
    /// HTTP 403.
    #[error("Zugriff verweigert")]
    Forbidden,
    /// HTTP 404.
    #[error("Nicht gefunden")]
    NotFound,
    /// Anderer HTTP-Status (Pfad ohne Query).
    #[error("Server antwortete mit HTTP {status} für {path}")]
    Status {
        /// HTTP-Statuscode.
        status: u16,
        /// Pfad ohne Query.
        path: String,
    },
    /// Antwort nicht lesbar (JSON).
    #[error("Antwort nicht lesbar: {0}")]
    Decode(String),
    /// Serveradresse oder URL ungültig bzw. nicht erlaubt.
    #[error("Ungültige Adresse: {0}")]
    InvalidUrl(String),
    /// Aktion braucht eine Anmeldung (Benutzer-Id fehlt).
    #[error("Keine Anmeldung vorhanden")]
    NotLoggedIn,
    /// Quick Connect ist auf dem Server abgeschaltet.
    #[error("Quick Connect ist auf dem Server nicht aktiviert")]
    QuickConnectDisabled,
    /// Ein-/Ausgabefehler (z. B. Server-Suche).
    #[error("E/A-Fehler: {0}")]
    Io(#[from] std::io::Error),
}

/// Kurzform für Ergebnisse des Clients.
pub type Result<T> = std::result::Result<T, ApiError>;

impl ApiError {
    /// Lohnt sich ein erneuter Versuch (nur für idempotente Anfragen verwendet)?
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Network(_) | Self::Timeout => true,
            Self::Status { status, .. } => matches!(status, 502..=504),
            _ => false,
        }
    }

    pub(crate) fn from_reqwest(err: reqwest::Error) -> Self {
        // URL aus der Fehlermeldung entfernen (könnte Host/Query enthalten).
        let err = err.without_url();
        if err.is_timeout() {
            Self::Timeout
        } else if err.is_decode() || err.is_body() {
            Self::Decode(err.to_string())
        } else {
            Self::Network(err.to_string())
        }
    }
}
