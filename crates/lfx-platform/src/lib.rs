//! Plattformdienste für LENZFLIX Desktop.
//!
//! - [`secrets`]: Token im OS-Schlüsselbund (Windows Credential Manager, macOS Keychain) hinter einem Trait,
//!   dazu eine In-Memory-Variante für Tests.
//! - [`paths`]: Konfig-, Cache- und Datenordner der App (nie im Repo).
//! - [`config`]: lokale JSON-Datei mit Serverliste, Benutzern und der Geräte-Id (atomar geschrieben).

pub mod config;
pub mod paths;
pub mod secrets;

pub use config::{ActiveSelection, Config, ConfigError, ServerEntry, UserEntry};
pub use paths::AppDirs;
pub use secrets::{
    KeyringSecretStore, MemorySecretStore, SERVICE, SecretError, SecretStore, account_name,
    os_secret_store,
};
