//! Bildschirmschoner und Energiesparen während der Wiedergabe verhindern.
//!
//! [`KeepAwake`] hält das System (und die Anzeige) wach, solange der Wert lebt. Windows nutzt
//! `SetThreadExecutionState` (gilt je Thread, deshalb nicht `Send`: im selben Thread anlegen und
//! freigeben). macOS startet `caffeinate` an die eigene Prozess-Id gebunden (kein Fremdcode, keine
//! Systembibliothek nötig). Andere Systeme: kein Effekt.

use std::marker::PhantomData;

/// Hält das System wach, bis der Wert verworfen wird.
#[derive(Debug)]
pub struct KeepAwake {
    #[cfg(target_os = "macos")]
    child: Option<std::process::Child>,
    // Thread-gebunden (siehe Modulbeschreibung).
    _not_send: PhantomData<*const ()>,
}

impl KeepAwake {
    /// Fordert „System und Anzeige wach halten“ an.
    pub fn acquire() -> Self {
        #[cfg(windows)]
        {
            use windows::Win32::System::Power::{
                ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED, SetThreadExecutionState,
            };
            // SAFETY: reiner Zustandsaufruf ohne Zeiger.
            unsafe {
                SetThreadExecutionState(ES_CONTINUOUS | ES_DISPLAY_REQUIRED | ES_SYSTEM_REQUIRED);
            }
            Self {
                _not_send: PhantomData,
            }
        }
        #[cfg(target_os = "macos")]
        {
            let child = std::process::Command::new("/usr/bin/caffeinate")
                .args(["-d", "-i", "-w", &std::process::id().to_string()])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .ok();
            Self {
                child,
                _not_send: PhantomData,
            }
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            Self {
                _not_send: PhantomData,
            }
        }
    }

    /// Ist die Anforderung auf diesem System wirksam (Windows immer, macOS wenn `caffeinate` lief)?
    pub fn is_effective(&self) -> bool {
        #[cfg(windows)]
        {
            true
        }
        #[cfg(target_os = "macos")]
        {
            self.child.is_some()
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            false
        }
    }
}

impl Drop for KeepAwake {
    fn drop(&mut self) {
        #[cfg(windows)]
        {
            use windows::Win32::System::Power::{ES_CONTINUOUS, SetThreadExecutionState};
            // SAFETY: setzt nur den Dauerzustand zurück.
            unsafe {
                SetThreadExecutionState(ES_CONTINUOUS);
            }
        }
        #[cfg(target_os = "macos")]
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acquire_and_release_do_not_panic() {
        let k = KeepAwake::acquire();
        let _ = k.is_effective();
        drop(k);
    }
}
