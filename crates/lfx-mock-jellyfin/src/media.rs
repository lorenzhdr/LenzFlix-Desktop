//! Medien des Mocks: ein kleines synthetisches Testvideo mit HTTP-Range-Unterstützung und ein
//! erzeugtes Platzhalter-Cover (gültiges PNG).
//!
//! Das Video wird nie eingecheckt, sondern zur Testzeit erzeugt ([`generate_test_video`], braucht
//! `ffmpeg` im Suchpfad) und dem Mock per [`set_video_file`] oder der Umgebungsvariable
//! `LFX_MOCK_VIDEO_FILE` bekannt gemacht. Ohne Datei antwortet der Mock wie bisher mit Platzhalterbytes.
//! Nur Standardbibliothek, keine zusätzlichen Abhängigkeiten.

use super::{S, find};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use std::path::{Path as FsPath, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;

/// Name der Umgebungsvariable mit dem Pfad der Videodatei.
pub const VIDEO_ENV: &str = "LFX_MOCK_VIDEO_FILE";

static VIDEO_FILE: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Legt die Videodatei fest, die unter dem Direct-Stream-Pfad ausgeliefert wird (`None` = Platzhalter).
pub fn set_video_file(path: Option<PathBuf>) {
    *VIDEO_FILE.lock().unwrap_or_else(|e| e.into_inner()) = path;
}

pub(crate) fn video_file() -> Option<PathBuf> {
    // W5-downloads: pub(crate)
    let set = VIDEO_FILE.lock().unwrap_or_else(|e| e.into_inner()).clone();
    set.or_else(|| {
        std::env::var_os(VIDEO_ENV)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    })
    .filter(|p| p.is_file())
}

/// Erzeugt ein stummes Testvideo (Testbild, h264, `seconds` Sekunden lang) mit `ffmpeg`.
///
/// Gibt `Err` mit einem Text zurück, wenn `ffmpeg` fehlt oder scheitert; Tests überspringen sich dann.
pub fn generate_test_video(path: &FsPath, seconds: u32) -> Result<(), String> {
    let source = format!("testsrc2=size=640x360:rate=30:duration={seconds}");
    let run = |codec: &[&str]| -> Result<(), String> {
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
            .args(codec)
            .args([
                "-an",
                "-pix_fmt",
                "yuv420p",
                "-g",
                "30",
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
    };
    run(&["-c:v", "libx264", "-preset", "ultrafast", "-crf", "30"])
        .or_else(|_| run(&["-c:v", "mpeg4", "-q:v", "8"]))
}

/// Auslieferung mit `Range`-Unterstützung (`bytes=a-b`, `a-`, `-n`). Ohne Videodatei der alte Platzhalter.
pub(crate) async fn stream(state: S, Path(id): Path<String>, headers: HeaderMap) -> Response {
    let State(shared) = &state;
    if find(shared, &id).is_none() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(file) = video_file() else {
        return super::stream(state, Path(id)).await;
    };
    let Ok(data) = std::fs::read(&file) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let ctype = if file.extension().is_some_and(|e| e == "mkv") {
        "video/x-matroska"
    } else {
        "video/mp4"
    };
    let total = data.len() as u64;
    let range = headers.get(header::RANGE).and_then(|v| v.to_str().ok());
    match range.map(|r| parse_range(r, total)) {
        None => (
            [
                (header::CONTENT_TYPE, ctype.to_owned()),
                (header::ACCEPT_RANGES, "bytes".to_owned()),
            ],
            data,
        )
            .into_response(),
        Some(Some((a, b))) => (
            StatusCode::PARTIAL_CONTENT,
            [
                (header::CONTENT_TYPE, ctype.to_owned()),
                (header::ACCEPT_RANGES, "bytes".to_owned()),
                (header::CONTENT_RANGE, format!("bytes {a}-{b}/{total}")),
            ],
            data[a as usize..=b as usize].to_vec(),
        )
            .into_response(),
        Some(None) => (
            StatusCode::RANGE_NOT_SATISFIABLE,
            [(header::CONTENT_RANGE, format!("bytes */{total}"))],
        )
            .into_response(),
    }
}

/// Zerlegt einen `Range`-Header (nur ein Bereich) in `(Anfang, Ende)` inklusive; `None` = nicht erfüllbar.
pub(crate) fn parse_range(value: &str, total: u64) -> Option<(u64, u64)> {
    let spec = value.trim().strip_prefix("bytes=")?;
    if spec.contains(',') || total == 0 {
        return None;
    }
    let (from, to) = spec.split_once('-')?;
    let (a, b) = if from.is_empty() {
        let n: u64 = to.parse().ok()?;
        if n == 0 {
            return None;
        }
        (total.saturating_sub(n), total - 1)
    } else {
        let a: u64 = from.parse().ok()?;
        let b = if to.is_empty() {
            total - 1
        } else {
            to.parse::<u64>().ok()?.min(total - 1)
        };
        (a, b)
    };
    (a <= b && a < total).then_some((a, b))
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + u32::from(x)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], body: &[u8]) {
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    let mut crc_in = kind.to_vec();
    crc_in.extend_from_slice(body);
    out.extend_from_slice(&crc_in);
    out.extend_from_slice(&crc32(&crc_in).to_be_bytes());
}

/// Erzeugt ein Platzhalter-Cover als gültiges PNG (Verlauf mit Streifen, `seed` bestimmt die Farbe).
/// Ungepackt (gespeicherte Deflate-Blöcke), also nur für kleine Bilder gedacht.
pub fn cover_png(width: u32, height: u32, seed: u32) -> Vec<u8> {
    let hue = seed.wrapping_mul(67) % 360;
    let mut raw = Vec::with_capacity(((width * 3 + 1) * height) as usize);
    for y in 0..height {
        raw.push(0); // Filter „keiner“
        for x in 0..width {
            let t = (y * 100 / height.max(1) + x * 20 / width.max(1)) as i64;
            let stripe = ((x + y) * 5 / width.max(1)).is_multiple_of(2);
            let base = [
                (60 + (hue as i64 * 2) % 120 - t / 2).clamp(0, 255),
                (90 + (hue as i64 * 3) % 100 - t / 2).clamp(0, 255),
                (140 + (hue as i64 * 5) % 90 - t).clamp(0, 255),
            ];
            for c in base {
                raw.push(if stripe { (c * 9 / 10) as u8 } else { c as u8 });
            }
        }
    }
    let mut z = vec![0x78, 0x01];
    let mut blocks = raw.chunks(65_535).peekable();
    while let Some(b) = blocks.next() {
        z.push(u8::from(blocks.peek().is_none()));
        z.extend_from_slice(&(b.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(b.len() as u16)).to_le_bytes());
        z.extend_from_slice(b);
    }
    z.extend_from_slice(&adler32(&raw).to_be_bytes());
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]); // 8 Bit, RGB
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_forms() {
        assert_eq!(parse_range("bytes=0-9", 100), Some((0, 9)));
        assert_eq!(parse_range("bytes=90-", 100), Some((90, 99)));
        assert_eq!(parse_range("bytes=-10", 100), Some((90, 99)));
        assert_eq!(parse_range("bytes=50-500", 100), Some((50, 99)));
        assert_eq!(parse_range("bytes=100-", 100), None);
        assert_eq!(parse_range("bytes=5-2", 100), None);
        assert_eq!(parse_range("bytes=0-1,5-6", 100), None);
        assert_eq!(parse_range("items=0-1", 100), None);
    }

    #[test]
    fn crc_and_adler_known_values() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }

    #[test]
    fn cover_png_structure() {
        let png = cover_png(32, 18, 3);
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        assert_eq!(&png[12..16], b"IHDR");
        assert_eq!(&png[png.len() - 8..png.len() - 4], b"IEND");
    }
}
