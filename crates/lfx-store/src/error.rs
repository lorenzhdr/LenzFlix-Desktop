//! Fehlertyp der Datenbank.

/// Fehler beim Zugriff auf die lokale Datenbank.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// SQLite meldet einen Fehler.
    #[error("Datenbankfehler: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// JSON konnte nicht gelesen oder geschrieben werden.
    #[error("JSON-Fehler: {0}")]
    Json(#[from] serde_json::Error),
    /// Ordner der Datenbank konnte nicht angelegt werden.
    #[error("Ordner nicht zugreifbar: {0}")]
    Io(#[from] std::io::Error),
    /// Die Datenbank stammt von einer neueren App-Version.
    #[error("Datenbank hat ein neueres Schema (Version {found}, bekannt bis {known})")]
    NewerSchema {
        /// Version in der Datei.
        found: i64,
        /// Höchste bekannte Version.
        known: i64,
    },
}

/// Kurzform für Ergebnisse der Datenbank.
pub type Result<T> = std::result::Result<T, StoreError>;
