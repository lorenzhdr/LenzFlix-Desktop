//! Kleiner Migrator: jede Migration ist ein SQL-Skript, die Reihenfolge der Liste ist die Version
//! (`PRAGMA user_version` = Anzahl angewendeter Migrationen). Jede läuft in einer eigenen Transaktion.
//! Bestehende Einträge der Liste nie ändern, nur anhängen.

use crate::error::{Result, StoreError};
use rusqlite::Connection;

/// Alle Migrationen in Reihenfolge.
pub const MIGRATIONS: &[&str] = &[
    // 1: Grundschema
    "
    CREATE TABLE items (
        server_id  TEXT NOT NULL,
        item_id    TEXT NOT NULL,
        json       TEXT NOT NULL,
        updated_at INTEGER NOT NULL,
        PRIMARY KEY (server_id, item_id)
    );
    CREATE TABLE user_data (
        server_id  TEXT NOT NULL,
        user_id    TEXT NOT NULL,
        item_id    TEXT NOT NULL,
        json       TEXT NOT NULL,
        updated_at INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (server_id, user_id, item_id)
    );
    CREATE TABLE settings (
        server_id TEXT NOT NULL,
        user_id   TEXT NOT NULL,
        key       TEXT NOT NULL,
        json      TEXT NOT NULL,
        PRIMARY KEY (server_id, user_id, key)
    );
    CREATE TABLE outbox (
        id              INTEGER PRIMARY KEY AUTOINCREMENT,
        server_id       TEXT NOT NULL,
        user_id         TEXT NOT NULL,
        kind            TEXT NOT NULL,
        payload         TEXT NOT NULL,
        created_at      INTEGER NOT NULL,
        attempts        INTEGER NOT NULL DEFAULT 0,
        last_attempt_at INTEGER,
        last_error      TEXT
    );
    CREATE INDEX outbox_by_account ON outbox (server_id, user_id, id);
    CREATE TABLE image_index (
        url_key     TEXT NOT NULL,
        size        TEXT NOT NULL,
        path        TEXT NOT NULL,
        bytes       INTEGER NOT NULL,
        last_access INTEGER NOT NULL,
        PRIMARY KEY (url_key, size)
    );
    CREATE INDEX image_by_access ON image_index (last_access);
    -- Platzhalter für den Offline-Modus (W5); Spalten werden dort per neuer Migration ergänzt.
    CREATE TABLE downloads (
        id         TEXT PRIMARY KEY,
        server_id  TEXT NOT NULL,
        user_id    TEXT NOT NULL,
        item_id    TEXT NOT NULL,
        state      TEXT NOT NULL DEFAULT 'pending',
        json       TEXT NOT NULL DEFAULT '{}',
        updated_at INTEGER NOT NULL DEFAULT 0
    );
    ",
];

/// Höchste bekannte Schemaversion.
pub fn latest_version() -> i64 {
    MIGRATIONS.len() as i64
}

/// Aktuelle Version der Datenbank.
pub fn current_version(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row("PRAGMA user_version", [], |r| r.get(0))?)
}

/// Wendet alle fehlenden Migrationen an. Eine neuere Datenbank wird nie angefasst.
pub fn migrate(conn: &mut Connection) -> Result<()> {
    migrate_with(conn, MIGRATIONS)
}

/// Wie [`migrate`] mit eigener Liste (Tests).
pub fn migrate_with(conn: &mut Connection, migrations: &[&str]) -> Result<()> {
    let known = migrations.len() as i64;
    let found = current_version(conn)?;
    if found > known {
        return Err(StoreError::NewerSchema { found, known });
    }
    for (index, sql) in migrations.iter().enumerate().skip(found as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", index as i64 + 1)?;
        tx.commit()?;
    }
    Ok(())
}
