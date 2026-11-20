//! Verbindung, Öffnen und Aufräumen.

use crate::error::Result;
use crate::migrations;
use rusqlite::Connection;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

/// Sekunden seit 1970 (UTC).
pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Lokale Datenbank. Klonen ist billig (geteilte Verbindung).
#[derive(Clone)]
pub struct Store {
    conn: Arc<Mutex<Connection>>,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Store")
    }
}

impl Store {
    /// Öffnet (oder erstellt) die Datenbank-Datei, schaltet WAL ein und migriert.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        // journal_mode liefert eine Zeile zurück, daher query statt execute.
        conn.pragma_update_and_check(None, "journal_mode", "WAL", |_| Ok(()))?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        Self::init(conn)
    }

    /// Datenbank nur im Arbeitsspeicher (Tests).
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Self> {
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        migrations::migrate(&mut conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub(crate) fn lock(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Schemaversion der geöffneten Datenbank.
    pub fn schema_version(&self) -> Result<i64> {
        migrations::current_version(&self.lock())
    }

    /// Aktiver Journal-Modus (`wal` bei Dateien).
    pub fn journal_mode(&self) -> Result<String> {
        Ok(self
            .lock()
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))?)
    }

    /// Löscht alle Daten eines Servers (Katalog, Benutzerdaten, Einstellungen, Outbox, Downloads-Zeilen).
    /// Der Bild-Index ist nicht serverbezogen und bleibt.
    pub fn purge_server(&self, server_id: &str) -> Result<()> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        for table in ["items", "user_data", "settings", "outbox", "downloads"] {
            tx.execute(
                &format!("DELETE FROM {table} WHERE server_id = ?1"),
                [server_id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Löscht alle Daten eines Benutzers auf einem Server (Benutzerdaten, Einstellungen, Outbox).
    /// Der Katalog-Cache gehört dem Server und bleibt.
    pub fn purge_user(&self, server_id: &str, user_id: &str) -> Result<()> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        for table in ["user_data", "settings", "outbox", "downloads"] {
            tx.execute(
                &format!("DELETE FROM {table} WHERE server_id = ?1 AND user_id = ?2"),
                [server_id, user_id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}
