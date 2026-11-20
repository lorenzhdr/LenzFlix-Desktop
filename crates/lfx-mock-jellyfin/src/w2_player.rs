//! Mock-Erweiterungen für den Player (W2-player): Testvideo mit Kapiteln, zwei Tonspuren und Untertitel,
//! feste Medien-Segmente passend zur Videolänge und Trickplay-Kacheln (synthetisch).

use super::{S, find, jv};
use axum::extract::Path;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::json;
use std::collections::HashMap;
use std::path::Path as FsPath;
use std::process::{Command, Stdio};
use std::sync::Mutex;

/// Segment-Überschreibung: (Art, Beginn, Ende) in Sekunden.
pub type SegmentSpec = (String, f64, f64);

static SEGMENTS: Mutex<Option<Vec<SegmentSpec>>> = Mutex::new(None);
static TILES: Mutex<Option<HashMap<u32, Vec<u8>>>> = Mutex::new(None);

/// Legt feste Segmente fest (passend zur echten Videolänge); `None` = Standardwerte des Mocks.
pub fn set_segments(segments: Option<Vec<SegmentSpec>>) {
    *SEGMENTS.lock().unwrap_or_else(|e| e.into_inner()) = segments;
}

/// Segmente für `GET /MediaSegments/{id}`, falls überschrieben.
pub(crate) fn try_segments(id: &str) -> Option<Response> {
    let spec = SEGMENTS.lock().unwrap_or_else(|e| e.into_inner()).clone()?;
    let t = 10_000_000.0;
    let items: Vec<_> = spec
        .iter()
        .enumerate()
        .map(|(i, (kind, a, b))| {
            json!({ "Id": format!("w2seg{i}"), "ItemId": id, "Type": kind,
                    "StartTicks": (a * t) as i64, "EndTicks": (b * t) as i64 })
        })
        .collect();
    Some(jv(
        json!({ "Items": items, "TotalRecordCount": items.len(), "StartIndex": 0 }),
    ))
}

/// Erzeugt ein Testvideo (Matroska) mit Testbild, zwei Tonspuren (Deutsch/Englisch), einem deutschen
/// Untertitel und drei Kapiteln. Braucht `ffmpeg`; nie eingecheckt.
pub fn generate_rich_video(path: &FsPath, seconds: u32) -> Result<(), String> {
    let dir = path.parent().unwrap_or_else(|| FsPath::new("."));
    let srt = dir.join("w2-untertitel.srt");
    let meta = dir.join("w2-kapitel.txt");
    let stamp = |s: u32| format!("{:02}:{:02}:{:02},000", s / 3600, (s / 60) % 60, s % 60);
    let mut cues = String::new();
    let mut i = 1;
    let mut t = 1;
    while t + 3 <= seconds {
        cues.push_str(&format!(
            "{i}\n{} --> {}\nDies ist Untertitel Nummer {i}.\n\n",
            stamp(t),
            stamp(t + 3)
        ));
        i += 1;
        t += 5;
    }
    std::fs::write(&srt, cues).map_err(|e| e.to_string())?;
    let ms = |s: f64| (s * 1000.0) as u64;
    let d = f64::from(seconds);
    let mut m = String::from(";FFMETADATA1\n");
    for (a, b, title) in [
        (0.0, d / 3.0, "Anfang"),
        (d / 3.0, 2.0 * d / 3.0, "Mitte"),
        (2.0 * d / 3.0, d, "Ende"),
    ] {
        m.push_str(&format!(
            "[CHAPTER]\nTIMEBASE=1/1000\nSTART={}\nEND={}\ntitle={title}\n",
            ms(a),
            ms(b)
        ));
    }
    std::fs::write(&meta, m).map_err(|e| e.to_string())?;
    let run = |vcodec: &[&str]| -> Result<(), String> {
        let status = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y"])
            .args(["-f", "lavfi", "-i"])
            .arg(format!("testsrc2=size=640x360:rate=30:duration={seconds}"))
            .args(["-f", "lavfi", "-i"])
            .arg(format!("sine=frequency=440:duration={seconds}"))
            .args(["-f", "lavfi", "-i"])
            .arg(format!("sine=frequency=660:duration={seconds}"))
            .arg("-i")
            .arg(&srt)
            .arg("-i")
            .arg(&meta)
            .args(["-map", "0:v", "-map", "1:a", "-map", "2:a", "-map", "3:s"])
            .args(["-map_metadata", "4", "-map_chapters", "4"])
            .args(vcodec)
            .args(["-pix_fmt", "yuv420p", "-g", "30"])
            .args(["-c:a", "aac", "-c:s", "srt"])
            .args([
                "-metadata:s:a:0",
                "language=ger",
                "-metadata:s:a:1",
                "language=eng",
            ])
            .args(["-metadata:s:s:0", "language=ger"])
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
    };
    run(&["-c:v", "libx264", "-preset", "ultrafast", "-crf", "30"])
        .or_else(|_| run(&["-c:v", "mpeg4", "-q:v", "8"]))
}

/// Kachelbild (10 × 10 Vorschauen à 320 × 180, jede Zelle eine eigene Farbe), PNG.
fn tile_png(tile: u32) -> Vec<u8> {
    const CW: usize = 320;
    const CH: usize = 180;
    const N: usize = 10;
    let (w, h) = (CW * N, CH * N);
    let mut rgb = vec![0u8; w * h * 3];
    for y in 0..h {
        let (cy, ly) = (y / CH, y % CH);
        for x in 0..w {
            let (cx, lx) = (x / CW, x % CW);
            let idx = (cy * N + cx) as u32 + tile * 100;
            let o = (y * w + x) * 3;
            let base = (idx.wrapping_mul(37) % 200) as u8;
            let edge = lx < 4 || ly < 4;
            rgb[o] = if edge { 255 } else { base / 2 + 30 };
            rgb[o + 1] = if edge {
                255
            } else {
                ((lx * 120 / CW) as u8).saturating_add(base / 3)
            };
            rgb[o + 2] = if edge {
                255
            } else {
                110 + (ly * 100 / CH) as u8
            };
        }
    }
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w as u32, h as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Fast);
        if let Ok(mut wr) = enc.write_header() {
            let _ = wr.write_image_data(&rgb);
        }
    }
    out
}

/// `GET /Videos/{id}/Trickplay/{breite}/{datei}` mit `datei` = `<n>.jpg` (liefert PNG-Bytes, das Format
/// erkennt der Client am Inhalt).
pub(crate) async fn trickplay_tile(
    state: S,
    Path((id, _width, file)): Path<(String, String, String)>,
) -> Response {
    if find(&state.0, &id).is_none() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(n) = file.split('.').next().and_then(|s| s.parse::<u32>().ok()) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let bytes = tokio::task::spawn_blocking(move || {
        let mut g = TILES.lock().unwrap_or_else(|e| e.into_inner());
        g.get_or_insert_with(HashMap::new)
            .entry(n)
            .or_insert_with(|| tile_png(n))
            .clone()
    })
    .await
    .unwrap_or_default();
    ([(header::CONTENT_TYPE, "image/png")], bytes).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_png_has_expected_size() {
        let png_bytes = tile_png(0);
        let dec = png::Decoder::new(std::io::Cursor::new(png_bytes));
        let r = dec.read_info().unwrap();
        assert_eq!(r.info().width, 3200);
        assert_eq!(r.info().height, 1800);
    }
}
