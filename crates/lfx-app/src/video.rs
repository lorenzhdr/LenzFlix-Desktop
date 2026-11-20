//! Videoausgabe: ein Render-Thread verbindet mpv (D3D11-Render-API) mit dem Video-Layer der Shell.
//!
//! Nur Windows trägt die Ausgabe. Anderswo meldet [`VideoPipe::start`] einen Hinweistext; die App
//! zeigt dann „Wiedergabe auf macOS folgt“ (OpenGL-Pfad kommt später).

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64};

/// Zähler der Videoausgabe (für Diagnose und Tests: ohne gezeichnete Bilder ist die Ausgabe leer).
#[derive(Debug, Default)]
pub struct VideoStats {
    /// Gezeichnete Bilder seit dem Start.
    pub frames: AtomicU64,
    /// Breite der letzten Zielfläche in Pixeln.
    pub width: AtomicU32,
    /// Höhe der letzten Zielfläche in Pixeln.
    pub height: AtomicU32,
}

/// Gemeinsamer Zähler.
pub type SharedStats = Arc<VideoStats>;

#[cfg(windows)]
mod imp {
    use super::{SharedStats, VideoStats};
    use lfx_player::{D3d11Output, FrameReady, Player, RenderTarget, VideoOutput};
    use lfx_shell::VideoLayer;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Condvar, Mutex};
    use std::thread::JoinHandle;
    use std::time::Duration;

    #[derive(Default)]
    struct Signal {
        flag: Mutex<bool>,
        cv: Condvar,
    }

    impl Signal {
        fn notify(&self) {
            *self.flag.lock().unwrap_or_else(|e| e.into_inner()) = true;
            self.cv.notify_one();
        }

        fn wait(&self, timeout: Duration) {
            let mut g = self.flag.lock().unwrap_or_else(|e| e.into_inner());
            if !*g {
                g = self
                    .cv
                    .wait_timeout(g, timeout)
                    .unwrap_or_else(|e| e.into_inner())
                    .0;
            }
            *g = false;
        }
    }

    /// Render-Thread samt mpv-Render-Kontext. Beim Verwerfen wird der Thread beendet.
    pub struct VideoPipe {
        stop: Arc<AtomicBool>,
        signal: Arc<Signal>,
        thread: Option<JoinHandle<()>>,
        pub stats: SharedStats,
    }

    impl VideoPipe {
        /// Legt den Render-Kontext an (vor `Player::open`!) und startet den Thread.
        pub fn start(player: &Player, layer: VideoLayer) -> Result<Self, String> {
            let signal = Arc::new(Signal::default());
            let frame_ready: FrameReady = {
                let s = signal.clone();
                Arc::new(move || s.notify())
            };
            // SAFETY: das Gerät gehört dem Video-Layer (Multithread-Schutz an) und lebt, solange `layer` lebt;
            // der Layer wird in den Render-Thread verschoben und dort zuletzt freigegeben.
            let mut output =
                unsafe { D3d11Output::new(player, layer.d3d11_device_raw(), frame_ready) }
                    .map_err(|e| format!("Videoausgabe nicht startbar: {}", e.text))?;
            let stop = Arc::new(AtomicBool::new(false));
            let stats: SharedStats = Arc::new(VideoStats::default());
            let (t_stop, t_signal, t_stats) = (stop.clone(), signal.clone(), stats.clone());
            let thread = std::thread::Builder::new()
                .name("lfx-video".into())
                .spawn(move || {
                    while !t_stop.load(Ordering::Acquire) {
                        t_signal.wait(Duration::from_millis(40));
                        let new_frame = output.update();
                        let dirty = layer.take_dirty();
                        if !(new_frame || dirty) {
                            continue;
                        }
                        let Some(target) = layer.acquire_target() else {
                            continue;
                        };
                        let rendered = output
                            .render(RenderTarget::D3d11 {
                                texture: target.texture_raw,
                                width: target.width,
                                height: target.height,
                            })
                            .is_ok();
                        layer.present();
                        output.report_swap();
                        if rendered {
                            t_stats.frames.fetch_add(1, Ordering::Relaxed);
                            t_stats.width.store(target.width, Ordering::Relaxed);
                            t_stats.height.store(target.height, Ordering::Relaxed);
                        }
                    }
                    drop(output);
                    drop(layer);
                })
                .map_err(|e| format!("Render-Thread nicht startbar: {e}"))?;
            Ok(Self {
                stop,
                signal,
                thread: Some(thread),
                stats,
            })
        }
    }

    impl Drop for VideoPipe {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
            self.signal.notify();
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::SharedStats;
    use lfx_player::Player;
    use lfx_shell::VideoLayer;

    /// Auf dieser Plattform gibt es noch keine Videoausgabe.
    pub struct VideoPipe {
        pub stats: SharedStats,
    }

    impl VideoPipe {
        pub fn start(_player: &Player, _layer: VideoLayer) -> Result<Self, String> {
            Err("Wiedergabe auf macOS folgt".to_owned())
        }
    }
}

pub use imp::VideoPipe;
