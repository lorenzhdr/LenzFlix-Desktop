//! Abgleich der Outbox beim Online-Werden. Regel: **neueres `LastPlayedDate` gewinnt**.
//!
//! Je Titel zählt nur die neueste Meldung (ältere sind überholt). Vor dem Senden holt der Abgleich die
//! Benutzerdaten des Servers: ist der Server-Stand neuer oder gleich alt, wird die lokale Meldung verworfen und
//! der lokale Datensatz auf den Server-Stand gesetzt; sonst geht Position/Gesehen mit dem lokalen Zeitpunkt
//! (nicht „jetzt“) an den Server. Fremde Outbox-Arten bleiben unberührt.

use crate::error::{OfflineError, Result};
use crate::library::{OUTBOX_KIND, OfflineLibrary};
use crate::store::{OfflineOutbox, OutboxItem};
use crate::time::{iso_from_millis, millis_from_iso};
use crate::types::LocalUserData;
use lfx_api::{ApiError, Client};
use reqwest::Method;
use serde_json::{Value, json};
use std::sync::Arc;

/// Ergebnis eines Abgleichs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SyncReport {
    /// Titel, deren lokaler Stand an den Server ging.
    pub pushed: usize,
    /// Titel, bei denen der Server-Stand neuer war (lokal verworfen).
    pub kept_server: usize,
    /// Titel, die es auf dem Server nicht mehr gibt (Meldung verworfen).
    pub dropped: usize,
    /// Überholte Meldungen, die ohne Senden entfernt wurden.
    pub superseded: usize,
    /// Titel, die wegen Netzfehlern liegen blieben (nächster Abgleich versucht es erneut).
    pub failed: usize,
    /// Nach „gesehen“ automatisch gelöschte Titel.
    pub auto_deleted: usize,
}

/// Gleicht die Outbox mit dem Server ab.
pub struct OutboxSyncer {
    client: Client,
    library: OfflineLibrary,
    outbox: Arc<dyn OfflineOutbox>,
    server_id: String,
    user_id: String,
}

fn item_of(entry: &OutboxItem) -> Option<&str> {
    entry.payload.get("item_id").and_then(Value::as_str)
}

fn stamp(entry: &OutboxItem) -> i64 {
    entry
        .payload
        .get("last_played_ms")
        .and_then(Value::as_i64)
        .unwrap_or(0)
}

impl OutboxSyncer {
    /// Neuer Abgleich für ein Konto (Client mit Sitzung).
    pub fn new(
        client: Client,
        library: OfflineLibrary,
        outbox: Arc<dyn OfflineOutbox>,
        server_id: impl Into<String>,
        user_id: impl Into<String>,
    ) -> Self {
        Self {
            client,
            library,
            outbox,
            server_id: server_id.into(),
            user_id: user_id.into(),
        }
    }

    /// Anzahl ausstehender Offline-Meldungen.
    pub fn pending(&self) -> Result<usize> {
        Ok(self
            .outbox
            .pending(&self.server_id, &self.user_id, OUTBOX_KIND)?
            .len())
    }

    async fn server_last_played(&self, item_id: &str) -> Result<Value> {
        let query = vec![("userId".to_owned(), self.client.user_id()?)];
        Ok(self
            .client
            .get_json::<Value>(&format!("/UserItems/{item_id}/UserData"), &query)
            .await?)
    }

    /// Führt den Abgleich aus. Bei `Unauthorized` bricht er mit Fehler ab (nichts geht verloren).
    pub async fn sync(&self) -> Result<SyncReport> {
        let mut report = SyncReport::default();
        let pending = self
            .outbox
            .pending(&self.server_id, &self.user_id, OUTBOX_KIND)?;
        // Je Titel in Reihenfolge des ersten Auftretens; neueste Meldung gewinnt.
        let mut order: Vec<&str> = Vec::new();
        for e in &pending {
            if let Some(id) = item_of(e)
                && !order.contains(&id)
            {
                order.push(id);
            }
        }
        for item_id in order {
            let entries: Vec<&OutboxItem> = pending
                .iter()
                .filter(|e| item_of(e) == Some(item_id))
                .collect();
            let Some(latest) = entries.iter().max_by_key(|e| (stamp(e), e.id)).copied() else {
                continue;
            };
            let local_ms = stamp(latest);
            let server = match self.server_last_played(item_id).await {
                Ok(v) => Some(v),
                Err(OfflineError::Api(ApiError::NotFound)) => None,
                Err(OfflineError::Api(ApiError::Unauthorized)) => {
                    return Err(OfflineError::Api(ApiError::Unauthorized));
                }
                Err(e) => {
                    for entry in &entries {
                        self.outbox.mark_failed(entry.id, &e.to_string())?;
                    }
                    report.failed += 1;
                    // Vermutlich wieder offline: Rest bleibt für den nächsten Abgleich liegen.
                    break;
                }
            };
            let Some(server) = server else {
                self.complete_all(&entries)?;
                report.dropped += 1;
                continue;
            };
            let server_ms = server
                .get("LastPlayedDate")
                .and_then(Value::as_str)
                .and_then(millis_from_iso);
            if server_ms.is_none_or(|s| local_ms > s) {
                let body = json!({
                    "PlaybackPositionTicks": latest.payload.get("position_ticks").and_then(Value::as_i64).unwrap_or(0),
                    "Played": latest.payload.get("played").and_then(Value::as_bool).unwrap_or(false),
                    "LastPlayedDate": iso_from_millis(local_ms),
                });
                let query = vec![("userId".to_owned(), self.client.user_id()?)];
                let sent = self
                    .client
                    .call_json::<Value>(
                        Method::POST,
                        &format!("/UserItems/{item_id}/UserData"),
                        &query,
                        Some(&body),
                    )
                    .await;
                match sent {
                    Ok(_) => {
                        self.complete_all(&entries)?;
                        report.pushed += 1;
                    }
                    Err(ApiError::Unauthorized) => {
                        return Err(OfflineError::Api(ApiError::Unauthorized));
                    }
                    Err(e) => {
                        for entry in &entries {
                            self.outbox.mark_failed(entry.id, &e.to_string())?;
                        }
                        report.failed += 1;
                        break;
                    }
                }
            } else {
                // Server ist neuer oder gleich alt: lokalen Stand verwerfen und übernehmen.
                self.complete_all(&entries)?;
                self.adopt_server(item_id, &server, server_ms.unwrap_or(local_ms));
                report.kept_server += 1;
            }
            report.superseded += entries.len().saturating_sub(1);
        }
        report.auto_deleted = self.library.apply_auto_delete(self.outbox.as_ref())?.len();
        Ok(report)
    }

    fn complete_all(&self, entries: &[&OutboxItem]) -> Result<()> {
        for e in entries {
            self.outbox.complete(e.id)?;
        }
        Ok(())
    }

    /// Setzt den lokalen Datensatz auf den Server-Stand (sofern der Titel noch offline vorliegt).
    fn adopt_server(&self, item_id: &str, server: &Value, server_ms: i64) {
        let Ok(mut rec) = self.library.record(item_id) else {
            return;
        };
        rec.local = Some(LocalUserData {
            position_ticks: server
                .get("PlaybackPositionTicks")
                .and_then(Value::as_i64)
                .unwrap_or(0),
            played: server
                .get("Played")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            play_count: server.get("PlayCount").and_then(Value::as_i64).unwrap_or(0),
            last_played_ms: server_ms,
        });
        if let Err(e) = self.library.save(&rec) {
            tracing::warn!(error = %e, "Server-Stand nicht übernommen");
        }
    }
}
