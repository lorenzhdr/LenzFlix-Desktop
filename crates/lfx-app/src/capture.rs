//! Bildschirmfoto des Fensters (Entwicklung, Tests): Windows liest die zusammengesetzte Client-Fläche
//! aus dem Desktop (enthält auch das Video), sonst bleibt der Slint-Schnappschuss der UI.

use std::path::Path;

/// Schreibt RGBA8-Pixel als PNG.
pub fn write_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().map_err(|e| e.to_string())?;
    w.write_image_data(rgba).map_err(|e| e.to_string())
}

/// Liest die Client-Fläche eines Fensters vom Desktop (RGBA8, deckend). `None`, wenn das nicht gelingt
/// (z. B. Sitzung gesperrt, Fenster minimiert).
#[cfg(windows)]
pub fn capture_hwnd(hwnd: isize) -> Option<(u32, u32, Vec<u8>)> {
    use std::ffi::c_void;
    use windows::Win32::Foundation::{HWND, POINT, RECT};
    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, ClientToScreen,
        CreateCompatibleBitmap, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC,
        GetDIBits, ReleaseDC, SRCCOPY, SelectObject,
    };
    use windows::Win32::UI::WindowsAndMessaging::GetClientRect;

    let hwnd = HWND(hwnd as *mut c_void);
    // SAFETY: reine GDI-Aufrufe mit selbst angelegten Objekten, die vor der Rückkehr freigegeben werden.
    unsafe {
        let mut rc = RECT::default();
        GetClientRect(hwnd, &mut rc).ok()?;
        let (w, h) = (rc.right - rc.left, rc.bottom - rc.top);
        if w <= 0 || h <= 0 {
            return None;
        }
        let mut origin = POINT { x: 0, y: 0 };
        let _ = ClientToScreen(hwnd, &mut origin);
        let screen = GetDC(None);
        if screen.is_invalid() {
            return None;
        }
        let mem = CreateCompatibleDC(Some(screen));
        let bmp = CreateCompatibleBitmap(screen, w, h);
        let old = SelectObject(mem, bmp.into());
        let copied = BitBlt(
            mem,
            0,
            0,
            w,
            h,
            Some(screen),
            origin.x,
            origin.y,
            SRCCOPY | CAPTUREBLT,
        );
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut buf = vec![0u8; (w * h * 4) as usize];
        let lines = GetDIBits(
            mem,
            bmp,
            0,
            h as u32,
            Some(buf.as_mut_ptr().cast()),
            &mut info,
            DIB_RGB_COLORS,
        );
        SelectObject(mem, old);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        if copied.is_err() || lines == 0 {
            return None;
        }
        for px in buf.as_chunks_mut::<4>().0 {
            px.swap(0, 2);
            px[3] = 255;
        }
        Some((w as u32, h as u32, buf))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a").join("t.png");
        let rgba = vec![200u8; 4 * 2 * 4];
        write_png(&path, 4, 2, &rgba).unwrap();
        let dec = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(&path).unwrap()));
        let mut reader = dec.read_info().unwrap();
        let mut out = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut out).unwrap();
        assert_eq!((info.width, info.height), (4, 2));
        assert_eq!(&out[..4], &[200, 200, 200, 200]);
    }
}
