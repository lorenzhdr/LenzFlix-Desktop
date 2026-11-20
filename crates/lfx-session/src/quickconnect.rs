//! Quick-Connect-Anmeldung: Code anzeigen, auf Bestätigung warten, anmelden.

use crate::error::{Result, SessionError};
use crate::manager::SessionManager;
use crate::types::QuickConnectState;
use lfx_api::ApiError;
use tokio::sync::mpsc;
use tokio::task::AbortHandle;

/// Laufende Quick-Connect-Anmeldung. Beim Verwerfen wird sie abgebrochen.
pub struct QuickConnectHandle {
    states: mpsc::Receiver<QuickConnectState>,
    abort: AbortHandle,
}

impl QuickConnectHandle {
    /// Nächster Zustand; `None`, wenn die Anmeldung beendet ist (nach `Authorized` oder `Failed`).
    pub async fn next(&mut self) -> Option<QuickConnectState> {
        self.states.recv().await
    }

    /// Bricht die Anmeldung ab.
    pub fn cancel(&self) {
        self.abort.abort();
    }
}

impl Drop for QuickConnectHandle {
    fn drop(&mut self) {
        self.abort.abort();
    }
}

impl std::fmt::Debug for QuickConnectHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("QuickConnectHandle")
    }
}

/// Zu viele aufeinanderfolgende Netzwerkfehler beim Abfragen beenden die Anmeldung.
const MAX_POLL_ERRORS: u32 = 5;

impl SessionManager {
    /// Startet Quick Connect für einen Server (braucht eine Tokio-Laufzeit). Der Handle liefert nacheinander
    /// `Code(..)`, `Waiting`, dann `Authorized` (Sitzung ist aktiv) oder `Failed(..)`.
    pub fn login_quick_connect(&self, server_id: &str) -> Result<QuickConnectHandle> {
        let server = self.server_entry(server_id)?;
        let client = self.new_client(&server.url, false)?;
        let (tx, rx) = mpsc::channel(8);
        let manager = self.clone();
        let server_id = server_id.to_owned();
        let task = tokio::spawn(async move {
            if let Err(message) = run(&manager, client, &server_id, &tx).await {
                let _ = tx.send(QuickConnectState::Failed(message)).await;
            }
        });
        Ok(QuickConnectHandle {
            states: rx,
            abort: task.abort_handle(),
        })
    }
}

/// Ablauf; der Fehlertext ist für die Anzeige gedacht (Deutsch, ohne Geheimnisse).
async fn run(
    manager: &SessionManager,
    client: lfx_api::Client,
    server_id: &str,
    tx: &mpsc::Sender<QuickConnectState>,
) -> std::result::Result<(), String> {
    let options = &manager.inner.options;
    let describe = |e: ApiError| match e {
        ApiError::QuickConnectDisabled => {
            "Quick Connect ist auf dem Server nicht aktiviert".to_owned()
        }
        ApiError::Network(_) | ApiError::Timeout => "Server nicht erreichbar".to_owned(),
        other => other.to_string(),
    };
    if !client.quick_connect_enabled().await.map_err(describe)? {
        return Err(describe(ApiError::QuickConnectDisabled));
    }
    let init = client.quick_connect_initiate().await.map_err(describe)?;
    if tx
        .send(QuickConnectState::Code(init.code.clone()))
        .await
        .is_err()
    {
        return Ok(()); // Empfänger weg
    }
    let deadline = tokio::time::Instant::now() + options.quick_connect_timeout;
    let mut announced_waiting = false;
    let mut errors = 0u32;
    loop {
        if tokio::time::Instant::now() >= deadline {
            return Err("Zeit abgelaufen, bitte erneut versuchen".into());
        }
        tokio::time::sleep(options.quick_connect_poll).await;
        match client.quick_connect_poll(&init.secret).await {
            Ok(state) if state.authenticated => break,
            Ok(_) => {
                errors = 0;
                if !announced_waiting {
                    announced_waiting = true;
                    if tx.send(QuickConnectState::Waiting).await.is_err() {
                        return Ok(());
                    }
                }
            }
            Err(ApiError::NotFound) => return Err("Der Code ist abgelaufen".into()),
            Err(err) if err.is_retryable() || matches!(err, ApiError::Network(_)) => {
                errors += 1;
                if errors >= MAX_POLL_ERRORS {
                    return Err(describe(err));
                }
            }
            Err(err) => return Err(describe(err)),
        }
    }
    let auth = client
        .quick_connect_login(&init.secret)
        .await
        .map_err(describe)?;
    manager
        .finish_login(client, server_id, &auth)
        .map_err(|e: SessionError| e.to_string())?;
    let _ = tx.send(QuickConnectState::Authorized).await;
    Ok(())
}
