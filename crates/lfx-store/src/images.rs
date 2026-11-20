//! Verzeichnis des Bild-Caches: welche Datei gehört zu welcher URL und Größe. Die Dateien selbst liegen
//! im Cache-Ordner (verwaltet der Aufrufer); hier nur Index, Größen und Zugriffszeit für die LRU-Räumung.

use crate::error::Result;
use crate::store::{Store, unix_now};
use rusqlite::{OptionalExtension, params};

/// Eintrag im Bild-Index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageEntry {
    /// Schlüssel der Bild-URL (ohne Token, z. B. Hash).
    pub url_key: String,
    /// Größenstufe (z. B. `w320`).
    pub size: String,
    /// Dateipfad im Cache-Ordner.
    pub path: String,
    /// Dateigröße in Bytes.
    pub bytes: u64,
    /// Letzter Zugriff (Unix-Sekunden).
    pub last_access: i64,
}

impl Store {
    /// Trägt eine Bilddatei ein oder ersetzt den Eintrag.
    pub fn image_put(&self, entry: &ImageEntry) -> Result<()> {
        self.lock().execute(
            "INSERT INTO image_index (url_key, size, path, bytes, last_access)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (url_key, size) DO UPDATE SET path = ?3, bytes = ?4, last_access = ?5",
            params![
                entry.url_key,
                entry.size,
                entry.path,
                entry.bytes as i64,
                entry.last_access
            ],
        )?;
        Ok(())
    }

    /// Sucht eine Bilddatei und setzt dabei den Zugriff auf jetzt.
    pub fn image_get(&self, url_key: &str, size: &str) -> Result<Option<ImageEntry>> {
        let now = unix_now();
        let conn = self.lock();
        let found = conn
            .query_row(
                "SELECT path, bytes FROM image_index WHERE url_key = ?1 AND size = ?2",
                [url_key, size],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
            )
            .optional()?;
        let Some((path, bytes)) = found else {
            return Ok(None);
        };
        conn.execute(
            "UPDATE image_index SET last_access = ?3 WHERE url_key = ?1 AND size = ?2",
            params![url_key, size, now],
        )?;
        Ok(Some(ImageEntry {
            url_key: url_key.to_owned(),
            size: size.to_owned(),
            path,
            bytes: bytes as u64,
            last_access: now,
        }))
    }

    /// Entfernt einen Eintrag (die Datei löscht der Aufrufer). `true`, wenn er existierte.
    pub fn image_remove(&self, url_key: &str, size: &str) -> Result<bool> {
        Ok(self.lock().execute(
            "DELETE FROM image_index WHERE url_key = ?1 AND size = ?2",
            [url_key, size],
        )? > 0)
    }

    /// Summe aller Dateigrößen im Index.
    pub fn image_total_bytes(&self) -> Result<u64> {
        Ok(self
            .lock()
            .query_row("SELECT COALESCE(SUM(bytes), 0) FROM image_index", [], |r| {
                r.get::<_, i64>(0)
            })? as u64)
    }

    /// Räumt die am längsten nicht benutzten Einträge aus dem Index, bis die Summe höchstens
    /// `max_bytes` beträgt. Gibt die entfernten Einträge zurück, damit der Aufrufer die Dateien löscht.
    pub fn image_evict_lru(&self, max_bytes: u64) -> Result<Vec<ImageEntry>> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        let mut total: i64 =
            tx.query_row("SELECT COALESCE(SUM(bytes), 0) FROM image_index", [], |r| {
                r.get(0)
            })?;
        let mut removed = Vec::new();
        {
            let mut stmt = tx.prepare(
                "SELECT url_key, size, path, bytes, last_access FROM image_index
                 ORDER BY last_access ASC, url_key ASC, size ASC",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok(ImageEntry {
                    url_key: r.get(0)?,
                    size: r.get(1)?,
                    path: r.get(2)?,
                    bytes: r.get::<_, i64>(3)? as u64,
                    last_access: r.get(4)?,
                })
            })?;
            for row in rows {
                if total as u64 <= max_bytes {
                    break;
                }
                let entry = row?;
                total -= entry.bytes as i64;
                removed.push(entry);
            }
        }
        for entry in &removed {
            tx.execute(
                "DELETE FROM image_index WHERE url_key = ?1 AND size = ?2",
                [&entry.url_key, &entry.size],
            )?;
        }
        tx.commit()?;
        Ok(removed)
    }
}
