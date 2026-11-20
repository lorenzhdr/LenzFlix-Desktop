//! Plattformdienste für LENZFLIX Desktop.
//!
//! - [`secrets`]: Token im OS-Schlüsselbund (Windows Credential Manager, macOS Keychain) hinter einem Trait,
//!   dazu eine In-Memory-Variante für Tests.
//! - [`paths`]: Konfig-, Cache- und Datenordner der App (nie im Repo).
//! - [`config`]: lokale JSON-Datei mit Serverliste, Benutzern und der Geräte-Id (atomar geschrieben).

pub mod config;
pub mod media_controls; // W2-player
pub mod paths;
pub mod power; // W2-player
pub mod secrets;

pub use config::{ActiveSelection, Config, ConfigError, ServerEntry, UserEntry};
pub use media_controls::{CommandHandler, MediaCommand, MediaControls, NowPlaying, PlaybackStatus}; // W2-player
pub use paths::AppDirs;
pub use power::KeepAwake; // W2-player
pub use secrets::{
    KeyringSecretStore, MemorySecretStore, SERVICE, SecretError, SecretStore, account_name,
    os_secret_store,
};
