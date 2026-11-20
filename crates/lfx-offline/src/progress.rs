//! Fortschritt offline: Meldungen der lokalen Wiedergabe landen im Datensatz (Anzeige) und in der Outbox
//! (Abgleich). Je Titel bleibt nur die neueste Meldung stehen (der Player meldet alle paar Sekunden).

use crate::error::Result;
use crate::library::{OUTBOX_KIND, OfflineLibrary};
use crate::store::OfflineOutbox;
use crate::time::{Clock, iso_from_millis};
use crate::types::LocalUserData;
use lfx_player::{Report, ReportError, ReportSink};
use serde_json::{Value, json};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Ab diesem Anteil der Laufzeit gilt ein Titel als gesehen (wie die Standardgrenze des Servers).
pub const PLAYED_FRACTION: f64 = 0.9;

/// Zeichnet Fortschritt offline auf.
#[derive(Clone)]
pub struct OfflineProgress {
    library: OfflineLibrary,
    outbox: Arc<dyn OfflineOutbox>,
    clock: Arc<dyn Clock>,
    server_id: String,
    user_id: String,
}

impl OfflineProgress {
    /// Neuer Rekorder für ein Konto.
    pub fn new(
        library: OfflineLibrary,
        outbox: Arc<dyn OfflineOutbox>,
        clock: Arc<dyn Clock>,
        server_id: impl Into<String>,
        user_id: impl Into<String>,
    ) -> Self {
        Self {
            library,
            outbox,
            clock,
            server_id: server_id.into(),
            user_id: user_id.into(),
        }
    }

    /// Hält Position und Gesehen-Stand eines Offline-Titels fest. `finished`: Wiedergabe ist bis zum Ende gelaufen.
    pub fn record(
        &self,
        item_id: &str,
        position_ticks: i64,
        finished: bool,
    ) -> Result<LocalUserData> {
        let mut rec = self.library.record(item_id)?;
        let now = self.clock.now_millis();
        let runtime = rec.runtime_ticks.unwrap_or(0);
        let reached = runtime > 0 && position_ticks as f64 >= runtime as f64 * PLAYED_FRACTION;
        let newly_played = finished || reached;
        let prior = rec.local.clone();
        let was_played = prior.as_ref().is_some_and(|l| l.played);
        let local = LocalUserData {
            position_ticks: if newly_played {
                0
            } else {
                position_ticks.max(0)
            },
            played: newly_played || was_played,
            play_count: prior.as_ref().map_or(0, |l| l.play_count)
                + i64::from(newly_played && !was_played),
            last_played_ms: now,
        };
        rec.local = Some(local.clone());
        self.library.save(&rec)?;

        // Ältere Meldungen zum selben Titel sind überholt.
        for old in self
            .outbox
            .pending(&self.server_id, &self.user_id, OUTBOX_KIND)?
        {
            if old.payload.get("item_id").and_then(Value::as_str) == Some(item_id) {
                self.outbox.complete(old.id)?;
            }
        }
        self.outbox.push(
            &self.server_id,
            &self.user_id,
            OUTBOX_KIND,
            &json!({
                "item_id": item_id,
                "media_source_id": rec.media_source_id,
                "position_ticks": local.position_ticks,
                "played": local.played,
                "play_count": local.play_count,
                "last_played_ms": local.last_played_ms,
                "last_played": iso_from_millis(local.last_played_ms),
            }),
        )?;
        Ok(local)
    }
}

/// [`ReportSink`] für den Player: Meldungen gehen in die Outbox statt zum Server.
/// Für Titel ohne Offline-Datensatz werden sie still verworfen.
pub struct OutboxSink {
    progress: OfflineProgress,
}

impl OutboxSink {
    /// Sink über einen Rekorder.
    pub fn new(progress: OfflineProgress) -> Self {
        Self { progress }
    }
}

impl ReportSink for OutboxSink {
    fn send(
        &self,
        report: Report,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<(), ReportError>> + Send + '_>> {
        Box::pin(async move {
            let result = match report {
                Report::Playing(_) => return Ok(()),
                Report::Progress(r) => self.progress.record(&r.item_id, r.position_ticks, false),
                Report::Stopped(s) => {
                    if s.failed {
                        return Ok(());
                    }
                    self.progress.record(&s.item_id, s.position_ticks, false)
                }
            };
            match result {
                Ok(_) | Err(crate::OfflineError::UnknownDownload) => Ok(()),
                Err(e) => Err(ReportError(e.to_string())),
            }
        })
    }
}
