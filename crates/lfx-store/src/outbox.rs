//! Outbox: Server-Meldungen (Fortschritt, Gesehen, Favorit …), die offline anfallen und später gesendet
//! werden. Die Reihenfolge ist die Einfügereihenfolge (`id`).

use crate::error::Result;
use crate::store::{Store, unix_now};
use rusqlite::{OptionalExtension, params};
use serde_json::Value;

/// Neuer Eintrag.
#[derive(Debug, Clone, PartialEq)]
pub struct NewOutboxEntry {
    /// Server.
    pub server_id: String,
    /// Benutzer, in dessen Namen gemeldet wird.
    pub user_id: String,
    /// Art der Meldung (z. B. `progress`, `played`, `favorite`), vom Aufrufer festgelegt.
    pub kind: String,
    /// Inhalt der Meldung.
    pub payload: Value,
}

/// Gespeicherter Eintrag.
#[derive(Debug, Clone, PartialEq)]
pub struct OutboxEntry {
    /// Laufende Nummer.
    pub id: i64,
    /// Server.
    pub server_id: String,
    /// Benutzer.
    pub user_id: String,
    /// Art der Meldung.
    pub kind: String,
    /// Inhalt der Meldung.
    pub payload: Value,
    /// Angelegt (Unix-Sekunden).
    pub created_at: i64,
    /// Bisherige Sendeversuche.
    pub attempts: u32,
    /// Zeitpunkt des letzten Versuchs.
    pub last_attempt_at: Option<i64>,
    /// Kurzer Fehlertext des letzten Versuchs (ohne Token/URL).
    pub last_error: Option<String>,
}

const COLUMNS: &str =
    "id, server_id, user_id, kind, created_at, payload, attempts, last_attempt_at, last_error";

fn row_to_entry(r: &rusqlite::Row<'_>) -> rusqlite::Result<(OutboxEntry, String)> {
    let payload: String = r.get(5)?;
    Ok((
        OutboxEntry {
            id: r.get(0)?,
            server_id: r.get(1)?,
            user_id: r.get(2)?,
            kind: r.get(3)?,
            payload: Value::Null,
            created_at: r.get(4)?,
            attempts: r.get::<_, i64>(6)? as u32,
            last_attempt_at: r.get(7)?,
            last_error: r.get(8)?,
        },
        payload,
    ))
}

impl Store {
    /// Hängt eine Meldung an. Gibt die Nummer zurück.
    pub fn outbox_push(&self, entry: &NewOutboxEntry) -> Result<i64> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO outbox (server_id, user_id, kind, payload, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                entry.server_id,
                entry.user_id,
                entry.kind,
                serde_json::to_string(&entry.payload)?,
                unix_now()
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Ausstehende Meldungen eines Kontos in Reihenfolge, höchstens `limit`.
    pub fn outbox_pending(
        &self,
        server_id: &str,
        user_id: &str,
        limit: u32,
    ) -> Result<Vec<OutboxEntry>> {
        let conn = self.lock();
        let mut stmt = conn.prepare_cached(&format!(
            "SELECT {COLUMNS} FROM outbox WHERE server_id = ?1 AND user_id = ?2 ORDER BY id LIMIT ?3"
        ))?;
        let rows = stmt.query_map(params![server_id, user_id, limit], row_to_entry)?;
        let mut out = Vec::new();
        for row in rows {
            let (mut entry, payload) = row?;
            entry.payload = serde_json::from_str(&payload)?;
            out.push(entry);
        }
        Ok(out)
    }

    /// Eintrag nach Nummer.
    pub fn outbox_get(&self, id: i64) -> Result<Option<OutboxEntry>> {
        let found = self
            .lock()
            .query_row(
                &format!("SELECT {COLUMNS} FROM outbox WHERE id = ?1"),
                [id],
                row_to_entry,
            )
            .optional()?;
        match found {
            Some((mut entry, payload)) => {
                entry.payload = serde_json::from_str(&payload)?;
                Ok(Some(entry))
            }
            None => Ok(None),
        }
    }

    /// Anzahl ausstehender Meldungen (alle Konten).
    pub fn outbox_len(&self) -> Result<u64> {
        Ok(self
            .lock()
            .query_row("SELECT COUNT(*) FROM outbox", [], |r| r.get::<_, i64>(0))?
            as u64)
    }

    /// Vermerkt einen fehlgeschlagenen Versuch: Zähler +1, Zeitstempel und Fehlertext.
    pub fn outbox_mark_failed(&self, id: i64, error: &str) -> Result<()> {
        self.lock().execute(
            "UPDATE outbox SET attempts = attempts + 1, last_attempt_at = ?2, last_error = ?3
             WHERE id = ?1",
            params![id, unix_now(), error],
        )?;
        Ok(())
    }

    /// Meldung ist gesendet: Eintrag löschen. `true`, wenn er existierte.
    pub fn outbox_complete(&self, id: i64) -> Result<bool> {
        Ok(self
            .lock()
            .execute("DELETE FROM outbox WHERE id = ?1", [id])?
            > 0)
    }

    /// Verwirft Meldungen mit mindestens `max_attempts` Fehlversuchen; gibt die Anzahl zurück.
    pub fn outbox_drop_failed(&self, max_attempts: u32) -> Result<u64> {
        Ok(self
            .lock()
            .execute("DELETE FROM outbox WHERE attempts >= ?1", [max_attempts])? as u64)
    }
}
