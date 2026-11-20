//! Ordner der App. Standard sind die OS-üblichen Orte (App-Name „LENZFLIX“); für Tests und Entwicklung
//! lässt sich ein eigener Wurzelordner setzen ([`AppDirs::from_root`], Umgebungsvariable `LFX_HOME`).

use std::path::{Path, PathBuf};

/// App-Name für die OS-Ordner.
pub const APP_NAME: &str = "LENZFLIX";
/// Umgebungsvariable, die alle Ordner unter einen Wurzelordner legt.
pub const HOME_ENV: &str = "LFX_HOME";
/// Name der Konfigurationsdatei.
pub const CONFIG_FILE: &str = "config.json";

/// Konfig-, Cache- und Datenordner (nur Pfade, es wird nichts angelegt).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppDirs {
    /// Einstellungen und Serverliste.
    pub config: PathBuf,
    /// Wegwerfbares (Bilder, Zwischenspeicher).
    pub cache: PathBuf,
    /// Dauerhafte Daten (Offline-Downloads, lokale Datenbank).
    pub data: PathBuf,
}

impl AppDirs {
    /// OS-übliche Ordner. `None`, wenn das Betriebssystem keinen Home-Ordner liefert.
    pub fn system() -> Option<Self> {
        let dirs = directories::ProjectDirs::from("", "", APP_NAME)?;
        Some(Self {
            config: dirs.config_dir().to_path_buf(),
            cache: dirs.cache_dir().to_path_buf(),
            data: dirs.data_dir().to_path_buf(),
        })
    }

    /// Alles unter `root` (`config/`, `cache/`, `data/`).
    pub fn from_root(root: impl AsRef<Path>) -> Self {
        let root = root.as_ref();
        Self {
            config: root.join("config"),
            cache: root.join("cache"),
            data: root.join("data"),
        }
    }

    /// `LFX_HOME` hat Vorrang, sonst die OS-Ordner.
    pub fn resolve() -> Option<Self> {
        match std::env::var_os(HOME_ENV).filter(|v| !v.is_empty()) {
            Some(root) => Some(Self::from_root(root)),
            None => Self::system(),
        }
    }

    /// Pfad der Konfigurationsdatei.
    pub fn config_file(&self) -> PathBuf {
        self.config.join(CONFIG_FILE)
    }

    /// Legt alle drei Ordner an.
    pub fn create_all(&self) -> std::io::Result<()> {
        for dir in [&self.config, &self.cache, &self.data] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_layout() {
        let d = AppDirs::from_root("wurzel");
        assert_eq!(d.config, Path::new("wurzel").join("config"));
        assert_eq!(
            d.config_file(),
            Path::new("wurzel").join("config").join("config.json")
        );
    }

    #[test]
    fn system_dirs_use_app_name() {
        if let Some(d) = AppDirs::system() {
            assert!(d.config.to_string_lossy().contains(APP_NAME));
            assert!(d.cache.to_string_lossy().contains(APP_NAME));
            assert!(d.data.to_string_lossy().contains(APP_NAME));
        }
    }

    #[test]
    fn create_all_makes_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let d = AppDirs::from_root(tmp.path());
        d.create_all().unwrap();
        assert!(d.config.is_dir() && d.cache.is_dir() && d.data.is_dir());
    }
}
