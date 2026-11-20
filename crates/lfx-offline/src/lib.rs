//! Offline-Modus für LENZFLIX Desktop (ohne Oberfläche).
//!
//! - [`DownloadManager`]: Warteschlange, begrenzte Parallelität, Pause/Fortsetzen/Abbrechen, Wiederaufnahme
//!   nach Neustart, `Range`-Fortsetzen, Größen- und Prüfsummenprüfung, Platzprüfung; lädt Zubehör mit
//!   (Bilder, externe Untertitel, Element-JSON).
//! - Qualität: [`Quality::Original`] (`/Items/{id}/Download`) oder [`Quality::Transcoded`] (MP4 vom Server,
//!   nicht fortsetzbar; nur gegen den Mock getestet, nie gegen eine echte Umwandlung).
//! - Serien und Staffeln werden zu Sammel-Downloads ([`EnqueueOptions::only_unplayed`]).
//! - [`OfflineLibrary`]: Liste, Belegung, Löschen, „gesehen, dann löschen“, [`LocalPlayback`] (Dateipfad statt URL).
//! - [`OfflineProgress`] / [`OutboxSink`]: Fortschritt offline in die Outbox; [`OutboxSyncer`] gleicht beim
//!   Online-Werden ab (neueres `LastPlayedDate` gewinnt).
//! - Persistenz hinter [`DownloadStore`] und [`OfflineOutbox`] (Speicher- und SQLite-Variante).
//!
//! Tokens gehen nur im `Authorization`-Header mit, nie in URLs, Logs oder Fehlern.

mod error;
mod fetch;
mod layout;
mod library;
mod manager;
mod plan;
mod progress;
mod store;
mod sync;
mod time;
mod types;

pub use error::{OfflineError, Result};
pub use layout::{FixedSpace, OfflineLayout, SpaceProbe, SystemSpace, safe_segment};
pub use library::{
    LocalPlayback, OUTBOX_KIND, OfflineEntry, OfflineLibrary, OfflineSubtitle, Usage,
};
pub use manager::{DownloadManager, EnqueueOptions, ManagerConfig, ManagerSetup};
pub use progress::{OfflineProgress, OutboxSink, PLAYED_FRACTION};
pub use store::{
    DownloadStore, MemoryDownloadStore, MemoryOutbox, OfflineOutbox, OutboxItem,
    SqliteDownloadStore, SqliteOutbox,
};
pub use sync::{OutboxSyncer, SyncReport};
pub use time::{Clock, SystemClock, iso_from_millis, millis_from_iso};
pub use types::{
    DownloadRecord, DownloadState, LocalUserData, OfflineEvent, Quality, StoredFile,
    TranscodeProfile,
};
