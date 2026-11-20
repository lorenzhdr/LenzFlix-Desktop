//! W5-tray: Einzelinstanz. Ein zweiter Start meldet sich bei der laufenden App (die zeigt ihr Fenster) und endet.
//!
//! Der Name hängt vom Datenordner ab: Läufe mit eigenem `--home` (Tests, Entwicklung) stören sich nicht, nur
//! Starts mit demselben Datenordner zählen als dieselbe App.
//!
//! - Windows: benannte Pipe. `first_pipe_instance(true)` ist Sperre und Kanal zugleich; der zweite Start öffnet die
//!   Pipe und schreibt `show`.
//! - Sonst (macOS): Sperrdatei (`File::try_lock`) plus „Weckdatei“, deren Änderungszeit die laufende App abfragt.
//!   Die Dateivariante wird auf allen Systemen mitgebaut und getestet, genutzt wird sie außerhalb von Windows.

use std::fs::File;
use std::path::{Path, PathBuf};

/// Kennung aus dem Datenordner (FNV-1a, 64 Bit, hex).
pub fn instance_key(data_dir: &Path) -> String {
    let text = data_dir.to_string_lossy().to_lowercase().replace('\\', "/");
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.trim_end_matches('/').bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// Ausgang der Anmeldung als Einzelinstanz.
pub enum Role {
    /// Diese App ist die erste; der Wächter muss bis zum Ende leben.
    Primary(Guard),
    /// Es läuft schon eine App und wurde benachrichtigt: sofort beenden.
    Secondary,
}

/// Hält die Sperre (Pipe bzw. Sperrdatei) bis zum Beenden.
pub struct Guard {
    _file: Option<FileLock>,
    wake_file: Option<PathBuf>,
}

impl Guard {
    /// Weckdatei zum Abfragen (nur Dateivariante).
    pub fn wake_file(&self) -> Option<&Path> {
        self.wake_file.as_deref()
    }
}

/// Dateisperre.
pub struct FileLock {
    _f: File,
}

/// Dateivariante: Sperre nehmen oder die laufende App wecken.
pub fn acquire_file(dir: &Path, key: &str) -> std::io::Result<Role> {
    std::fs::create_dir_all(dir)?;
    let lock_path = dir.join(format!("lenzflix-{key}.lock"));
    let wake_path = dir.join(format!("lenzflix-{key}.wake"));
    let f = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)?;
    match f.try_lock() {
        Ok(()) => Ok(Role::Primary(Guard {
            _file: Some(FileLock { _f: f }),
            wake_file: Some(wake_path),
        })),
        Err(std::fs::TryLockError::WouldBlock) => {
            // Zeitstempel schreiben: ändert die Änderungszeit, auch bei gleichem Inhalt.
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos());
            std::fs::write(&wake_path, now.to_string())?;
            Ok(Role::Secondary)
        }
        Err(std::fs::TryLockError::Error(e)) => Err(e),
    }
}

/// Stand der Weckdatei: `None` = noch nie abgefragt, `Some(None)` = Datei fehlte.
pub type WakeStamp = Option<Option<std::time::SystemTime>>;

/// Hat jemand die Weckdatei seit dem letzten Abruf berührt? Der erste Abruf legt nur den Ausgangsstand fest.
pub fn wake_requested(path: &Path, last: &mut WakeStamp) -> bool {
    let m = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    let first = last.is_none();
    let changed = m.is_some() && Some(m) != *last;
    *last = Some(m);
    changed && !first
}

#[cfg(windows)]
pub use win::acquire_pipe;

#[cfg(windows)]
mod win {
    use std::sync::Arc;
    use std::time::Duration;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::windows::named_pipe::{ClientOptions, ServerOptions};

    use super::{Guard, Role};

    fn pipe_name(key: &str) -> String {
        format!(r"\\.\pipe\lenzflix-{key}")
    }

    /// Pipe als Sperre: Erster Start wird `Primary` (der Server läuft auf `rt`, jeder `show` ruft `wake`),
    /// jeder weitere schreibt `show` und ist `Secondary`.
    pub fn acquire_pipe(
        rt: &tokio::runtime::Handle,
        key: &str,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Role, String> {
        let name = pipe_name(key);
        let _enter = rt.enter();
        match ServerOptions::new()
            .first_pipe_instance(true)
            .reject_remote_clients(true)
            .create(&name)
        {
            Ok(server) => {
                rt.spawn(serve(name, server, wake));
                Ok(Role::Primary(Guard {
                    _file: None,
                    wake_file: None,
                }))
            }
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                rt.block_on(notify(&name)).map_err(|e| e.to_string())?;
                Ok(Role::Secondary)
            }
            Err(e) => Err(e.to_string()),
        }
    }

    async fn notify(name: &str) -> std::io::Result<()> {
        let mut last = None;
        for _ in 0..20 {
            match ClientOptions::new().open(name) {
                Ok(mut c) => {
                    c.write_all(b"show\n").await?;
                    c.flush().await?;
                    return Ok(());
                }
                // Pipe gerade ausgelastet (ERROR_PIPE_BUSY = 231): kurz warten.
                Err(e) if e.raw_os_error() == Some(231) => {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    last = Some(e);
                }
                Err(e) => return Err(e),
            }
        }
        Err(last.unwrap_or_else(|| std::io::Error::other("Pipe nicht erreichbar")))
    }

    async fn serve(
        name: String,
        mut server: tokio::net::windows::named_pipe::NamedPipeServer,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) {
        loop {
            if server.connect().await.is_err() {
                return;
            }
            let Ok(next) = ServerOptions::new()
                .reject_remote_clients(true)
                .create(&name)
            else {
                return;
            };
            let mut conn = std::mem::replace(&mut server, next);
            let mut buf = [0u8; 16];
            let read = tokio::time::timeout(Duration::from_secs(1), conn.read(&mut buf)).await;
            if let Ok(Ok(n)) = read
                && buf[..n].starts_with(b"show")
            {
                wake();
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::sync::atomic::{AtomicUsize, Ordering};

        #[test]
        fn zweiter_start_weckt_den_ersten() {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let key = format!("test-{}", std::process::id());
            let hits = Arc::new(AtomicUsize::new(0));
            let h = hits.clone();
            let first = acquire_pipe(
                rt.handle(),
                &key,
                Arc::new(move || {
                    h.fetch_add(1, Ordering::SeqCst);
                }),
            )
            .unwrap();
            assert!(matches!(first, Role::Primary(_)));
            let second = acquire_pipe(rt.handle(), &key, Arc::new(|| {})).unwrap();
            assert!(matches!(second, Role::Secondary));
            for _ in 0..50 {
                if hits.load(Ordering::SeqCst) == 1 {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            assert_eq!(hits.load(Ordering::SeqCst), 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schluessel_haengt_vom_ordner_ab() {
        let a = instance_key(Path::new("C:/A/data"));
        assert_eq!(a, instance_key(Path::new("c:\\a\\data\\")));
        assert_ne!(a, instance_key(Path::new("C:/B/data")));
        assert_eq!(a.len(), 16);
    }

    #[test]
    fn dateivariante_sperre_und_wecken() {
        let dir = tempfile::tempdir().unwrap();
        let first = acquire_file(dir.path(), "k").unwrap();
        let Role::Primary(guard) = first else {
            panic!("erste Instanz")
        };
        let wake = guard.wake_file().unwrap().to_path_buf();
        let mut last = None;
        assert!(!wake_requested(&wake, &mut last)); // Ausgangsstand: keine Datei
        assert!(matches!(
            acquire_file(dir.path(), "k").unwrap(),
            Role::Secondary
        ));
        assert!(wake_requested(&wake, &mut last));
        assert!(!wake_requested(&wake, &mut last));
        std::thread::sleep(std::time::Duration::from_millis(30));
        assert!(matches!(
            acquire_file(dir.path(), "k").unwrap(),
            Role::Secondary
        ));
        assert!(wake_requested(&wake, &mut last));
        assert!(!wake_requested(&wake, &mut last));
        drop(guard);
        assert!(matches!(
            acquire_file(dir.path(), "k").unwrap(),
            Role::Primary(_)
        ));
    }
}
