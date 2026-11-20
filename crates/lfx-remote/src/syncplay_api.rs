//! REST-Aufrufe der SyncPlay-Gruppenverwaltung (`/SyncPlay/*`) und Anmeldung der Fähigkeiten.

use crate::capabilities::Capabilities;
use crate::enums::{GroupQueueMode, GroupRepeatMode, GroupShuffleMode};
use crate::message::GroupInfo;
use crate::syncplay::ReadyRequest;
use lfx_api::{ApiError, Client};
use serde::Serialize;
use serde_json::{Value, json};

/// Dünne Hülle um die SyncPlay-Endpunkte. Klone teilen den [`Client`].
#[derive(Clone)]
pub struct SyncPlayApi {
    client: Client,
}

impl std::fmt::Debug for SyncPlayApi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SyncPlayApi")
    }
}

type Result<T> = std::result::Result<T, ApiError>;

fn post_method() -> reqwest::Method {
    reqwest::Method::POST
}

impl SyncPlayApi {
    /// Neu mit angemeldetem Client.
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    async fn post(&self, path: &str, body: Value) -> Result<()> {
        self.client
            .call_empty(post_method(), path, &[], Some(&body))
            .await
    }

    /// Alle Gruppen, denen der Benutzer beitreten darf.
    pub async fn list_groups(&self) -> Result<Vec<GroupInfo>> {
        self.client.get_json("/SyncPlay/List", &[]).await
    }

    /// Eine Gruppe.
    pub async fn group(&self, id: &str) -> Result<GroupInfo> {
        self.client.get_json(&format!("/SyncPlay/{id}"), &[]).await
    }

    /// Neue Gruppe anlegen (der Ersteller tritt automatisch bei).
    pub async fn new_group(&self, name: &str) -> Result<()> {
        self.post("/SyncPlay/New", json!({ "GroupName": name }))
            .await
    }

    /// Gruppe beitreten.
    pub async fn join(&self, group_id: &str) -> Result<()> {
        self.post("/SyncPlay/Join", json!({ "GroupId": group_id }))
            .await
    }

    /// Gruppe verlassen.
    pub async fn leave(&self) -> Result<()> {
        self.post("/SyncPlay/Leave", json!({})).await
    }

    /// Gruppe pausieren.
    pub async fn pause(&self) -> Result<()> {
        self.post("/SyncPlay/Pause", json!({})).await
    }

    /// Gruppe fortsetzen.
    pub async fn unpause(&self) -> Result<()> {
        self.post("/SyncPlay/Unpause", json!({})).await
    }

    /// Gruppe stoppen.
    pub async fn stop(&self) -> Result<()> {
        self.post("/SyncPlay/Stop", json!({})).await
    }

    /// Gruppe an Position springen lassen.
    pub async fn seek(&self, position_ticks: i64) -> Result<()> {
        self.post("/SyncPlay/Seek", json!({ "PositionTicks": position_ticks }))
            .await
    }

    /// Neue Wiedergabeliste setzen.
    pub async fn set_new_queue(
        &self,
        item_ids: &[String],
        playing_item_position: i32,
        start_position_ticks: i64,
    ) -> Result<()> {
        self.post(
            "/SyncPlay/SetNewQueue",
            json!({ "PlayingQueue": item_ids, "PlayingItemPosition": playing_item_position,
                    "StartPositionTicks": start_position_ticks }),
        )
        .await
    }

    /// Elemente einreihen.
    pub async fn queue(&self, item_ids: &[String], mode: GroupQueueMode) -> Result<()> {
        self.post(
            "/SyncPlay/Queue",
            json!({ "ItemIds": item_ids, "Mode": mode }),
        )
        .await
    }

    /// Laufendes Element wählen.
    pub async fn set_playlist_item(&self, playlist_item_id: &str) -> Result<()> {
        self.post(
            "/SyncPlay/SetPlaylistItem",
            json!({ "PlaylistItemId": playlist_item_id }),
        )
        .await
    }

    /// Nächstes Element (Id des gerade laufenden Eintrags).
    pub async fn next_item(&self, playlist_item_id: &str) -> Result<()> {
        self.post(
            "/SyncPlay/NextItem",
            json!({ "PlaylistItemId": playlist_item_id }),
        )
        .await
    }

    /// Vorheriges Element.
    pub async fn previous_item(&self, playlist_item_id: &str) -> Result<()> {
        self.post(
            "/SyncPlay/PreviousItem",
            json!({ "PlaylistItemId": playlist_item_id }),
        )
        .await
    }

    /// Einträge entfernen oder die Liste leeren.
    pub async fn remove_from_playlist(
        &self,
        playlist_item_ids: &[String],
        clear_playlist: bool,
        clear_playing_item: bool,
    ) -> Result<()> {
        self.post(
            "/SyncPlay/RemoveFromPlaylist",
            json!({ "PlaylistItemIds": playlist_item_ids, "ClearPlaylist": clear_playlist,
                    "ClearPlayingItem": clear_playing_item }),
        )
        .await
    }

    /// Eintrag verschieben.
    pub async fn move_playlist_item(&self, playlist_item_id: &str, new_index: i32) -> Result<()> {
        self.post(
            "/SyncPlay/MovePlaylistItem",
            json!({ "PlaylistItemId": playlist_item_id, "NewIndex": new_index }),
        )
        .await
    }

    /// Wiederholmodus.
    pub async fn set_repeat_mode(&self, mode: GroupRepeatMode) -> Result<()> {
        self.post("/SyncPlay/SetRepeatMode", json!({ "Mode": mode }))
            .await
    }

    /// Mischmodus.
    pub async fn set_shuffle_mode(&self, mode: GroupShuffleMode) -> Result<()> {
        self.post("/SyncPlay/SetShuffleMode", json!({ "Mode": mode }))
            .await
    }

    /// Meldet „bereit“.
    pub async fn ready(&self, req: &ReadyRequest) -> Result<()> {
        self.post("/SyncPlay/Ready", to_value(req)).await
    }

    /// Meldet „puffert“.
    pub async fn buffering(&self, req: &ReadyRequest) -> Result<()> {
        self.post("/SyncPlay/Buffering", to_value(req)).await
    }

    /// Meldet die eigene Laufzeit zum Server in ms.
    pub async fn ping(&self, ping_ms: i64) -> Result<()> {
        self.post("/SyncPlay/Ping", json!({ "Ping": ping_ms }))
            .await
    }

    /// Gruppe soll auf diesen Client nicht mehr warten.
    pub async fn set_ignore_wait(&self, ignore: bool) -> Result<()> {
        self.post("/SyncPlay/SetIgnoreWait", json!({ "IgnoreWait": ignore }))
            .await
    }

    /// Meldet Fähigkeiten (`POST /Sessions/Capabilities/Full`).
    pub async fn report_capabilities(&self, caps: &Capabilities) -> Result<()> {
        self.client
            .call_empty(
                post_method(),
                "/Sessions/Capabilities/Full",
                &[],
                Some(caps),
            )
            .await
    }
}

fn to_value(v: &impl Serialize) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}
