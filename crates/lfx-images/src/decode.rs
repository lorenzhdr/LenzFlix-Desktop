//! Dekodieren (JPEG, PNG, WebP) und hochwertiges Verkleinern auf RGBA8.

use std::io::Cursor;
use std::sync::Arc;

use fast_image_resize::images::Image;
use fast_image_resize::{PixelType, Resizer};

use crate::types::{DecodedImage, ImageError};

/// Zielgröße, die in `max_width` × `max_height` passt (Seitenverhältnis bleibt, nie vergrößern).
pub fn fit_within(width: u32, height: u32, max_w: Option<u32>, max_h: Option<u32>) -> (u32, u32) {
    let mut scale = 1.0_f64;
    if let Some(mw) = max_w.filter(|v| *v > 0) {
        scale = scale.min(f64::from(mw) / f64::from(width));
    }
    if let Some(mh) = max_h.filter(|v| *v > 0) {
        scale = scale.min(f64::from(mh) / f64::from(height));
    }
    if scale >= 1.0 {
        return (width, height);
    }
    let tw = (f64::from(width) * scale).round().max(1.0) as u32;
    let th = (f64::from(height) * scale).round().max(1.0) as u32;
    (tw.min(width), th.min(height))
}

/// Vormultipliziert die Farbkanäle (`c * a / 255`, gerundet) an Ort und Stelle.
pub fn premultiply(pixels: &mut [u8]) {
    for px in pixels.as_chunks_mut::<4>().0.iter_mut() {
        let a = u32::from(px[3]);
        if a == 255 {
            continue;
        }
        for c in &mut px[..3] {
            *c = ((u32::from(*c) * a + 127) / 255) as u8;
        }
    }
}

/// Dekodiert `bytes` (Format aus dem Inhalt erkannt) und skaliert bei Bedarf herunter.
pub fn decode_image(
    bytes: &[u8],
    max_w: Option<u32>,
    max_h: Option<u32>,
    premultiplied: bool,
) -> Result<DecodedImage, ImageError> {
    let reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| ImageError::Decode(e.to_string()))?;
    let img = reader
        .decode()
        .map_err(|e| ImageError::Decode(e.to_string()))?;
    let has_alpha = img.color().has_alpha();
    let rgba = img.into_rgba8();
    let (w, h) = rgba.dimensions();
    if w == 0 || h == 0 {
        return Err(ImageError::Decode("leeres Bild".into()));
    }
    let (tw, th) = fit_within(w, h, max_w, max_h);
    let mut pixels = if (tw, th) == (w, h) {
        rgba.into_raw()
    } else {
        let src = Image::from_vec_u8(w, h, rgba.into_raw(), PixelType::U8x4)
            .map_err(|e| ImageError::Decode(e.to_string()))?;
        let mut dst = Image::new(tw, th, PixelType::U8x4);
        // Standard: Lanczos3, Alpha wird beim Skalieren berücksichtigt (gerade ein, gerade aus).
        Resizer::new()
            .resize(&src, &mut dst, None)
            .map_err(|e| ImageError::Decode(e.to_string()))?;
        dst.into_vec()
    };
    if premultiplied && has_alpha {
        premultiply(&mut pixels);
    }
    Ok(DecodedImage {
        width: tw,
        height: th,
        pixels: Arc::from(pixels),
        premultiplied,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::chunks_exact_to_as_chunks)]
    use super::*;
    use image::{ImageEncoder, codecs};

    fn gradient(w: u32, h: u32, alpha: u8) -> image::RgbaImage {
        image::RgbaImage::from_fn(w, h, |x, y| {
            image::Rgba([(x * 255 / w) as u8, (y * 255 / h) as u8, 90, alpha])
        })
    }

    fn png(img: &image::RgbaImage) -> Vec<u8> {
        let mut out = Vec::new();
        codecs::png::PngEncoder::new(&mut out)
            .write_image(
                img.as_raw(),
                img.width(),
                img.height(),
                image::ExtendedColorType::Rgba8,
            )
            .unwrap();
        out
    }

    fn jpeg(img: &image::RgbaImage) -> Vec<u8> {
        let rgb = image::DynamicImage::ImageRgba8(img.clone()).into_rgb8();
        let mut out = Vec::new();
        codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90)
            .write_image(
                rgb.as_raw(),
                rgb.width(),
                rgb.height(),
                image::ExtendedColorType::Rgb8,
            )
            .unwrap();
        out
    }

    fn webp(img: &image::RgbaImage) -> Vec<u8> {
        let mut out = Vec::new();
        codecs::webp::WebPEncoder::new_lossless(&mut out)
            .write_image(
                img.as_raw(),
                img.width(),
                img.height(),
                image::ExtendedColorType::Rgba8,
            )
            .unwrap();
        out
    }

    #[test]
    fn fit_keeps_aspect_and_never_upscales() {
        assert_eq!(fit_within(400, 600, Some(200), None), (200, 300));
        assert_eq!(fit_within(400, 600, None, Some(150)), (100, 150));
        assert_eq!(fit_within(400, 600, Some(200), Some(150)), (100, 150));
        assert_eq!(fit_within(400, 600, Some(800), Some(900)), (400, 600));
        assert_eq!(fit_within(400, 600, None, None), (400, 600));
        assert_eq!(fit_within(1000, 1, Some(10), None), (10, 1));
    }

    #[test]
    fn decodes_png_exactly() {
        let src = gradient(8, 6, 255);
        let out = decode_image(&png(&src), None, None, false).unwrap();
        assert_eq!((out.width, out.height), (8, 6));
        assert_eq!(&*out.pixels, src.as_raw().as_slice());
        assert!(!out.premultiplied);
    }

    #[test]
    fn decodes_jpeg_and_webp() {
        let src = gradient(32, 24, 255);
        let j = decode_image(&jpeg(&src), None, None, false).unwrap();
        assert_eq!((j.width, j.height), (32, 24));
        // JPEG ist verlustbehaftet: Mittelwert nahe am Original.
        let mean = |p: &[u8]| {
            p.chunks_exact(4).map(|c| u64::from(c[0])).sum::<u64>() / (p.len() as u64 / 4)
        };
        assert!(mean(&j.pixels).abs_diff(mean(src.as_raw())) < 6);
        let w = decode_image(&webp(&src), None, None, false).unwrap();
        assert_eq!(&*w.pixels, src.as_raw().as_slice());
    }

    #[test]
    fn downscales_with_aspect() {
        let src = gradient(400, 600, 255);
        let out = decode_image(&png(&src), Some(200), Some(1000), false).unwrap();
        assert_eq!((out.width, out.height), (200, 300));
        assert_eq!(out.pixels.len(), 200 * 300 * 4);
        // Verlauf bleibt ein Verlauf: links dunkler als rechts im Rotkanal.
        let at = |x: usize, y: usize| out.pixels[(y * 200 + x) * 4];
        assert!(at(10, 100) < at(180, 100));
    }

    #[test]
    fn premultiplies_only_when_asked() {
        let src = gradient(4, 4, 128);
        let straight = decode_image(&png(&src), None, None, false).unwrap();
        let pre = decode_image(&png(&src), None, None, true).unwrap();
        assert!(pre.premultiplied);
        for (s, p) in straight
            .pixels
            .chunks_exact(4)
            .zip(pre.pixels.chunks_exact(4))
        {
            assert_eq!(p[3], 128);
            assert_eq!(u32::from(p[0]), (u32::from(s[0]) * 128 + 127) / 255);
        }
    }

    #[test]
    fn scaling_respects_alpha() {
        // Rot und deckend neben transparentem Schwarz: die Farbe darf nicht abdunkeln.
        let src = image::RgbaImage::from_fn(16, 16, |x, _| {
            if x < 8 {
                image::Rgba([255, 0, 0, 255])
            } else {
                image::Rgba([0, 0, 0, 0])
            }
        });
        let out = decode_image(&png(&src), Some(4), Some(4), false).unwrap();
        let first = &out.pixels[0..4];
        assert_eq!(first[3], 255);
        assert!(first[0] > 240, "Rot bleibt erhalten, ist {first:?}");
    }

    #[test]
    fn garbage_is_an_error() {
        let err = decode_image(b"das ist kein bild", None, None, false).unwrap_err();
        assert!(matches!(err, ImageError::Decode(_)));
    }
}
