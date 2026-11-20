//! Katalog-Cache (`items`) und Benutzerdaten je Konto (`user_data`).

use crate::error::Result;
use crate::store::{Store, unix_now};
use rusqlite::{OptionalExtension, params};
use serde_json::Value;

impl Store {
    /// Legt ein Element im Cache ab oder ersetzt es.
    pub fn put_item(&self, server_id: &str, item_id: &str, json: &Value) -> Result<()> {
        self.put_items(server_id, [(item_id, json)])
    }

    /// Mehrere Elemente in einer Transaktion.
    pub fn put_items<'a>(
        &self,
        server_id: &str,
        items: impl IntoIterator<Item = (&'a str, &'a Value)>,
    ) -> Result<()> {
        let now = unix_now();
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT INTO items (server_id, item_id, json, updated_at) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (server_id, item_id) DO UPDATE SET json = ?3, updated_at = ?4",
            )?;
            for (id, json) in items {
                stmt.execute(params![server_id, id, serde_json::to_string(json)?, now])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Element aus dem Cache.
    pub fn get_item(&self, server_id: &str, item_id: &str) -> Result<Option<Value>> {
        let text: Option<String> = self
            .lock()
            .query_row(
                "SELECT json FROM items WHERE server_id = ?1 AND item_id = ?2",
                [server_id, item_id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(text.map(|t| serde_json::from_str(&t)).transpose()?)
    }

    /// Zeitpunkt der letzten Aktualisierung (Unix-Sekunden).
    pub fn item_updated_at(&self, server_id: &str, item_id: &str) -> Result<Option<i64>> {
        Ok(self
            .lock()
            .query_row(
                "SELECT updated_at FROM items WHERE server_id = ?1 AND item_id = ?2",
                [server_id, item_id],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// Entfernt ein Element. `true`, wenn es existierte.
    pub fn delete_item(&self, server_id: &str, item_id: &str) -> Result<bool> {
        Ok(self.lock().execute(
            "DELETE FROM items WHERE server_id = ?1 AND item_id = ?2",
            [server_id, item_id],
        )? > 0)
    }

    /// Anzahl der Elemente eines Servers.
    pub fn count_items(&self, server_id: &str) -> Result<u64> {
        Ok(self.lock().query_row(
            "SELECT COUNT(*) FROM items WHERE server_id = ?1",
            [server_id],
            |r| r.get::<_, i64>(0),
        )? as u64)
    }

    /// Alle Elemente eines Servers (nach Id sortiert).
    pub fn list_items(&self, server_id: &str) -> Result<Vec<(String, Value)>> {
        let conn = self.lock();
        let mut stmt = conn.prepare_cached(
            "SELECT item_id, json FROM items WHERE server_id = ?1 ORDER BY item_id",
        )?;
        let rows = stmt.query_map([server_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, text) = row?;
            out.push((id, serde_json::from_str(&text)?));
        }
        Ok(out)
    }

    /// Benutzerdaten (Position, Gesehen, Favorit …) eines Elements ablegen.
    pub fn set_user_data(
        &self,
        server_id: &str,
        user_id: &str,
        item_id: &str,
        json: &Value,
    ) -> Result<()> {
        self.lock().execute(
            "INSERT INTO user_data (server_id, user_id, item_id, json, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (server_id, user_id, item_id) DO UPDATE SET json = ?4, updated_at = ?5",
            params![
                server_id,
                user_id,
                item_id,
                serde_json::to_string(json)?,
                unix_now()
            ],
        )?;
        Ok(())
    }

    /// Benutzerdaten eines Elements.
    pub fn get_user_data(
        &self,
        server_id: &str,
        user_id: &str,
        item_id: &str,
    ) -> Result<Option<Value>> {
        let text: Option<String> = self
            .lock()
            .query_row(
                "SELECT json FROM user_data WHERE server_id = ?1 AND user_id = ?2 AND item_id = ?3",
                [server_id, user_id, item_id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(text.map(|t| serde_json::from_str(&t)).transpose()?)
    }

    /// Entfernt die Benutzerdaten eines Elements. `true`, wenn sie existierten.
    pub fn delete_user_data(&self, server_id: &str, user_id: &str, item_id: &str) -> Result<bool> {
        Ok(self.lock().execute(
            "DELETE FROM user_data WHERE server_id = ?1 AND user_id = ?2 AND item_id = ?3",
            [server_id, user_id, item_id],
        )? > 0)
    }
}
