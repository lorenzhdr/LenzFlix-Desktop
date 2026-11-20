//! Lokale SQLite-Datenbank für LENZFLIX Desktop.
//!
//! - [`Store`]: `Send + Sync + Clone`, eine Verbindung hinter einem Mutex, WAL-Modus.
//! - Migrationen: eigener kleiner Migrator über `PRAGMA user_version` ([`migrations`]).
//! - Tabellen: `items` (Katalog-Cache), `user_data` (Benutzerdaten je Konto), `settings` (App-Einstellungen
//!   je Benutzer, nichts davon wird mit dem Web geteilt), `outbox` (ausstehende Server-Meldungen für Offline),
//!   `image_index` (Bild-Cache-Verzeichnis), `downloads` (Zeilen des Offline-Modus, siehe `lfx-offline`).
//!
//! Die Datenbank enthält nie Tokens oder Passwörter (die liegen im Schlüsselbund).

mod downloads;
mod error;
mod images;
mod items;
pub mod migrations;
mod outbox;
mod settings;
mod store;

pub use downloads::DownloadRow;
pub use error::{Result, StoreError};
pub use images::ImageEntry;
pub use outbox::{NewOutboxEntry, OutboxEntry};
pub use store::{Store, unix_now};
