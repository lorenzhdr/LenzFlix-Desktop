//! Fehlertyp des Offline-Modus. Enthält nie Tokens oder Server-Adressen.

use lfx_api::ApiError;
use lfx_store::StoreError;

/// Alle Fehler des Offline-Modus.
#[derive(Debug, thiserror::Error)]
pub enum OfflineError {
    /// Fehler der Server-API.
    #[error("Serverfehler: {0}")]
    Api(#[from] ApiError),
    /// Fehler der lokalen Datenbank.
    #[error("Datenbankfehler: {0}")]
    Store(#[from] StoreError),
    /// Datei-Ein-/Ausgabe.
    #[error("Dateifehler: {0}")]
    Io(#[from] std::io::Error),
    /// JSON nicht lesbar.
    #[error("JSON-Fehler: {0}")]
    Json(#[from] serde_json::Error),
    /// Das Konto darf keine Originale laden (Recht `EnableContentDownloading` fehlt) oder das Element ist gesperrt.
    #[error("Download für dieses Konto oder diesen Titel nicht erlaubt")]
    NotAllowed,
    /// Nicht genug freier Speicherplatz.
    #[error("Zu wenig Speicherplatz: {needed} Byte nötig, {available} Byte frei")]
    InsufficientSpace {
        /// Benötigte Bytes (inklusive Reserve).
        needed: u64,
        /// Freie Bytes.
        available: u64,
    },
    /// Größe der fertigen Datei weicht von der angekündigten ab.
    #[error("Größe stimmt nicht: erwartet {expected} Byte, erhalten {actual} Byte")]
    SizeMismatch {
        /// Angekündigte Größe.
        expected: u64,
        /// Tatsächliche Größe.
        actual: u64,
    },
    /// Prüfsumme der fertigen Datei weicht ab.
    #[error("Prüfsumme stimmt nicht")]
    ChecksumMismatch,
    /// Der Server antwortete unerwartet (nur Statuscode).
    #[error("Server antwortete mit HTTP {0}")]
    Http(u16),
    /// Netzwerkfehler beim Übertragen (ohne URL).
    #[error("Netzwerkfehler: {0}")]
    Network(String),
    /// Der Server lieferte nichts mehr (Leerlauf-Zeitlimit).
    #[error("Zeitüberschreitung bei der Übertragung")]
    Stalled,
    /// Download unbekannt.
    #[error("Download nicht gefunden")]
    UnknownDownload,
    /// Elementart lässt sich nicht laden.
    #[error("Dieser Titel lässt sich nicht herunterladen: {0}")]
    Unsupported(String),
    /// Ungültige Kennung (Pfadzeichen).
    #[error("Ungültige Kennung")]
    InvalidId,
    /// Aktion passt nicht zum Zustand.
    #[error("Aktion im aktuellen Zustand nicht möglich")]
    InvalidState,
}

/// Kurzform für Ergebnisse.
pub type Result<T> = std::result::Result<T, OfflineError>;

impl OfflineError {
    /// Lohnt sich ein automatischer neuer Versuch?
    pub(crate) fn is_transient(&self) -> bool {
        match self {
            Self::Network(_) | Self::Stalled => true,
            Self::Http(s) => matches!(s, 500 | 502..=504),
            Self::Api(e) => e.is_retryable(),
            _ => false,
        }
    }
}
