//! Zusatzdienste des Players: Medientasten („Wird wiedergegeben“), Bildschirmschoner-Sperre, Vollbild, Trickplay-Kacheln
//! und das Vorschaubild für die Medientasten.

use std::rc::Rc;
use std::time::{Duration, Instant};

use lfx_platform::{KeepAwake, MediaCommand, NowPlaying, PlaybackStatus};
use lfx_player::PlayerState;
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

use crate::app::{App, post};

/// Gespeicherte Fensterlage vor dem Vollbild (nur Windows).
#[cfg(windows)]
#[derive(Debug, Clone, Copy)]
pub struct SavedWindow {
    pub left: i32,
    pub top: i32,
    pub width: i32,
    pub height: i32,
    pub maximized: bool,
}

impl App {
    // ---------------------------------------------------------------- Medientasten

    /// Legt die Anbindung an die Medientasten des Systems einmal an (nur Windows; macOS folgt).
    #[cfg(not(windows))]
    pub(crate) fn ensure_media_controls(self: &Rc<Self>) {}

    #[cfg(windows)]
    pub(crate) fn ensure_media_controls(self: &Rc<Self>) {
        use lfx_platform::MediaControls;
        use std::sync::Arc;
        {
            {
                let mut st = self.st.borrow_mut();
                if st.play.media.is_some() || st.play.media_tried {
                    return;
                }
                st.play.media_tried = true;
            }
            let handler: lfx_platform::CommandHandler =
                Arc::new(|cmd| post(move |app| app.media_command(cmd)));
            match MediaControls::new(self.shell.hwnd_raw(), handler) {
                Ok(m) => {
                    self.st.borrow_mut().play.media = Some(m);
                    tracing::info!("Medientasten verbunden");
                }
                Err(e) => tracing::info!("{e}"),
            }
        }
    }

    pub(crate) fn clear_media_controls(&self) {
        let mut st = self.st.borrow_mut();
        if let Some(m) = &st.play.media {
            m.clear();
        }
        st.play.media_status = None;
        st.play.media_meta_sent = false;
        st.play.cover_file_written = false;
    }

    /// Befehl des Systems (Medientaste, Sperrbildschirm, Lautstärke-Flyout).
    pub fn media_command(self: &Rc<Self>, cmd: MediaCommand) {
        if self.audio_media_command(cmd) {
            return; // W4-media: Musik läuft
        }
        if !matches!(
            self.st.borrow().router.current(),
            crate::route::Route::Player { .. }
        ) {
            return;
        }
        let Some(p) = self.st.borrow().play.player.clone() else {
            return;
        };
        // W5-remote: in einer SyncPlay-Gruppe geht die Bedienung an den Server
        let handled = match cmd {
            MediaCommand::Play => self.remote_intent_pause(Some(false)),
            MediaCommand::Pause => self.remote_intent_pause(Some(true)),
            MediaCommand::Toggle => self.remote_intent_pause(None),
            MediaCommand::FastForward => self.remote_intent_seek_by(30.0),
            MediaCommand::Rewind => self.remote_intent_seek_by(-10.0),
            MediaCommand::SeekTo(t) => self.remote_intent_seek(t),
            _ => false,
        };
        if handled {
            self.player_activity();
            return;
        }
        match cmd {
            MediaCommand::Play => p.play(),
            MediaCommand::Pause => p.pause(),
            MediaCommand::Toggle => p.toggle_pause(),
            MediaCommand::Stop => self.leave_player(),
            MediaCommand::Next => self.play_next(),
            MediaCommand::Previous => p.seek_to(0.0),
            MediaCommand::FastForward => p.seek_by(30.0),
            MediaCommand::Rewind => p.seek_by(-10.0),
            MediaCommand::SeekTo(t) => p.seek_to(t),
        }
        self.player_activity();
    }

    /// Wachhalten und Systemanzeige je Tick.
    pub(crate) fn tick_media(self: &Rc<Self>, s: &PlayerState) {
        let playing = s.loaded && !s.paused;
        {
            let mut st = self.st.borrow_mut();
            if playing && st.play.awake.is_none() {
                st.play.awake = Some(KeepAwake::acquire());
            } else if !playing {
                st.play.awake = None;
            }
        }
        let (title, subtitle) = self.now_playing_title();
        let mut guard = self.st.borrow_mut();
        let play = &mut guard.play;
        let Some(m) = &play.media else { return };
        if !play.media_meta_sent && s.loaded {
            let cover_file = play
                .cover_file_written
                .then(|| self.dirs.cache.join("nowplaying.png"));
            m.set_now_playing(&NowPlaying {
                title,
                subtitle,
                cover_file,
            });
            play.media_meta_sent = true;
        }
        let status = if !s.loaded || s.buffering {
            PlaybackStatus::Changing
        } else if s.paused {
            PlaybackStatus::Paused
        } else {
            PlaybackStatus::Playing
        };
        if play.media_status != Some(status) {
            m.set_status(status);
            m.set_buttons(play.next_item.is_some(), true, true);
            play.media_status = Some(status);
        }
        let due = play
            .media_timeline
            .is_none_or(|t| t.elapsed() >= Duration::from_secs(1));
        if due && s.loaded {
            if let Some(d) = s.duration {
                m.set_timeline(s.position, d);
            }
            play.media_timeline = Some(Instant::now());
        }
    }

    /// Schreibt das Startbild als Vorschau für die Medientasten (und meldet Titel neu).
    pub(crate) fn write_cover_file(&self) {
        let Some(img) = self.st.borrow().play.cover.clone() else {
            return;
        };
        let Some(buf) = img.to_rgba8() else { return };
        let path = self.dirs.cache.join("nowplaying.png");
        if crate::capture::write_png(&path, buf.width(), buf.height(), buf.as_bytes()).is_ok() {
            let mut st = self.st.borrow_mut();
            st.play.cover_file_written = true;
            // Beim nächsten Tick mit Bild erneut melden.
            st.play.media_meta_sent = false;
        }
    }

    // ---------------------------------------------------------------- Vollbild

    pub fn toggle_fullscreen(self: &Rc<Self>) {
        if self.st.borrow().play.fullscreen {
            self.exit_fullscreen();
        } else {
            self.enter_fullscreen();
        }
    }

    #[cfg(windows)]
    fn enter_fullscreen(self: &Rc<Self>) {
        use windows::Win32::Foundation::{HWND, RECT};
        use windows::Win32::Graphics::Gdi::{
            GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
        };
        use windows::Win32::UI::WindowsAndMessaging::{
            GetWindowRect, HWND_TOP, IsZoomed, SW_RESTORE, SWP_FRAMECHANGED, SWP_NOOWNERZORDER,
            SetWindowPos, ShowWindow,
        };
        let hwnd = HWND(self.shell.hwnd_raw() as *mut _);
        // SAFETY: Fenster des eigenen Prozesses; reine Fensterabfragen und -verschiebung im UI-Thread.
        unsafe {
            let mut rect = RECT::default();
            if GetWindowRect(hwnd, &mut rect).is_err() {
                return;
            }
            let maximized = IsZoomed(hwnd).as_bool();
            let mon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
            let mut mi = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..MONITORINFO::default()
            };
            if !GetMonitorInfoW(mon, &mut mi).as_bool() {
                return;
            }
            if maximized {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
            let r = mi.rcMonitor;
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_TOP),
                r.left,
                r.top,
                r.right - r.left,
                r.bottom - r.top,
                SWP_NOOWNERZORDER | SWP_FRAMECHANGED,
            );
            let mut st = self.st.borrow_mut();
            st.play.saved_window = Some(SavedWindow {
                left: rect.left,
                top: rect.top,
                width: rect.right - rect.left,
                height: rect.bottom - rect.top,
                maximized,
            });
            st.play.fullscreen = true;
        }
    }

    #[cfg(not(windows))]
    fn enter_fullscreen(self: &Rc<Self>) {
        self.toast("Vollbild folgt auf diesem System", lfx_ui::ToastKind::Info);
    }

    #[cfg(windows)]
    pub(crate) fn exit_fullscreen(&self) {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::{
            HWND_TOP, SW_MAXIMIZE, SWP_FRAMECHANGED, SWP_NOOWNERZORDER, SetWindowPos, ShowWindow,
        };
        let saved = {
            let mut st = self.st.borrow_mut();
            if !st.play.fullscreen {
                return;
            }
            st.play.fullscreen = false;
            st.play.saved_window.take()
        };
        let Some(w) = saved else { return };
        let hwnd = HWND(self.shell.hwnd_raw() as *mut _);
        // SAFETY: siehe `enter_fullscreen`.
        unsafe {
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_TOP),
                w.left,
                w.top,
                w.width,
                w.height,
                SWP_NOOWNERZORDER | SWP_FRAMECHANGED,
            );
            if w.maximized {
                let _ = ShowWindow(hwnd, SW_MAXIMIZE);
            }
        }
    }

    #[cfg(not(windows))]
    pub(crate) fn exit_fullscreen(&self) {
        self.st.borrow_mut().play.fullscreen = false;
    }

    // ---------------------------------------------------------------- Trickplay

    /// Lädt ein Trickplay-Kachelbild (einmal) und zeigt es danach in der Vorschau.
    pub(crate) fn request_trick_tile(self: &Rc<Self>, tile: u32) {
        let Some(active) = self.session.active() else {
            return;
        };
        let (epoch, item_id, width, source) = {
            let mut st = self.st.borrow_mut();
            if st.play.trick.tiles.contains_key(&tile) || !st.play.trick.requested.insert(tile) {
                return;
            }
            let Some(item) = st.play.item.as_ref() else {
                return;
            };
            (
                st.play.generation,
                item.id.clone(),
                st.play.trick.width,
                st.play.trick.source_id.clone(),
            )
        };
        let client = active.client.clone();
        self.rt.spawn(async move {
            let Ok(url) = client.trickplay_tile_url(&item_id, width, tile, Some(&source)) else {
                return;
            };
            let Ok(bytes) = client.fetch_bytes(&url).await else {
                tracing::debug!("Trickplay-Kachel nicht geladen");
                return;
            };
            let decoded = tokio::task::spawn_blocking(move || {
                lfx_images::decode_image(&bytes, None, None, false)
            })
            .await;
            let Ok(Ok(img)) = decoded else { return };
            let mut buf = SharedPixelBuffer::<Rgba8Pixel>::new(img.width, img.height);
            let dst = buf.make_mut_bytes();
            let n = dst.len().min(img.pixels.len());
            dst[..n].copy_from_slice(&img.pixels[..n]);
            post(move |app| {
                {
                    let mut st = app.st.borrow_mut();
                    if st.play.generation != epoch {
                        return;
                    }
                    if st.play.trick.tiles.len() >= 6 {
                        st.play.trick.tiles.clear();
                    }
                    st.play.trick.tiles.insert(tile, Image::from_rgba8(buf));
                }
                app.refresh_view();
            });
        });
    }
}
