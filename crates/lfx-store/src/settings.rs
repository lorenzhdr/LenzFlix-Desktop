//! App-Einstellungen je Server und Benutzer (z. B. Design, Datensparmodus). Rein lokal, nichts davon
//! wird zum Server oder in die Web-DisplayPreferences geschrieben.

use crate::error::Result;
use crate::store::Store;
use rusqlite::{OptionalExtension, params};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

impl Store {
    /// Einstellung als JSON schreiben.
    pub fn set_setting(
        &self,
        server_id: &str,
        user_id: &str,
        key: &str,
        value: &Value,
    ) -> Result<()> {
        self.lock().execute(
            "INSERT INTO settings (server_id, user_id, key, json) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (server_id, user_id, key) DO UPDATE SET json = ?4",
            params![server_id, user_id, key, serde_json::to_string(value)?],
        )?;
        Ok(())
    }

    /// Einstellung als JSON lesen.
    pub fn get_setting(&self, server_id: &str, user_id: &str, key: &str) -> Result<Option<Value>> {
        let text: Option<String> = self
            .lock()
            .query_row(
                "SELECT json FROM settings WHERE server_id = ?1 AND user_id = ?2 AND key = ?3",
                [server_id, user_id, key],
                |r| r.get(0),
            )
            .optional()?;
        Ok(text.map(|t| serde_json::from_str(&t)).transpose()?)
    }

    /// Typisiert schreiben.
    pub fn set_setting_as<T: Serialize>(
        &self,
        server_id: &str,
        user_id: &str,
        key: &str,
        value: &T,
    ) -> Result<()> {
        self.set_setting(server_id, user_id, key, &serde_json::to_value(value)?)
    }

    /// Typisiert lesen. Passt der gespeicherte Wert nicht zum Typ, gibt es einen JSON-Fehler.
    pub fn get_setting_as<T: DeserializeOwned>(
        &self,
        server_id: &str,
        user_id: &str,
        key: &str,
    ) -> Result<Option<T>> {
        self.get_setting(server_id, user_id, key)?
            .map(serde_json::from_value)
            .transpose()
            .map_err(Into::into)
    }

    /// Einstellung entfernen. `true`, wenn sie existierte.
    pub fn delete_setting(&self, server_id: &str, user_id: &str, key: &str) -> Result<bool> {
        Ok(self.lock().execute(
            "DELETE FROM settings WHERE server_id = ?1 AND user_id = ?2 AND key = ?3",
            [server_id, user_id, key],
        )? > 0)
    }

    /// Alle Einstellungen eines Benutzers (nach Schlüssel sortiert).
    pub fn list_settings(&self, server_id: &str, user_id: &str) -> Result<Vec<(String, Value)>> {
        let conn = self.lock();
        let mut stmt = conn.prepare_cached(
            "SELECT key, json FROM settings WHERE server_id = ?1 AND user_id = ?2 ORDER BY key",
        )?;
        let rows = stmt.query_map([server_id, user_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (key, text) = row?;
            out.push((key, serde_json::from_str(&text)?));
        }
        Ok(out)
    }
}
