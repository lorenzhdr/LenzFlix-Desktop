//! Mock-Erweiterungen für den TV-Modus (W2-tvui): ein langes, kleines Testvideo (der TV-Modus startet mitten in der
//! Sendung) und ein Schalter, der die Wiedergabe-Vorbereitung scheitern lässt (Fehlerkarte mit Code).

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use std::path::Path as FsPath;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};

static FAIL_PLAYBACK: AtomicBool = AtomicBool::new(false);

/// Lässt jede PlaybackInfo-Anfrage mit 503 scheitern (`true`) oder normal antworten (`false`).
pub fn set_fail_playback(on: bool) {
    FAIL_PLAYBACK.store(on, Ordering::Relaxed);
}

/// Hängt sich an den Anfang des PlaybackInfo-Handlers.
pub(crate) fn try_playback_info() -> Option<Response> {
    FAIL_PLAYBACK
        .load(Ordering::Relaxed)
        .then(|| StatusCode::SERVICE_UNAVAILABLE.into_response())
}

/// Erzeugt ein kleines, langes Testvideo (320x180, 1 Bild/s, h264) mit `ffmpeg`.
pub fn generate_long_test_video(path: &FsPath, seconds: u32) -> Result<(), String> {
    let source = format!("testsrc2=size=320x180:rate=1:duration={seconds}");
    let status = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
        ])
        .arg(&source)
        .args([
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-crf",
            "35",
            "-an",
            "-pix_fmt",
            "yuv420p",
            "-g",
            "10",
            "-movflags",
            "+faststart",
        ])
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("ffmpeg nicht startbar: {e}"))?;
    if status.success() && path.is_file() {
        Ok(())
    } else {
        Err(format!("ffmpeg endete mit {status}"))
    }
}
