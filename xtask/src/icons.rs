//! App-Icons (ICO, ICNS, PNG) aus dem Logo erzeugen, rein in Rust (resvg), ohne Fremdwerkzeuge.

use std::fmt::Write as _;

type Res<T> = Result<T, Box<dyn std::error::Error>>;

/// Quell-Logo (Zeichen „L“), wird auf dunkler, abgerundeter Fläche zentriert.
const LOGO_SVG: &str = include_str!("../../assets/logo/lenzflix-l.svg");

/// Pfaddaten und Maße des Logos aus der SVG-Datei lesen.
fn logo_path() -> Res<(String, f64, f64)> {
    let d = LOGO_SVG
        .split(" d=\"")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .ok_or("Logo-SVG ohne Pfad")?;
    let vb = LOGO_SVG
        .split("viewBox=\"")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .ok_or("Logo-SVG ohne viewBox")?;
    let nums: Vec<f64> = vb
        .split_whitespace()
        .filter_map(|n| n.parse().ok())
        .collect();
    if nums.len() != 4 {
        return Err("viewBox nicht lesbar".into());
    }
    Ok((d.to_string(), nums[2], nums[3]))
}

/// Das zusammengesetzte Icon als SVG (1024 x 1024).
pub fn icon_svg() -> Res<String> {
    let (d, w, h) = logo_path()?;
    let scale = 640.0 / h;
    let tx = (1024.0 - w * scale) / 2.0;
    let ty = (1024.0 - h * scale) / 2.0;
    let mut s = String::new();
    write!(
        s,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 1024 1024\">\
         <rect width=\"1024\" height=\"1024\" rx=\"224\" fill=\"#141414\"/>\
         <g transform=\"translate({tx:.2} {ty:.2}) scale({scale:.5})\">\
         <path fill=\"#E50914\" fill-rule=\"nonzero\" d=\"{d}\"/></g></svg>"
    )?;
    Ok(s)
}

/// Rendert das Icon als PNG in der Kantenlänge `size`.
pub fn render_png(size: u32) -> Res<Vec<u8>> {
    let svg = icon_svg()?;
    let tree = resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default())?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size).ok_or("Pixmap")?;
    let k = size as f32 / 1024.0;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(k, k),
        &mut pixmap.as_mut(),
    );
    Ok(pixmap.encode_png()?)
}

/// ICO mit PNG-komprimierten Bildern (16 bis 256 px).
pub fn make_ico() -> Res<Vec<u8>> {
    const SIZES: [u32; 7] = [16, 24, 32, 48, 64, 128, 256];
    let images: Vec<(u32, Vec<u8>)> = SIZES
        .iter()
        .map(|&s| render_png(s).map(|p| (s, p)))
        .collect::<Res<_>>()?;
    let mut out = Vec::new();
    out.extend_from_slice(&[0, 0, 1, 0]);
    out.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len() as u32;
    for (s, png) in &images {
        // 256 wird als 0 geschrieben.
        let dim = if *s >= 256 { 0 } else { *s as u8 };
        out.extend_from_slice(&[dim, dim, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes()); // Farbebenen
        out.extend_from_slice(&32u16.to_le_bytes()); // Bit pro Pixel
        out.extend_from_slice(&(png.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += png.len() as u32;
    }
    for (_, png) in &images {
        out.extend_from_slice(png);
    }
    Ok(out)
}

/// ICNS mit PNG-Einträgen (16 bis 1024 px).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn make_icns() -> Res<Vec<u8>> {
    const ENTRIES: [(&[u8; 4], u32); 7] = [
        (b"icp4", 16),
        (b"icp5", 32),
        (b"icp6", 64),
        (b"ic07", 128),
        (b"ic08", 256),
        (b"ic09", 512),
        (b"ic10", 1024),
    ];
    let mut body = Vec::new();
    for (ty, size) in ENTRIES {
        let png = render_png(size)?;
        body.extend_from_slice(ty);
        body.extend_from_slice(&(png.len() as u32 + 8).to_be_bytes());
        body.extend_from_slice(&png);
    }
    let mut out = Vec::new();
    out.extend_from_slice(b"icns");
    out.extend_from_slice(&(body.len() as u32 + 8).to_be_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_is_valid_and_sized() {
        let p = render_png(64).unwrap();
        assert_eq!(&p[..8], b"\x89PNG\r\n\x1a\n");
    }

    #[test]
    fn ico_header_and_entries() {
        let ico = make_ico().unwrap();
        assert_eq!(&ico[..4], &[0, 0, 1, 0]);
        assert_eq!(u16::from_le_bytes([ico[4], ico[5]]), 7);
        // Erster Eintrag zeigt auf ein PNG.
        let off = u32::from_le_bytes(ico[18..22].try_into().unwrap()) as usize;
        assert_eq!(&ico[off..off + 4], b"\x89PNG");
    }

    #[test]
    fn icns_header_and_length() {
        let icns = make_icns().unwrap();
        assert_eq!(&icns[..4], b"icns");
        let len = u32::from_be_bytes(icns[4..8].try_into().unwrap()) as usize;
        assert_eq!(len, icns.len());
    }
}
