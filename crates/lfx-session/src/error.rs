//! Fehlertyp der Sitzungsverwaltung. Enthält nie Tokens, Passwörter oder Server-URLs.

use lfx_api::ApiError;
use lfx_platform::{ConfigError, SecretError};
use lfx_store::StoreError;

/// Fehler der Sitzungsverwaltung.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    /// Fehler der Server-Schnittstelle.
    #[error(transparent)]
    Api(#[from] ApiError),
    /// Schlüsselbund nicht zugreifbar.
    #[error(transparent)]
    Secret(#[from] SecretError),
    /// Lokale Konfiguration nicht les- oder schreibbar.
    #[error(transparent)]
    Config(#[from] ConfigError),
    /// Lokale Datenbank nicht zugreifbar.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// Server ist nicht in der Liste.
    #[error("Server unbekannt")]
    UnknownServer,
    /// Benutzer ist auf diesem Server nicht bekannt.
    #[error("Benutzer unbekannt")]
    UnknownUser,
    /// Für den Benutzer liegt kein Zugriffstoken im Schlüsselbund (neu anmelden).
    #[error("Keine gespeicherte Anmeldung für diesen Benutzer")]
    NoStoredToken,
    /// Benutzername oder Passwort falsch.
    #[error("Benutzername oder Passwort falsch")]
    InvalidCredentials,
    /// Die gespeicherte Anmeldung ist abgelaufen (Token vom Server abgelehnt).
    #[error("Anmeldung abgelaufen")]
    AuthExpired,
    /// Die Adresse gehört nicht zu einem Jellyfin-Server.
    #[error("Kein gültiger Server: {0}")]
    InvalidServer(String),
}

/// Kurzform für Ergebnisse der Sitzungsverwaltung.
pub type Result<T> = std::result::Result<T, SessionError>;
