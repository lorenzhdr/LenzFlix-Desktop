//! Blurhash-Platzhalter: Jellyfin liefert `ImageBlurHashes` (`{ "Primary": { "<tag>": "<hash>" } }`).

use std::collections::HashMap;
use std::sync::Arc;

use lfx_api::ImageType;

use crate::types::{DecodedImage, ImageError};

/// Größte erlaubte Kantenlänge eines Platzhalters.
pub const MAX_PLACEHOLDER_SIDE: u32 = 256;

/// Dekodiert einen Blurhash zu einem kleinen RGBA8-Bild (deckend).
pub fn decode_blurhash(hash: &str, width: u32, height: u32) -> Result<DecodedImage, ImageError> {
    if width == 0 || height == 0 || width > MAX_PLACEHOLDER_SIDE || height > MAX_PLACEHOLDER_SIDE {
        return Err(ImageError::Invalid("Platzhaltergröße".into()));
    }
    let pixels = blurhash::decode(hash, width, height, 1.0)
        .map_err(|e| ImageError::Invalid(format!("Blurhash: {e}")))?;
    Ok(DecodedImage {
        width,
        height,
        pixels: Arc::from(pixels),
        premultiplied: false,
    })
}

/// Platzhaltergröße für ein Seitenverhältnis (Breite / Höhe); die längere Seite hat `long_side` Pixel.
pub fn placeholder_size(aspect: f32, long_side: u32) -> (u32, u32) {
    let long_side = long_side.clamp(1, MAX_PLACEHOLDER_SIDE);
    let aspect = if aspect.is_finite() && aspect > 0.0 {
        aspect
    } else {
        1.0
    };
    if aspect >= 1.0 {
        (
            long_side,
            ((long_side as f32 / aspect).round() as u32).max(1),
        )
    } else {
        (
            ((long_side as f32 * aspect).round() as u32).max(1),
            long_side,
        )
    }
}

/// Sucht den Blurhash zu Bildart und Tag. Ohne passenden Tag gilt der Eintrag mit dem
/// kleinsten Tag (deterministisch), damit auch Bilder ohne bekannten Tag einen Platzhalter bekommen.
pub fn blurhash_for<'a>(
    hashes: &'a HashMap<String, HashMap<String, String>>,
    kind: ImageType,
    tag: Option<&str>,
) -> Option<&'a str> {
    let by_tag = hashes.get(kind.as_str())?;
    if let Some(hash) = tag.and_then(|t| by_tag.get(t)) {
        return Some(hash);
    }
    by_tag
        .iter()
        .min_by_key(|(t, _)| t.as_str())
        .map(|(_, h)| h.as_str())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::chunks_exact_to_as_chunks)]
    use super::*;

    const B83: &[u8] =
        b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz#$%*+,-.:;=?@[]^_{|}~";

    /// Blurhash mit nur einer Komponente (1x1) laut Spezifikation: Kopf `0`, Maximalwert `0`,
    /// danach die DC-Farbe (sRGB, 24 Bit) in vier Base-83-Zeichen.
    fn flat_hash(rgb: [u8; 3]) -> String {
        let mut v = (u32::from(rgb[0]) << 16) | (u32::from(rgb[1]) << 8) | u32::from(rgb[2]);
        let mut dc = [0u8; 4];
        for slot in dc.iter_mut().rev() {
            *slot = B83[(v % 83) as usize];
            v /= 83;
        }
        format!("00{}", String::from_utf8(dc.to_vec()).unwrap())
    }

    #[test]
    fn decodes_flat_color_exactly() {
        let hash = flat_hash([200, 40, 60]);
        let img = decode_blurhash(&hash, 8, 6).unwrap();
        assert_eq!((img.width, img.height), (8, 6));
        assert_eq!(img.pixels.len(), 8 * 6 * 4);
        for px in img.pixels.chunks_exact(4) {
            assert!(
                px[0].abs_diff(200) <= 1 && px[1].abs_diff(40) <= 1 && px[2].abs_diff(60) <= 1,
                "{px:?}"
            );
            assert_eq!(px[3], 255);
        }
    }

    #[test]
    fn rejects_bad_input() {
        assert!(decode_blurhash("zz", 8, 8).is_err());
        let hash = flat_hash([1, 2, 3]);
        assert!(decode_blurhash(&hash, 0, 8).is_err());
        assert!(decode_blurhash(&hash, 9999, 8).is_err());
    }

    #[test]
    fn sizes_follow_aspect() {
        assert_eq!(placeholder_size(2.0, 32), (32, 16));
        assert_eq!(placeholder_size(0.5, 32), (16, 32));
        assert_eq!(placeholder_size(f32::NAN, 32), (32, 32));
    }

    #[test]
    fn lookup_by_tag_with_fallback() {
        let mut hashes = HashMap::new();
        hashes.insert(
            "Primary".to_string(),
            HashMap::from([
                ("b".to_string(), "HB".to_string()),
                ("a".to_string(), "HA".to_string()),
            ]),
        );
        assert_eq!(
            blurhash_for(&hashes, ImageType::Primary, Some("b")),
            Some("HB")
        );
        assert_eq!(
            blurhash_for(&hashes, ImageType::Primary, Some("x")),
            Some("HA")
        );
        assert_eq!(blurhash_for(&hashes, ImageType::Primary, None), Some("HA"));
        assert_eq!(blurhash_for(&hashes, ImageType::Backdrop, None), None);
    }
}
