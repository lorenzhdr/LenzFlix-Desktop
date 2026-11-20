//! W2-detail: Mock-Erweiterungen für Detailseiten (Personen, Taglines, Studios, Sammlungsmerkmale).
//! Alle Daten sind erfunden.

use std::collections::HashMap;

use axum::response::Response;
use serde_json::{Value, json};

use crate::catalog::{Item, SERVER_ID};
use crate::{Shared, jv};

/// Id der n-ten Person (1-basiert), gleiche Ids wie in `People` des Katalogs.
pub fn person_id(n: u128) -> String {
    format!("{:032x}", (0x60u128 << 16) + n)
}

fn person_name(id: &str) -> Option<(&'static str, &'static str)> {
    if id == person_id(1) {
        Some((
            "Mira Kessler",
            "Erfundene Schauspielerin für Testzwecke. Bekannt aus mehreren erfundenen Produktionen.",
        ))
    } else if id == person_id(2) {
        Some(("Jonas Albrecht", "Erfundener Regisseur für Testzwecke."))
    } else {
        None
    }
}

/// `GET /Items/{id}` für Personen.
pub fn try_person(id: &str) -> Option<Response> {
    let (name, bio) = person_name(id)?;
    Some(jv(json!({
        "Id": id, "Name": name, "ServerId": SERVER_ID, "Type": "Person", "Overview": bio,
        "ImageTags": {}, "UserData": { "PlaybackPositionTicks": 0, "PlayCount": 0, "IsFavorite": false, "Played": false },
    })))
}

/// `GET /Items?personIds=…`: die ersten Filme und Serien des Katalogs.
pub fn try_person_items(shared: &Shared, params: &HashMap<String, String>) -> Option<Response> {
    let pid = params.get("personids")?;
    person_name(pid)?;
    let fields = params.get("fields").map(String::as_str).unwrap_or("");
    let users = shared.lock();
    let items: Vec<Value> = shared
        .catalog
        .iter()
        .filter(|i| matches!(i.kind, "Movie" | "Series"))
        .take(5)
        .map(|i| crate::catalog::render(&shared.catalog, i, &users.users, fields))
        .collect();
    let n = items.len();
    Some(jv(
        json!({ "Items": items, "TotalRecordCount": n, "StartIndex": 0 }),
    ))
}

/// Ergänzt die gerenderten Katalogeinträge um Merkmale, die nur Detailseiten zeigen.
pub fn decorate(v: &mut Value, it: &Item, catalog: &[Item], fields: &str) {
    let want = |f: &str| fields.split(',').any(|p| p.eq_ignore_ascii_case(f));
    if matches!(it.kind, "Movie" | "Series") {
        if want("Taglines") {
            v["Taglines"] = json!(["Eine erfundene Geschichte für Testzwecke."]);
        }
        if want("Studios") {
            v["Studios"] =
                json!([{ "Name": "Nordlicht Studio", "Id": "00000000000000000000000000c00001" }]);
        }
        if want("RemoteTrailers") {
            v["RemoteTrailers"] = json!([]);
        }
        v["Width"] = json!(1920);
    }
    if it.kind == "Series" {
        let seasons = catalog
            .iter()
            .filter(|e| e.kind == "Season" && e.series_id.as_deref() == Some(&it.id))
            .count();
        let episodes = catalog
            .iter()
            .filter(|e| e.kind == "Episode" && e.series_id.as_deref() == Some(&it.id))
            .count();
        v["ChildCount"] = json!(seasons);
        v["RecursiveItemCount"] = json!(episodes);
    }
    if it.kind == "Season" {
        let episodes = catalog
            .iter()
            .filter(|e| e.kind == "Episode" && e.season_id.as_deref() == Some(&it.id))
            .count();
        v["ChildCount"] = json!(episodes);
    }
}
