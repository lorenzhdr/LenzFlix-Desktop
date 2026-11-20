//! Lokale Konfiguration als JSON-Datei: Serverliste mit Benutzern, zuletzt aktives Konto, Geräte-Id.
//!
//! Enthält keine Zugangsdaten (Tokens liegen im Schlüsselbund, siehe [`crate::secrets`]). Geschrieben wird
//! atomar: erst in eine Nachbardatei, dann Umbenennen, damit ein Absturz nie eine halbe Datei hinterlässt.

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Aktuelle Schemaversion der Datei.
pub const SCHEMA_VERSION: u32 = 1;

/// Fehler beim Lesen oder Schreiben.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// Datei konnte nicht gelesen/geschrieben werden.
    #[error("Konfigurationsdatei nicht zugreifbar: {0}")]
    Io(#[from] std::io::Error),
    /// Datei ist kein gültiges JSON in diesem Format (wird nie überschrieben).
    #[error("Konfigurationsdatei ist beschädigt: {0}")]
    Corrupt(String),
    /// Datei stammt von einer neueren App-Version.
    #[error("Konfigurationsdatei hat eine unbekannte Version ({0})")]
    UnknownVersion(u32),
}

/// Benutzer eines Servers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserEntry {
    /// Benutzer-Id des Servers.
    pub id: String,
    /// Anzeigename.
    pub name: String,
}

/// Bekannter Server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerEntry {
    /// Server-Id.
    pub id: String,
    /// Anzeigename.
    pub name: String,
    /// Adresse (`http(s)://host:port[/pfad]`).
    pub url: String,
    /// Angemeldete Benutzer.
    #[serde(default)]
    pub users: Vec<UserEntry>,
}

/// Zuletzt benutztes Konto.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveSelection {
    /// Server-Id.
    pub server_id: String,
    /// Benutzer-Id.
    pub user_id: String,
}

/// Inhalt der Konfigurationsdatei.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    /// Schemaversion.
    pub version: u32,
    /// Geräte-Id (UUID v4), einmal pro Installation erzeugt.
    pub device_id: String,
    /// Bekannte Server.
    #[serde(default)]
    pub servers: Vec<ServerEntry>,
    /// Zuletzt benutztes Konto.
    #[serde(default)]
    pub active: Option<ActiveSelection>,
}

impl Config {
    /// Neue Konfiguration mit frisch erzeugter Geräte-Id.
    pub fn new() -> Self {
        Self {
            version: SCHEMA_VERSION,
            device_id: uuid::Uuid::new_v4().to_string(),
            servers: Vec::new(),
            active: None,
        }
    }

    /// Lädt die Datei oder legt eine neue Konfiguration an, wenn sie fehlt.
    /// Eine neu erzeugte Konfiguration wird sofort gespeichert, damit die Geräte-Id stabil bleibt.
    pub fn load_or_create(path: &Path) -> Result<Self, ConfigError> {
        match std::fs::read(path) {
            Ok(bytes) => Self::parse(&bytes),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                let config = Self::new();
                config.save(path)?;
                Ok(config)
            }
            Err(err) => Err(err.into()),
        }
    }

    /// Wertet JSON-Bytes aus.
    pub fn parse(bytes: &[u8]) -> Result<Self, ConfigError> {
        let config: Self =
            serde_json::from_slice(bytes).map_err(|e| ConfigError::Corrupt(e.to_string()))?;
        if config.version > SCHEMA_VERSION {
            return Err(ConfigError::UnknownVersion(config.version));
        }
        if config.device_id.trim().is_empty() {
            return Err(ConfigError::Corrupt("Geräte-Id fehlt".into()));
        }
        Ok(config)
    }

    /// Schreibt atomar (Nachbardatei + Umbenennen). Legt fehlende Ordner an.
    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        let json = serde_json::to_vec_pretty(self)
            .map_err(|e| ConfigError::Corrupt(format!("Serialisierung: {e}")))?;
        if let Some(dir) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        let tmp: PathBuf = {
            let mut name = path.file_name().unwrap_or_default().to_os_string();
            name.push(format!(".{}.tmp", std::process::id()));
            path.with_file_name(name)
        };
        let write = || -> std::io::Result<()> {
            let mut file = std::fs::File::create(&tmp)?;
            file.write_all(&json)?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&tmp, path)
        };
        write().inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })?;
        Ok(())
    }

    /// Server per Id.
    pub fn server(&self, id: &str) -> Option<&ServerEntry> {
        self.servers.iter().find(|s| s.id == id)
    }

    /// Legt einen Server an oder aktualisiert Name und Adresse (Benutzer bleiben erhalten).
    pub fn upsert_server(&mut self, id: &str, name: &str, url: &str) -> &mut ServerEntry {
        let idx = match self.servers.iter().position(|s| s.id == id) {
            Some(i) => {
                self.servers[i].name = name.to_owned();
                self.servers[i].url = url.to_owned();
                i
            }
            None => {
                self.servers.push(ServerEntry {
                    id: id.to_owned(),
                    name: name.to_owned(),
                    url: url.to_owned(),
                    users: Vec::new(),
                });
                self.servers.len() - 1
            }
        };
        &mut self.servers[idx]
    }

    /// Legt einen Benutzer an oder aktualisiert den Namen. `false`, wenn der Server unbekannt ist.
    pub fn upsert_user(&mut self, server_id: &str, user_id: &str, name: &str) -> bool {
        let Some(server) = self.servers.iter_mut().find(|s| s.id == server_id) else {
            return false;
        };
        match server.users.iter_mut().find(|u| u.id == user_id) {
            Some(user) => user.name = name.to_owned(),
            None => server.users.push(UserEntry {
                id: user_id.to_owned(),
                name: name.to_owned(),
            }),
        }
        true
    }

    /// Entfernt einen Benutzer (der Server bleibt in der Liste). `true`, wenn etwas entfernt wurde.
    pub fn remove_user(&mut self, server_id: &str, user_id: &str) -> bool {
        let removed = self
            .servers
            .iter_mut()
            .find(|s| s.id == server_id)
            .is_some_and(|s| {
                let before = s.users.len();
                s.users.retain(|u| u.id != user_id);
                s.users.len() != before
            });
        if removed
            && self
                .active
                .as_ref()
                .is_some_and(|a| a.server_id == server_id && a.user_id == user_id)
        {
            self.active = None;
        }
        removed
    }

    /// Entfernt einen Server samt Benutzern. `true`, wenn er existierte.
    pub fn remove_server(&mut self, server_id: &str) -> bool {
        let before = self.servers.len();
        self.servers.retain(|s| s.id != server_id);
        if self
            .active
            .as_ref()
            .is_some_and(|a| a.server_id == server_id)
        {
            self.active = None;
        }
        self.servers.len() != before
    }

    /// Merkt das zuletzt benutzte Konto.
    pub fn set_active(&mut self, server_id: &str, user_id: &str) {
        self.active = Some(ActiveSelection {
            server_id: server_id.to_owned(),
            user_id: user_id.to_owned(),
        });
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_stable_device_id() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("sub").join("config.json");
        let first = Config::load_or_create(&path).unwrap();
        assert!(path.is_file());
        let second = Config::load_or_create(&path).unwrap();
        assert_eq!(first.device_id, second.device_id);
        let parsed = uuid::Uuid::parse_str(&first.device_id).unwrap();
        assert_eq!(parsed.get_version_num(), 4);
    }

    #[test]
    fn save_and_reload_servers() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.json");
        let mut c = Config::load_or_create(&path).unwrap();
        c.upsert_server("s1", "Server Eins", "http://server-eins.invalid:8096");
        assert!(c.upsert_user("s1", "u1", "Anna"));
        assert!(c.upsert_user("s1", "u2", "Ben"));
        assert!(!c.upsert_user("zzz", "u9", "Nie"));
        c.set_active("s1", "u2");
        c.save(&path).unwrap();
        let back = Config::load_or_create(&path).unwrap();
        assert_eq!(back, c);
        assert_eq!(back.server("s1").unwrap().users.len(), 2);
    }

    #[test]
    fn upsert_keeps_users_and_updates_url() {
        let mut c = Config::new();
        c.upsert_server("s1", "A", "http://a.invalid");
        c.upsert_user("s1", "u1", "Anna");
        c.upsert_server("s1", "A neu", "http://b.invalid");
        let s = c.server("s1").unwrap();
        assert_eq!(
            (s.name.as_str(), s.url.as_str()),
            ("A neu", "http://b.invalid")
        );
        assert_eq!(s.users.len(), 1);
        assert_eq!(c.servers.len(), 1);
    }

    #[test]
    fn remove_user_and_server_clear_active() {
        let mut c = Config::new();
        c.upsert_server("s1", "A", "http://a.invalid");
        c.upsert_user("s1", "u1", "Anna");
        c.upsert_user("s1", "u2", "Ben");
        c.set_active("s1", "u1");
        assert!(c.remove_user("s1", "u1"));
        assert!(c.active.is_none());
        assert!(!c.remove_user("s1", "u1"));
        c.set_active("s1", "u2");
        assert!(c.remove_server("s1"));
        assert!(c.active.is_none() && c.servers.is_empty());
    }

    #[test]
    fn corrupt_file_is_not_overwritten() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.json");
        std::fs::write(&path, b"{kaputt").unwrap();
        let err = Config::load_or_create(&path).unwrap_err();
        assert!(matches!(err, ConfigError::Corrupt(_)), "{err:?}");
        assert_eq!(std::fs::read(&path).unwrap(), b"{kaputt");
    }

    #[test]
    fn newer_version_is_rejected() {
        let json = br#"{"version":99,"device_id":"x"}"#;
        assert!(matches!(
            Config::parse(json),
            Err(ConfigError::UnknownVersion(99))
        ));
    }

    #[test]
    fn atomic_save_leaves_no_temp_files() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.json");
        let mut c = Config::new();
        for i in 0..5 {
            c.upsert_server(&format!("s{i}"), "N", "http://x.invalid");
            c.save(&path).unwrap();
        }
        let files: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(files.len(), 1, "{files:?}");
    }
}
