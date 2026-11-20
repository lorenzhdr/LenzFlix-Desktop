//! Offline-Downloads: Zeilen der Tabelle `downloads` (Platzhalterschema aus Migration 1, keine neue
//! Migration nötig). Der Inhalt (`json`) gehört dem Offline-Modus (`lfx-offline`); hier liegen nur
//! Schlüssel, Zustand als Text und Zeitstempel.

use crate::error::Result;
use crate::store::{Store, unix_now};
use rusqlite::{OptionalExtension, params};
use serde_json::Value;

/// Eine Download-Zeile.
#[derive(Debug, Clone, PartialEq)]
pub struct DownloadRow {
    /// Eindeutige Kennung (vom Aufrufer vergeben, über alle Konten eindeutig).
    pub id: String,
    /// Server.
    pub server_id: String,
    /// Benutzer.
    pub user_id: String,
    /// Element auf dem Server.
    pub item_id: String,
    /// Zustand als Kurztext (`queued`, `downloading`, `paused`, `completed`, `failed`).
    pub state: String,
    /// Inhalt, vom Offline-Modus gedeutet.
    pub json: Value,
    /// Zuletzt geändert (Unix-Sekunden, wird beim Schreiben gesetzt).
    pub updated_at: i64,
}

const COLUMNS: &str = "id, server_id, user_id, item_id, state, json, updated_at";

fn row_to_download(r: &rusqlite::Row<'_>) -> rusqlite::Result<(DownloadRow, String)> {
    let json: String = r.get(5)?;
    Ok((
        DownloadRow {
            id: r.get(0)?,
            server_id: r.get(1)?,
            user_id: r.get(2)?,
            item_id: r.get(3)?,
            state: r.get(4)?,
            json: Value::Null,
            updated_at: r.get(6)?,
        },
        json,
    ))
}

fn decode((mut row, json): (DownloadRow, String)) -> Result<DownloadRow> {
    row.json = serde_json::from_str(&json)?;
    Ok(row)
}

impl Store {
    /// Legt eine Download-Zeile an oder ersetzt sie (nach `id`).
    pub fn download_put(&self, row: &DownloadRow) -> Result<()> {
        self.lock().execute(
            "INSERT INTO downloads (id, server_id, user_id, item_id, state, json, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT (id) DO UPDATE SET server_id = ?2, user_id = ?3, item_id = ?4,
                 state = ?5, json = ?6, updated_at = ?7",
            params![
                row.id,
                row.server_id,
                row.user_id,
                row.item_id,
                row.state,
                serde_json::to_string(&row.json)?,
                unix_now()
            ],
        )?;
        Ok(())
    }

    /// Download-Zeile nach Kennung.
    pub fn download_get(&self, id: &str) -> Result<Option<DownloadRow>> {
        let found = self
            .lock()
            .query_row(
                &format!("SELECT {COLUMNS} FROM downloads WHERE id = ?1"),
                [id],
                row_to_download,
            )
            .optional()?;
        found.map(decode).transpose()
    }

    /// Alle Downloads eines Kontos, älteste zuerst (Anlegereihenfolge über `rowid`).
    pub fn download_list(&self, server_id: &str, user_id: &str) -> Result<Vec<DownloadRow>> {
        let conn = self.lock();
        let mut stmt = conn.prepare_cached(&format!(
            "SELECT {COLUMNS} FROM downloads WHERE server_id = ?1 AND user_id = ?2 ORDER BY rowid"
        ))?;
        let rows = stmt.query_map([server_id, user_id], row_to_download)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(decode(row?)?);
        }
        Ok(out)
    }

    /// Löscht eine Download-Zeile. `true`, wenn sie existierte. Dateien löscht der Aufrufer.
    pub fn download_delete(&self, id: &str) -> Result<bool> {
        Ok(self
            .lock()
            .execute("DELETE FROM downloads WHERE id = ?1", [id])?
            > 0)
    }
}
