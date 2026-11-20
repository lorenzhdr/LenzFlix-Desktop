//! Erreichbarkeit des aktiven Servers: `System/Ping`, danach eine kurze Token-Prüfung (`Users/Me`).
//! Bei Ausfall wartet die Schleife mit Exponential Backoff, bei Erfolg den normalen Abstand.

use crate::manager::{SessionManager, is_offline_error, lock};
use lfx_api::ApiError;
use std::sync::Weak;
use std::time::Duration;

impl SessionManager {
    /// Prüft den aktiven Server einmal. Setzt den Online-Zustand (Ereignis bei Änderung) und meldet ein
    /// abgelehntes Token als `AuthExpired`. Ohne aktive Sitzung ändert sich nichts. Gibt den Zustand zurück.
    pub async fn check_connectivity(&self) -> bool {
        let Some((session, probe)) = self.active_snapshot() else {
            return self.is_online();
        };
        let ping = probe
            .fetch_bytes(&format!("{}/System/Ping", probe.base_url()))
            .await;
        match ping {
            Err(err) if is_offline_error(&err) => {
                self.set_online(false);
                return false;
            }
            Ok(_) | Err(_) => {}
        }
        match probe.me().await {
            Err(ApiError::Unauthorized) => {
                // Nur verwerfen, wenn noch dasselbe Konto aktiv ist.
                let same = lock(&self.inner.active).as_ref().is_some_and(|a| {
                    a.session.server_id == session.server_id && a.session.user_id == session.user_id
                });
                if same {
                    self.expire_active();
                }
            }
            Err(err) if is_offline_error(&err) => {
                self.set_online(false);
                return false;
            }
            _ => {}
        }
        self.set_online(true);
        true
    }

    /// Startet die Prüfschleife im Hintergrund (braucht eine Tokio-Laufzeit). Ein laufender Wächter wird
    /// ersetzt. Die Schleife endet, wenn die Verwaltung verworfen wird.
    pub fn start_heartbeat(&self) {
        let weak = std::sync::Arc::downgrade(&self.inner);
        let task = tokio::spawn(heartbeat_loop(weak));
        if let Some(old) = lock(&self.inner.heartbeat).replace(task) {
            old.abort();
        }
    }

    /// Beendet die Prüfschleife.
    pub fn stop_heartbeat(&self) {
        if let Some(task) = lock(&self.inner.heartbeat).take() {
            task.abort();
        }
    }
}

async fn heartbeat_loop(weak: Weak<crate::manager::Inner>) {
    let mut backoff: Option<Duration> = None;
    let mut wait = Duration::ZERO;
    loop {
        tokio::time::sleep(wait).await;
        let Some(inner) = weak.upgrade() else {
            return;
        };
        let (interval, min, max) = (
            inner.options.heartbeat_interval,
            inner.options.backoff_min,
            inner.options.backoff_max,
        );
        let manager = SessionManager { inner };
        let online = manager.check_connectivity().await;
        drop(manager);
        wait = if online {
            backoff = None;
            interval
        } else {
            let next = backoff.map_or(min, |b| (b * 2).min(max));
            backoff = Some(next);
            next
        };
    }
}
