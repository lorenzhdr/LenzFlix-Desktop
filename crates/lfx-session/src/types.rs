//! Öffentliche Datentypen der Sitzungsverwaltung.

use lfx_api::{Client, ClientOptions, RetryPolicy};
use std::time::Duration;

/// Aktive Sitzung: fertig konfigurierter API-Client samt Kontoangaben.
#[derive(Debug, Clone)]
pub struct ActiveSession {
    /// API-Client mit gesetztem Token (Klonen ist billig).
    pub client: Client,
    /// Server-Id.
    pub server_id: String,
    /// Benutzer-Id.
    pub user_id: String,
    /// Anzeigename des Benutzers.
    pub user_name: String,
    /// Administrator? (Beim Offline-Wechsel aus dem lokalen Speicher.)
    pub is_admin: bool,
}

/// Kennung eines Kontos (kein Geheimnis).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionId {
    /// Server-Id.
    pub server_id: String,
    /// Benutzer-Id.
    pub user_id: String,
}

/// Ereignisse der Sitzungsverwaltung (Bus per `tokio::sync::broadcast`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionEvent {
    /// Aktives Konto gewechselt; `None` nach Abmeldung.
    SessionChanged(Option<SessionId>),
    /// Erreichbarkeit des aktiven Servers hat sich geändert (`true` = online).
    Connectivity(bool),
    /// Der Server hat das Token abgelehnt (401); die gespeicherte Anmeldung wurde verworfen.
    AuthExpired(SessionId),
}

/// Eintrag für „Wer schaut?“.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserChoice {
    /// Benutzer-Id.
    pub id: String,
    /// Anzeigename.
    pub name: String,
    /// Adresse des Profilbilds (ohne Token), falls vorhanden.
    pub avatar_url: Option<String>,
    /// Braucht ein Passwort?
    pub has_password: bool,
    /// Gespeicherte Anmeldung im Schlüsselbund vorhanden (Wechsel ohne Passwort möglich)?
    pub signed_in: bool,
}

/// Zustand einer Quick-Connect-Anmeldung.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuickConnectState {
    /// Code, den der Nutzer in einer angemeldeten Sitzung bestätigen muss.
    Code(String),
    /// Wartet auf die Bestätigung.
    Waiting,
    /// Bestätigt; die Sitzung ist aktiv (siehe `SessionManager::active`).
    Authorized,
    /// Abgebrochen oder fehlgeschlagen (deutscher Text für die Anzeige).
    Failed(String),
}

/// Zeiten und Client-Einstellungen.
#[derive(Debug, Clone)]
pub struct SessionOptions {
    /// Abstand der Erreichbarkeitsprüfung, solange der Server erreichbar ist.
    pub heartbeat_interval: Duration,
    /// Erste Wartezeit nach einem Ausfall (verdoppelt sich).
    pub backoff_min: Duration,
    /// Obergrenze der Wartezeit bei Ausfall.
    pub backoff_max: Duration,
    /// Abstand der Quick-Connect-Abfragen.
    pub quick_connect_poll: Duration,
    /// Höchstdauer einer Quick-Connect-Anmeldung.
    pub quick_connect_timeout: Duration,
    /// Zeitlimit für kurze Anfragen (Prüfen, Benutzerliste, Abmelden).
    pub probe_timeout: Duration,
    /// Einstellungen des Haupt-Clients.
    pub client: ClientOptions,
}

impl Default for SessionOptions {
    fn default() -> Self {
        Self {
            heartbeat_interval: Duration::from_secs(30),
            backoff_min: Duration::from_secs(2),
            backoff_max: Duration::from_secs(60),
            quick_connect_poll: Duration::from_secs(2),
            quick_connect_timeout: Duration::from_secs(300),
            probe_timeout: Duration::from_secs(5),
            client: ClientOptions::default(),
        }
    }
}

impl SessionOptions {
    /// Einstellungen für kurze Anfragen ohne Wiederholung.
    pub(crate) fn fast_client(&self) -> ClientOptions {
        ClientOptions {
            timeout: self.probe_timeout,
            connect_timeout: self.probe_timeout.min(Duration::from_secs(3)),
            retry: RetryPolicy {
                retries: 0,
                ..RetryPolicy::default()
            },
        }
    }
}
