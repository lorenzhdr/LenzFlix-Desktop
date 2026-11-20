//! W5-downloads: Hilfen für den Offline-Modus der App.
//!
//! * [`Gate`]: ein kleines TCP-Tor vor dem Mock. Mit `set_online(false)` bricht es alle Verbindungen ab und weist
//!   neue sofort zurück (die App sieht dann „keine Verbindung“, der Mock-Zustand bleibt erhalten); mit
//!   `set_online(true)` ist alles wieder erreichbar. So lässt sich „Netz weg, Netz wieder da“ prüfen, ohne den
//!   Port zu verlieren.
//! * [`serve_video_downloads`]: der Download-Endpunkt (`/Items/{id}/Download`) liefert statt der
//!   Mustererzeugung die Videodatei aus [`crate::media::set_video_file`] (mit `Range`), damit die App heruntergeladene
//!   Dateien wirklich abspielen kann.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::{AbortHandle, JoinHandle};

static REAL_VIDEO: AtomicBool = AtomicBool::new(false);

/// Schaltet um, ob Downloads die echte Testvideo-Datei liefern (Standard aus).
pub fn serve_video_downloads(on: bool) {
    REAL_VIDEO.store(on, Ordering::SeqCst);
}

/// Inhalt der Testvideo-Datei, wenn die Umschaltung an ist und eine Datei gesetzt wurde.
pub(crate) fn real_video() -> Option<Arc<Vec<u8>>> {
    if !REAL_VIDEO.load(Ordering::SeqCst) {
        return None;
    }
    let file = crate::media::video_file()?;
    std::fs::read(file).ok().map(Arc::new)
}

/// TCP-Tor vor dem Mock (siehe Moduldoku).
pub struct Gate {
    addr: SocketAddr,
    online: Arc<AtomicBool>,
    conns: Arc<Mutex<Vec<AbortHandle>>>,
    task: JoinHandle<()>,
}

impl Gate {
    /// Startet das Tor auf einem freien lokalen Port; es leitet zu `upstream` weiter.
    pub async fn start(upstream: SocketAddr) -> Gate {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("Port binden");
        let addr = listener.local_addr().expect("Adresse");
        let online = Arc::new(AtomicBool::new(true));
        let conns: Arc<Mutex<Vec<AbortHandle>>> = Arc::default();
        let (on, cs) = (online.clone(), conns.clone());
        let task = tokio::spawn(async move {
            loop {
                let Ok((inbound, _)) = listener.accept().await else {
                    return;
                };
                if !on.load(Ordering::SeqCst) {
                    drop(inbound);
                    continue;
                }
                let h = tokio::spawn(pipe(inbound, upstream));
                let mut list = cs.lock().unwrap_or_else(|e| e.into_inner());
                list.retain(|a| !a.is_finished());
                list.push(h.abort_handle());
            }
        });
        Gate {
            addr,
            online,
            conns,
            task,
        }
    }

    /// Adresse, unter der die App den Mock erreicht.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Netz an oder aus. Aus: bestehende Verbindungen enden sofort, neue werden verworfen.
    pub fn set_online(&self, on: bool) {
        self.online.store(on, Ordering::SeqCst);
        if !on {
            let mut list = self.conns.lock().unwrap_or_else(|e| e.into_inner());
            for h in list.drain(..) {
                h.abort();
            }
        }
    }
}

impl Drop for Gate {
    fn drop(&mut self) {
        self.set_online(false);
        self.task.abort();
    }
}

async fn pipe(mut inbound: TcpStream, upstream: SocketAddr) {
    let Ok(mut out) = TcpStream::connect(upstream).await else {
        return;
    };
    let _ = tokio::io::copy_bidirectional(&mut inbound, &mut out).await;
}
