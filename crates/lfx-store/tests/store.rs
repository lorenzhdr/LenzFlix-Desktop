//! Tests der lokalen Datenbank (temporäre Dateien).

use lfx_store::migrations::{self, MIGRATIONS};
use lfx_store::{ImageEntry, NewOutboxEntry, Store, StoreError};
use rusqlite::Connection;
use serde_json::json;

fn temp_store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("sub").join("lfx.db")).unwrap();
    (dir, store)
}

#[test]
fn open_creates_file_wal_and_schema() {
    let (dir, store) = temp_store();
    assert!(dir.path().join("sub").join("lfx.db").is_file());
    assert_eq!(store.journal_mode().unwrap(), "wal");
    assert_eq!(
        store.schema_version().unwrap(),
        migrations::latest_version()
    );
}

#[test]
fn reopen_keeps_data_and_does_not_remigrate() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lfx.db");
    {
        let s = Store::open(&path).unwrap();
        s.put_item("s", "i", &json!({"Name": "Teststück"})).unwrap();
    }
    let s = Store::open(&path).unwrap();
    assert_eq!(s.get_item("s", "i").unwrap().unwrap()["Name"], "Teststück");
}

#[test]
fn migrator_applies_stepwise_and_rejects_newer() {
    let mut conn = Connection::open_in_memory().unwrap();
    migrations::migrate_with(&mut conn, &["CREATE TABLE a (x INTEGER);"]).unwrap();
    assert_eq!(migrations::current_version(&conn).unwrap(), 1);
    migrations::migrate_with(
        &mut conn,
        &[
            "CREATE TABLE a (x INTEGER);",
            "ALTER TABLE a ADD COLUMN y TEXT;",
        ],
    )
    .unwrap();
    assert_eq!(migrations::current_version(&conn).unwrap(), 2);
    conn.execute("INSERT INTO a (x, y) VALUES (1, 'k')", [])
        .unwrap();
    let err = migrations::migrate_with(&mut conn, &["CREATE TABLE a (x INTEGER);"]).unwrap_err();
    assert!(matches!(
        err,
        StoreError::NewerSchema { found: 2, known: 1 }
    ));
}

#[test]
fn failed_migration_rolls_back() {
    let mut conn = Connection::open_in_memory().unwrap();
    let res = migrations::migrate_with(
        &mut conn,
        &["CREATE TABLE a (x INTEGER); INSERT INTO gibt_es_nicht VALUES (1);"],
    );
    assert!(res.is_err());
    assert_eq!(migrations::current_version(&conn).unwrap(), 0);
    let n: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name = 'a'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 0);
    assert!(!MIGRATIONS.is_empty());
}

#[test]
fn newer_database_file_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lfx.db");
    {
        let conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "user_version", 999).unwrap();
    }
    assert!(matches!(
        Store::open(&path),
        Err(StoreError::NewerSchema { .. })
    ));
}

#[test]
fn items_roundtrip_and_isolation() {
    let (_d, s) = temp_store();
    s.put_item("a", "1", &json!({"n": 1})).unwrap();
    s.put_item("a", "1", &json!({"n": 2})).unwrap();
    s.put_item("b", "1", &json!({"n": 9})).unwrap();
    assert_eq!(s.get_item("a", "1").unwrap().unwrap()["n"], 2);
    assert_eq!(s.get_item("b", "1").unwrap().unwrap()["n"], 9);
    assert!(s.get_item("a", "zz").unwrap().is_none());
    assert_eq!(s.count_items("a").unwrap(), 1);
    assert!(s.item_updated_at("a", "1").unwrap().unwrap() > 0);
    let v1 = json!({"x": 1});
    let v2 = json!({"x": 2});
    s.put_items("a", [("2", &v1), ("3", &v2)]).unwrap();
    assert_eq!(s.list_items("a").unwrap().len(), 3);
    assert!(s.delete_item("a", "2").unwrap());
    assert!(!s.delete_item("a", "2").unwrap());
}

#[test]
fn user_data_is_per_user() {
    let (_d, s) = temp_store();
    s.set_user_data("s", "u1", "i", &json!({"Played": true}))
        .unwrap();
    s.set_user_data("s", "u2", "i", &json!({"Played": false}))
        .unwrap();
    assert_eq!(
        s.get_user_data("s", "u1", "i").unwrap().unwrap()["Played"],
        true
    );
    assert_eq!(
        s.get_user_data("s", "u2", "i").unwrap().unwrap()["Played"],
        false
    );
    assert!(s.delete_user_data("s", "u1", "i").unwrap());
    assert!(s.get_user_data("s", "u1", "i").unwrap().is_none());
}

#[test]
fn settings_typed_and_per_user() {
    let (_d, s) = temp_store();
    s.set_setting_as("s", "u1", "design", &"plus").unwrap();
    s.set_setting_as("s", "u1", "datasaver", &true).unwrap();
    s.set_setting_as("s", "u2", "design", &"retro").unwrap();
    assert_eq!(
        s.get_setting_as::<String>("s", "u1", "design")
            .unwrap()
            .as_deref(),
        Some("plus")
    );
    assert_eq!(
        s.get_setting_as::<String>("s", "u2", "design")
            .unwrap()
            .as_deref(),
        Some("retro")
    );
    assert_eq!(s.list_settings("s", "u1").unwrap().len(), 2);
    assert!(
        s.get_setting_as::<bool>("s", "u1", "fehlt")
            .unwrap()
            .is_none()
    );
    assert!(s.get_setting_as::<bool>("s", "u1", "design").is_err());
    assert!(s.delete_setting("s", "u1", "design").unwrap());
    assert!(s.get_setting("s", "u1", "design").unwrap().is_none());
}

#[test]
fn outbox_order_attempts_and_complete() {
    let (_d, s) = temp_store();
    let mk = |kind: &str, user: &str| NewOutboxEntry {
        server_id: "s".into(),
        user_id: user.into(),
        kind: kind.into(),
        payload: json!({"k": kind}),
    };
    let a = s.outbox_push(&mk("progress", "u1")).unwrap();
    let b = s.outbox_push(&mk("played", "u1")).unwrap();
    s.outbox_push(&mk("favorite", "u2")).unwrap();
    let pending = s.outbox_pending("s", "u1", 10).unwrap();
    assert_eq!(pending.iter().map(|e| e.id).collect::<Vec<_>>(), vec![a, b]);
    assert_eq!(pending[0].attempts, 0);
    assert_eq!(pending[0].payload["k"], "progress");
    s.outbox_mark_failed(a, "Zeitüberschreitung").unwrap();
    s.outbox_mark_failed(a, "Zeitüberschreitung").unwrap();
    let e = s.outbox_get(a).unwrap().unwrap();
    assert_eq!(e.attempts, 2);
    assert!(e.last_attempt_at.is_some());
    assert_eq!(e.last_error.as_deref(), Some("Zeitüberschreitung"));
    assert_eq!(s.outbox_pending("s", "u1", 1).unwrap().len(), 1);
    assert!(s.outbox_complete(b).unwrap());
    assert_eq!(s.outbox_len().unwrap(), 2);
    assert_eq!(s.outbox_drop_failed(2).unwrap(), 1);
    assert_eq!(s.outbox_len().unwrap(), 1);
}

#[test]
fn image_index_lru_eviction() {
    let (_d, s) = temp_store();
    let put = |key: &str, bytes: u64, t: i64| {
        s.image_put(&ImageEntry {
            url_key: key.into(),
            size: "w320".into(),
            path: format!("{key}.img"),
            bytes,
            last_access: t,
        })
        .unwrap();
    };
    put("alt", 400, 10);
    put("mittel", 400, 20);
    put("neu", 400, 30);
    assert_eq!(s.image_total_bytes().unwrap(), 1200);
    let removed = s.image_evict_lru(1000).unwrap();
    assert_eq!(removed.len(), 1);
    assert_eq!(removed[0].url_key, "alt");
    assert_eq!(removed[0].path, "alt.img");
    assert_eq!(s.image_total_bytes().unwrap(), 800);
    assert!(s.image_evict_lru(1000).unwrap().is_empty());
    // Zugriff aktualisiert die Zeit: "mittel" wird jetzt frisch, "neu" (t=30) ist älter als jetzt.
    assert!(s.image_get("mittel", "w320").unwrap().is_some());
    let removed = s.image_evict_lru(400).unwrap();
    assert_eq!(removed[0].url_key, "neu");
    assert!(s.image_get("neu", "w320").unwrap().is_none());
    assert!(s.image_remove("mittel", "w320").unwrap());
    assert_eq!(s.image_total_bytes().unwrap(), 0);
}

#[test]
fn purge_server_and_user() {
    let (_d, s) = temp_store();
    for srv in ["a", "b"] {
        s.put_item(srv, "i", &json!(1)).unwrap();
        s.set_user_data(srv, "u", "i", &json!(1)).unwrap();
        s.set_setting(srv, "u", "k", &json!(1)).unwrap();
        s.outbox_push(&NewOutboxEntry {
            server_id: srv.into(),
            user_id: "u".into(),
            kind: "x".into(),
            payload: json!(null),
        })
        .unwrap();
    }
    s.purge_user("a", "u").unwrap();
    assert!(s.get_item("a", "i").unwrap().is_some());
    assert!(s.get_user_data("a", "u", "i").unwrap().is_none());
    assert!(s.get_setting("a", "u", "k").unwrap().is_none());
    s.purge_server("a").unwrap();
    assert!(s.get_item("a", "i").unwrap().is_none());
    assert!(s.get_item("b", "i").unwrap().is_some());
    assert_eq!(s.outbox_len().unwrap(), 1);
}

#[test]
fn store_is_send_sync_and_shared_across_threads() {
    fn assert_ss<T: Send + Sync + Clone>() {}
    assert_ss::<Store>();
    let (_d, s) = temp_store();
    let handles: Vec<_> = (0..4)
        .map(|t| {
            let s = s.clone();
            std::thread::spawn(move || {
                for i in 0..25 {
                    s.put_item("s", &format!("{t}-{i}"), &json!(i)).unwrap();
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(s.count_items("s").unwrap(), 100);
}
