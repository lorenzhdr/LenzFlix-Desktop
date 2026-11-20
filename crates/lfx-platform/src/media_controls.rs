//! Medientasten und „Wird wiedergegeben“ des Betriebssystems.
//!
//! * Windows: SystemMediaTransportControls (Lautstärke-Flyout, Sperrbildschirm, Medientasten der Tastatur
//!   und von Kopfhörern) über die `windows`-Crate; Titel, Bild, Spulleiste, Play/Pause/Weiter/Zurück/Springen.
//! * macOS: **noch nicht umgesetzt** (MPNowPlayingInfoCenter/MPRemoteCommandCenter brauchen objc2-Bindungen und
//!   lassen sich ohne Mac nicht prüfen). [`MediaControls::new`] meldet dort einen Fehler, die App läuft ohne.
//!
//! Befehle des Systems kommen auf einem fremden Thread an und gehen an den `handler`; die App reicht sie in den
//! UI-Thread weiter.

use std::path::PathBuf;
use std::sync::Arc;

/// Befehl vom Betriebssystem.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MediaCommand {
    Play,
    Pause,
    Toggle,
    Stop,
    Next,
    Previous,
    /// Vorspulen (Taste), Schrittweite legt die App fest.
    FastForward,
    Rewind,
    /// Auf diese Position (Sekunden) springen.
    SeekTo(f64),
}

/// Zustand für die Anzeige des Systems.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackStatus {
    Stopped,
    Playing,
    Paused,
    /// Lädt/puffert.
    Changing,
}

/// Was gerade läuft.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NowPlaying {
    pub title: String,
    /// Zweite Zeile (z. B. Serienname).
    pub subtitle: String,
    /// Bilddatei (PNG/JPEG) für die Vorschau, falls vorhanden.
    pub cover_file: Option<PathBuf>,
}

/// Handler für Systembefehle (läuft auf einem fremden Thread).
pub type CommandHandler = Arc<dyn Fn(MediaCommand) + Send + Sync + 'static>;

/// 100-ns-Einheiten (Windows-`TimeSpan`) aus Sekunden; negative und ungültige Werte werden 0.
pub fn timespan_ticks(secs: f64) -> i64 {
    if secs.is_finite() && secs > 0.0 {
        (secs * 10_000_000.0).round() as i64
    } else {
        0
    }
}

/// Sekunden aus 100-ns-Einheiten.
pub fn ticks_to_secs(ticks: i64) -> f64 {
    ticks.max(0) as f64 / 10_000_000.0
}

#[cfg(windows)]
mod imp {
    use super::{
        CommandHandler, MediaCommand, NowPlaying, PlaybackStatus, ticks_to_secs, timespan_ticks,
    };
    use windows::Foundation::{TimeSpan, TypedEventHandler, Uri};
    use windows::Media::{
        MediaPlaybackStatus, MediaPlaybackType, PlaybackPositionChangeRequestedEventArgs,
        SystemMediaTransportControls, SystemMediaTransportControlsButton,
        SystemMediaTransportControlsButtonPressedEventArgs,
        SystemMediaTransportControlsTimelineProperties,
    };
    use windows::Storage::Streams::RandomAccessStreamReference;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::WinRT::ISystemMediaTransportControlsInterop;
    use windows::core::{HSTRING, factory};

    /// SMTC-Anbindung an ein Fenster.
    pub struct MediaControls {
        smtc: SystemMediaTransportControls,
        button_token: i64,
        seek_token: i64,
    }

    fn err(e: windows::core::Error) -> String {
        format!("Medientasten nicht verfügbar: {e}")
    }

    fn command_of(b: SystemMediaTransportControlsButton) -> Option<MediaCommand> {
        Some(match b {
            SystemMediaTransportControlsButton::Play => MediaCommand::Play,
            SystemMediaTransportControlsButton::Pause => MediaCommand::Pause,
            SystemMediaTransportControlsButton::Stop => MediaCommand::Stop,
            SystemMediaTransportControlsButton::Next => MediaCommand::Next,
            SystemMediaTransportControlsButton::Previous => MediaCommand::Previous,
            SystemMediaTransportControlsButton::FastForward => MediaCommand::FastForward,
            SystemMediaTransportControlsButton::Rewind => MediaCommand::Rewind,
            _ => return None,
        })
    }

    impl MediaControls {
        pub fn new(hwnd: isize, handler: CommandHandler) -> Result<Self, String> {
            let interop =
                factory::<SystemMediaTransportControls, ISystemMediaTransportControlsInterop>()
                    .map_err(err)?;
            // SAFETY: `hwnd` ist ein gültiges Fenster des eigenen Prozesses (vom Aufrufer geliefert).
            let smtc: SystemMediaTransportControls =
                unsafe { interop.GetForWindow(HWND(hwnd as *mut _)) }.map_err(err)?;
            smtc.SetIsEnabled(true).map_err(err)?;
            smtc.SetIsPlayEnabled(true).map_err(err)?;
            smtc.SetIsPauseEnabled(true).map_err(err)?;
            smtc.SetIsStopEnabled(true).map_err(err)?;
            smtc.SetIsNextEnabled(false).map_err(err)?;
            smtc.SetIsPreviousEnabled(false).map_err(err)?;
            let h = handler.clone();
            let button_token = smtc
                .ButtonPressed(&TypedEventHandler::new(
                    move |_,
                          args: windows::core::Ref<
                        SystemMediaTransportControlsButtonPressedEventArgs,
                    >| {
                        if let Some(a) = args.as_ref()
                            && let Ok(b) = a.Button()
                            && let Some(c) = command_of(b)
                        {
                            h(c);
                        }
                        Ok(())
                    },
                ))
                .map_err(err)?;
            let seek_token = smtc
                .PlaybackPositionChangeRequested(&TypedEventHandler::new(
                    move |_, args: windows::core::Ref<PlaybackPositionChangeRequestedEventArgs>| {
                        if let Some(a) = args.as_ref()
                            && let Ok(p) = a.RequestedPlaybackPosition()
                        {
                            handler(MediaCommand::SeekTo(ticks_to_secs(p.Duration)));
                        }
                        Ok(())
                    },
                ))
                .map_err(err)?;
            Ok(Self {
                smtc,
                button_token,
                seek_token,
            })
        }

        pub fn set_now_playing(&self, np: &NowPlaying) {
            let _ = self.try_set_now_playing(np);
        }

        fn try_set_now_playing(&self, np: &NowPlaying) -> windows::core::Result<()> {
            let up = self.smtc.DisplayUpdater()?;
            up.SetType(MediaPlaybackType::Video)?;
            let v = up.VideoProperties()?;
            v.SetTitle(&HSTRING::from(np.title.as_str()))?;
            v.SetSubtitle(&HSTRING::from(np.subtitle.as_str()))?;
            if let Some(path) = &np.cover_file {
                let uri = format!("file:///{}", path.to_string_lossy().replace('\\', "/"));
                if let Ok(u) = Uri::CreateUri(&HSTRING::from(uri))
                    && let Ok(r) = RandomAccessStreamReference::CreateFromUri(&u)
                {
                    let _ = up.SetThumbnail(&r);
                }
            }
            up.Update()
        }

        pub fn set_status(&self, status: PlaybackStatus) {
            let s = match status {
                PlaybackStatus::Stopped => MediaPlaybackStatus::Stopped,
                PlaybackStatus::Playing => MediaPlaybackStatus::Playing,
                PlaybackStatus::Paused => MediaPlaybackStatus::Paused,
                PlaybackStatus::Changing => MediaPlaybackStatus::Changing,
            };
            let _ = self.smtc.SetPlaybackStatus(s);
        }

        pub fn set_buttons(&self, can_next: bool, can_previous: bool, can_seek_buttons: bool) {
            let _ = self.smtc.SetIsNextEnabled(can_next);
            let _ = self.smtc.SetIsPreviousEnabled(can_previous);
            let _ = self.smtc.SetIsFastForwardEnabled(can_seek_buttons);
            let _ = self.smtc.SetIsRewindEnabled(can_seek_buttons);
        }

        pub fn set_timeline(&self, position: f64, duration: f64) {
            let Ok(t) = SystemMediaTransportControlsTimelineProperties::new() else {
                return;
            };
            let span = |s: f64| TimeSpan {
                Duration: timespan_ticks(s),
            };
            let _ = t.SetStartTime(span(0.0));
            let _ = t.SetMinSeekTime(span(0.0));
            let _ = t.SetEndTime(span(duration));
            let _ = t.SetMaxSeekTime(span(duration));
            let _ = t.SetPosition(span(position.min(duration.max(0.0))));
            let _ = self.smtc.UpdateTimelineProperties(&t);
        }

        pub fn clear(&self) {
            self.set_status(PlaybackStatus::Stopped);
            if let Ok(up) = self.smtc.DisplayUpdater() {
                let _ = up.ClearAll();
                let _ = up.Update();
            }
        }

        pub fn is_active(&self) -> bool {
            true
        }
    }

    impl Drop for MediaControls {
        fn drop(&mut self) {
            let _ = self.smtc.RemoveButtonPressed(self.button_token);
            let _ = self
                .smtc
                .RemovePlaybackPositionChangeRequested(self.seek_token);
            self.clear();
            let _ = self.smtc.SetIsEnabled(false);
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::{CommandHandler, NowPlaying, PlaybackStatus};

    /// Auf dieser Plattform noch ohne Anbindung (siehe Modulbeschreibung).
    pub struct MediaControls;

    impl MediaControls {
        pub fn new(_hwnd: isize, _handler: CommandHandler) -> Result<Self, String> {
            Err("Medientasten sind auf diesem System noch nicht angebunden".to_owned())
        }
        pub fn set_now_playing(&self, _np: &NowPlaying) {}
        pub fn set_status(&self, _status: PlaybackStatus) {}
        pub fn set_buttons(&self, _next: bool, _previous: bool, _seek_buttons: bool) {}
        pub fn set_timeline(&self, _position: f64, _duration: f64) {}
        pub fn clear(&self) {}
        pub fn is_active(&self) -> bool {
            false
        }
    }
}

pub use imp::MediaControls;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticks_roundtrip() {
        assert_eq!(timespan_ticks(1.5), 15_000_000);
        assert_eq!(timespan_ticks(-3.0), 0);
        assert_eq!(timespan_ticks(f64::NAN), 0);
        assert!((ticks_to_secs(timespan_ticks(754.2)) - 754.2).abs() < 1e-6);
        assert_eq!(ticks_to_secs(-5), 0.0);
    }

    #[cfg(not(windows))]
    #[test]
    fn stub_reports_unavailable() {
        assert!(MediaControls::new(0, Arc::new(|_| {})).is_err());
    }
}
