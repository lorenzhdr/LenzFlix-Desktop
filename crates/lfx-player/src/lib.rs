//! Wiedergabe-Kern von LENZFLIX Desktop (libmpv, ohne UI).
//!
//! * [`Player`]: mpv-Thread mit Kommandos, Zustand ([`PlayerState`]) und Ereignissen.
//! * [`VideoOutput`] ([`D3d11Output`], [`GlOutput`]): Render-Ausgabe über Rohzeiger.
//! * [`prepare`]: PlaybackInfo → [`PlayRequest`] (Direktwiedergabe, Rückfall HLS).
//! * [`ProgressReporter`] / [`ReportSink`]: Fortschrittsmeldungen.
//! * [`segments`]: Intro/Abspann überspringen, nächste Folge.

mod player;
mod prepare;
mod reporter;
mod request;
pub mod segments;
mod state;
mod video;

pub use player::{Player, PlayerConfig, TrackChoice, VideoMode};
pub use prepare::{PrepareError, PrepareOptions, prepare, sanitize_transcode_url};
pub use reporter::{
    ApiSink, Planned, ProgressReporter, Report, ReportError, ReportPlanner, ReportSink,
    ReporterConfig, Trigger, build_report,
};
pub use request::{ExternalSubtitle, PlayRequest, StreamKind, StreamMap};
pub use state::{
    ChapterInfo, EndKind, PlayerError, PlayerEvent, PlayerState, Track, TrackKind, VideoParams,
};
pub use video::{D3d11Output, FrameReady, GetProcAddress, GlOutput, RenderTarget, VideoOutput};
