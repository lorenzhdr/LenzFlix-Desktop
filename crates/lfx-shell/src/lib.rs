//! Hauptfenster von LENZFLIX Desktop.
//!
//! - Rahmenloses Fenster mit eigener Titelleiste (von der UI gezeichnet). Die Shell stellt
//!   `drag_window`, `minimize`, `toggle_maximize`, `close` und [`ShellHandle::set_hit_regions`] bereit.
//! - Windows (Modell A): DirectComposition-Baum mit [`VideoLayer`] (D3D11-Swapchain, unten) und der
//!   UI (Slint mit femtovg-wgpu auf DX12, premultiplied alpha, oben). Eine eigene
//!   `slint::platform::Platform` leitet winit-Ereignisse an Slint weiter.
//! - Sonst (macOS, CI): normales Slint-winit-Backend, kein Video-Layer ([`ShellHandle::video_layer`]
//!   liefert `None`).
//!
//! Ablauf: [`Shell::new`] anlegen, danach das Slint-Hauptfenster erzeugen und anzeigen, dann
//! [`Shell::run`]. Auf Nicht-Windows-Systemen vorher [`Shell::attach`] mit dem Slint-Fenster aufrufen.

pub mod hit;
pub mod stats;
pub mod video;

#[cfg(not(windows))]
mod other;
#[cfg(windows)]
mod win;

pub use hit::{HitRegions, HitResult, Rect, hit_test};
pub use stats::{FrameSummary, FrameTimes};
pub use video::{
    ContentKind, HdrMetadata, OutputHdrInfo, VideoFormat, VideoRect, VideoTarget, plan_format,
};

#[cfg(not(windows))]
pub use other::{Shell, ShellHandle, VideoLayer};
#[cfg(windows)]
pub use win::{Shell, ShellHandle, VideoLayer};

/// Welcher Pfad die UI zeichnet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiBackend {
    /// femtovg über wgpu (DX12) direkt auf ein DirectComposition-Visual.
    GpuComposition,
    /// Slint-Software-Renderer mit Dirty-Rects, Upload in eine Composition-Swapchain.
    Software,
    /// Standard-Slint-Backend (macOS und sonstige).
    Native,
}

/// Wunsch für den UI-Pfad.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UiBackendPref {
    /// GPU, bei Fehlschlag Software.
    #[default]
    Auto,
    /// Nur Software (Messung des Rückfalls, GPU-lose Rechner).
    Software,
}

/// Fensteroptionen.
#[derive(Clone, Debug)]
pub struct ShellOptions {
    pub title: String,
    /// Anfangsgröße (logische Pixel).
    pub size: (f32, f32),
    /// Mindestgröße (logische Pixel).
    pub min_size: (f32, f32),
    pub maximized: bool,
    pub ui_backend: UiBackendPref,
    /// Startformat des Video-Layers.
    pub video_format: VideoFormat,
}

impl Default for ShellOptions {
    fn default() -> Self {
        Self {
            title: "LENZFLIX".into(),
            size: (1280.0, 800.0),
            min_size: (640.0, 420.0),
            maximized: false,
            ui_backend: UiBackendPref::Auto,
            video_format: VideoFormat::Bgra8Srgb,
        }
    }
}

/// Fehler der Shell.
#[derive(Debug, thiserror::Error)]
pub enum ShellError {
    #[error("Fenster konnte nicht erstellt werden: {0}")]
    Window(String),
    #[error("Grafikinitialisierung fehlgeschlagen: {0}")]
    Graphics(String),
    #[error("Slint-Plattform: {0}")]
    Platform(String),
    #[error("Ereignisschleife: {0}")]
    EventLoop(String),
    #[error("nicht unterstützt auf dieser Plattform: {0}")]
    Unsupported(&'static str),
}
