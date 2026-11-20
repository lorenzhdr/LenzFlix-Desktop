//! Tests der Download-Zeilen.

use lfx_store::{DownloadRow, Store};
use serde_json::json;

fn row(id: &str, user: &str, state: &str) -> DownloadRow {
    DownloadRow {
        id: id.to_owned(),
        server_id: "s".to_owned(),
        user_id: user.to_owned(),
        item_id: format!("i-{id}"),
        state: state.to_owned(),
        json: json!({"Name": "Teststück"}),
        updated_at: 0,
    }
}

#[test]
fn put_get_list_delete() {
    let store = Store::open_in_memory().unwrap();
    store.download_put(&row("a", "u1", "queued")).unwrap();
    store.download_put(&row("b", "u1", "queued")).unwrap();
    store.download_put(&row("c", "u2", "queued")).unwrap();
    let mut changed = row("a", "u1", "completed");
    changed.json = json!({"Name": "Neu"});
    store.download_put(&changed).unwrap();

    let a = store.download_get("a").unwrap().unwrap();
    assert_eq!(a.state, "completed");
    assert_eq!(a.json["Name"], "Neu");
    let ids: Vec<String> = store
        .download_list("s", "u1")
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect();
    assert_eq!(ids, ["a", "b"]);
    assert!(store.download_delete("a").unwrap());
    assert!(!store.download_delete("a").unwrap());
    assert!(store.download_get("a").unwrap().is_none());
}

#[test]
fn purge_user_removes_downloads() {
    let store = Store::open_in_memory().unwrap();
    store.download_put(&row("a", "u1", "queued")).unwrap();
    store.download_put(&row("c", "u2", "queued")).unwrap();
    store.purge_user("s", "u1").unwrap();
    assert!(store.download_list("s", "u1").unwrap().is_empty());
    assert_eq!(store.download_list("s", "u2").unwrap().len(), 1);
}
