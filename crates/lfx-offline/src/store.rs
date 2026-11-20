//! Persistenz hinter Traits: [`DownloadStore`] (Datensätze) und [`OfflineOutbox`] (Meldungen für den Abgleich),
//! je mit Speicher-Variante (Tests) und SQLite-Variante gegen `lfx-store` ([`SqliteDownloadStore`], [`SqliteOutbox`]).

use crate::error::Result;
use crate::types::DownloadRecord;
use lfx_store::{DownloadRow, NewOutboxEntry, Store};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Mutex;

/// Ablage der Download-Datensätze.
pub trait DownloadStore: Send + Sync + 'static {
    /// Speichert (ersetzt) einen Datensatz.
    fn put(&self, record: &DownloadRecord) -> Result<()>;
    /// Datensatz nach Kennung.
    fn get(&self, id: &str) -> Result<Option<DownloadRecord>>;
    /// Alle Datensätze eines Kontos in Anlegereihenfolge.
    fn list(&self, server_id: &str, user_id: &str) -> Result<Vec<DownloadRecord>>;
    /// Löscht einen Datensatz.
    fn delete(&self, id: &str) -> Result<bool>;
}

/// Nur im Arbeitsspeicher.
#[derive(Default)]
pub struct MemoryDownloadStore {
    rows: Mutex<Vec<DownloadRecord>>,
}

impl MemoryDownloadStore {
    /// Leere Ablage.
    pub fn new() -> Self {
        Self::default()
    }
}

impl DownloadStore for MemoryDownloadStore {
    fn put(&self, record: &DownloadRecord) -> Result<()> {
        let mut rows = self.rows.lock().unwrap_or_else(|e| e.into_inner());
        match rows.iter_mut().find(|r| r.id == record.id) {
            Some(slot) => *slot = record.clone(),
            None => rows.push(record.clone()),
        }
        Ok(())
    }

    fn get(&self, id: &str) -> Result<Option<DownloadRecord>> {
        let rows = self.rows.lock().unwrap_or_else(|e| e.into_inner());
        Ok(rows.iter().find(|r| r.id == id).cloned())
    }

    fn list(&self, server_id: &str, user_id: &str) -> Result<Vec<DownloadRecord>> {
        let rows = self.rows.lock().unwrap_or_else(|e| e.into_inner());
        Ok(rows
            .iter()
            .filter(|r| r.server_id == server_id && r.user_id == user_id)
            .cloned()
            .collect())
    }

    fn delete(&self, id: &str) -> Result<bool> {
        let mut rows = self.rows.lock().unwrap_or_else(|e| e.into_inner());
        let before = rows.len();
        rows.retain(|r| r.id != id);
        Ok(rows.len() != before)
    }
}

/// SQLite über `lfx-store` (Tabelle `downloads`).
#[derive(Clone)]
pub struct SqliteDownloadStore {
    store: Store,
}

impl SqliteDownloadStore {
    /// Ablage über eine geöffnete Datenbank.
    pub fn new(store: Store) -> Self {
        Self { store }
    }
}

impl DownloadStore for SqliteDownloadStore {
    fn put(&self, record: &DownloadRecord) -> Result<()> {
        self.store.download_put(&DownloadRow {
            id: record.id.clone(),
            server_id: record.server_id.clone(),
            user_id: record.user_id.clone(),
            item_id: record.item_id.clone(),
            state: record.state.as_str().to_owned(),
            json: serde_json::to_value(record)?,
            updated_at: 0,
        })?;
        Ok(())
    }

    fn get(&self, id: &str) -> Result<Option<DownloadRecord>> {
        self.store
            .download_get(id)?
            .map(|row| Ok(serde_json::from_value(row.json)?))
            .transpose()
    }

    fn list(&self, server_id: &str, user_id: &str) -> Result<Vec<DownloadRecord>> {
        self.store
            .download_list(server_id, user_id)?
            .into_iter()
            .map(|row| Ok(serde_json::from_value(row.json)?))
            .collect()
    }

    fn delete(&self, id: &str) -> Result<bool> {
        Ok(self.store.download_delete(id)?)
    }
}

/// Eintrag der Outbox, wie der Offline-Modus ihn sieht.
#[derive(Debug, Clone, PartialEq)]
pub struct OutboxItem {
    /// Laufende Nummer.
    pub id: i64,
    /// Art der Meldung (hier nur `offline.userdata`).
    pub kind: String,
    /// Inhalt.
    pub payload: Value,
    /// Bisherige Fehlversuche.
    pub attempts: u32,
}

/// Ablage der Meldungen, die offline anfallen. Die Outbox kann auch Meldungen anderer Teile der App
/// enthalten; der Offline-Modus fasst nur seine eigenen Arten an (Präfix `offline.`).
pub trait OfflineOutbox: Send + Sync + 'static {
    /// Hängt eine Meldung an, gibt die Nummer zurück.
    fn push(&self, server_id: &str, user_id: &str, kind: &str, payload: &Value) -> Result<i64>;
    /// Ausstehende Meldungen der Art `kind` in Reihenfolge.
    fn pending(&self, server_id: &str, user_id: &str, kind: &str) -> Result<Vec<OutboxItem>>;
    /// Meldung erledigt: entfernen.
    fn complete(&self, id: i64) -> Result<()>;
    /// Fehlversuch vermerken.
    fn mark_failed(&self, id: i64, error: &str) -> Result<()>;
}

type MemoryRow = (String, String, OutboxItem);

/// Outbox im Arbeitsspeicher.
#[derive(Default)]
pub struct MemoryOutbox {
    inner: Mutex<(i64, BTreeMap<i64, MemoryRow>)>,
}

impl MemoryOutbox {
    /// Leere Outbox.
    pub fn new() -> Self {
        Self::default()
    }

    /// Anzahl aller Einträge.
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).1.len()
    }

    /// Ist die Outbox leer?
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl OfflineOutbox for MemoryOutbox {
    fn push(&self, server_id: &str, user_id: &str, kind: &str, payload: &Value) -> Result<i64> {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.0 += 1;
        let id = g.0;
        let item = OutboxItem {
            id,
            kind: kind.to_owned(),
            payload: payload.clone(),
            attempts: 0,
        };
        g.1.insert(id, (server_id.to_owned(), user_id.to_owned(), item));
        Ok(id)
    }

    fn pending(&self, server_id: &str, user_id: &str, kind: &str) -> Result<Vec<OutboxItem>> {
        let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        Ok(g.1
            .values()
            .filter(|(s, u, i)| s == server_id && u == user_id && i.kind == kind)
            .map(|(_, _, i)| i.clone())
            .collect())
    }

    fn complete(&self, id: i64) -> Result<()> {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .1
            .remove(&id);
        Ok(())
    }

    fn mark_failed(&self, id: i64, _error: &str) -> Result<()> {
        if let Some((_, _, item)) = self
            .inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .1
            .get_mut(&id)
        {
            item.attempts += 1;
        }
        Ok(())
    }
}

/// Outbox über die Tabelle `outbox` von `lfx-store`.
#[derive(Clone)]
pub struct SqliteOutbox {
    store: Store,
}

impl SqliteOutbox {
    /// Outbox über eine geöffnete Datenbank.
    pub fn new(store: Store) -> Self {
        Self { store }
    }
}

impl OfflineOutbox for SqliteOutbox {
    fn push(&self, server_id: &str, user_id: &str, kind: &str, payload: &Value) -> Result<i64> {
        Ok(self.store.outbox_push(&NewOutboxEntry {
            server_id: server_id.to_owned(),
            user_id: user_id.to_owned(),
            kind: kind.to_owned(),
            payload: payload.clone(),
        })?)
    }

    fn pending(&self, server_id: &str, user_id: &str, kind: &str) -> Result<Vec<OutboxItem>> {
        // Die Tabelle kennt keinen Filter nach Art; fremde Arten werden hier übersprungen und nie angefasst.
        Ok(self
            .store
            .outbox_pending(server_id, user_id, u32::MAX)?
            .into_iter()
            .filter(|e| e.kind == kind)
            .map(|e| OutboxItem {
                id: e.id,
                kind: e.kind,
                payload: e.payload,
                attempts: e.attempts,
            })
            .collect())
    }

    fn complete(&self, id: i64) -> Result<()> {
        self.store.outbox_complete(id)?;
        Ok(())
    }

    fn mark_failed(&self, id: i64, error: &str) -> Result<()> {
        self.store.outbox_mark_failed(id, error)?;
        Ok(())
    }
}
