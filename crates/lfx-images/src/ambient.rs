//! Ambient-Farbe eines Bildes. Port von `src/js/core/color.js` der Web-Vorlage
//! (gleiche Quantisierung, Gewichtung, Rundung und Gleichstand-Regel: der zuerst
//! gesehene Eimer gewinnt, wie die Einfügereihenfolge einer JS-`Map`).

use crate::types::DecodedImage;

/// Neutrale Standardfarbe, wenn ein Bild keine verwertbaren Pixel hat.
pub const DEFAULT_AMBIENT: [u8; 3] = [27, 40, 41];

/// Grenzen für den dunklen Ambient-Verlauf.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AmbientClamp {
    /// Kleinste Helligkeit (0..1).
    pub l_min: f64,
    /// Größte Helligkeit (0..1).
    pub l_max: f64,
    /// Größte Sättigung (0..1).
    pub s_max: f64,
}

impl Default for AmbientClamp {
    fn default() -> Self {
        Self {
            l_min: 0.18,
            l_max: 0.24,
            s_max: 0.55,
        }
    }
}

/// JS-`Math.round` für nichtnegative Werte (halbe Werte nach oben), begrenzt auf 0..=255.
fn js_round_u8(v: f64) -> u8 {
    (v + 0.5).floor().clamp(0.0, 255.0) as u8
}

/// RGB (0..255) → (Farbton 0..360, Sättigung 0..1, Helligkeit 0..1).
pub fn rgb_to_hsl(r: u8, g: u8, b: u8) -> (f64, f64, f64) {
    let (rf, gf, bf) = (
        f64::from(r) / 255.0,
        f64::from(g) / 255.0,
        f64::from(b) / 255.0,
    );
    let max = rf.max(gf).max(bf);
    let min = rf.min(gf).min(bf);
    let l = (max + min) / 2.0;
    let d = max - min;
    if d == 0.0 {
        return (0.0, 0.0, l);
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let mut h = if max == rf {
        ((gf - bf) / d) % 6.0
    } else if max == gf {
        (bf - rf) / d + 2.0
    } else {
        (rf - gf) / d + 4.0
    };
    h *= 60.0;
    if h < 0.0 {
        h += 360.0;
    }
    (h, s, l)
}

/// (Farbton, Sättigung, Helligkeit) → gerundetes RGB.
pub fn hsl_to_rgb(h: f64, s: f64, l: f64) -> [u8; 3] {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = (((h % 360.0) + 360.0) % 360.0) / 60.0;
    let x = c * (1.0 - ((hp % 2.0) - 1.0).abs());
    let (r, g, b) = if hp < 1.0 {
        (c, x, 0.0)
    } else if hp < 2.0 {
        (x, c, 0.0)
    } else if hp < 3.0 {
        (0.0, c, x)
    } else if hp < 4.0 {
        (0.0, x, c)
    } else if hp < 5.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };
    let m = l - c / 2.0;
    [
        js_round_u8((r + m) * 255.0),
        js_round_u8((g + m) * 255.0),
        js_round_u8((b + m) * 255.0),
    ]
}

struct Bucket {
    w: f64,
    r: f64,
    g: f64,
    b: f64,
}

/// Dominante, gesättigte Farbe aus geraden RGBA8-Pixeln.
///
/// Quantisierung 4 Bit je Kanal, Gewicht = Häufigkeit × Sättigung; sehr dunkle, sehr
/// helle und halbdurchsichtige Pixel zählen nicht. `None`, wenn nichts übrig bleibt.
pub fn dominant_color(rgba: &[u8]) -> Option<[u8; 3]> {
    // Eimer in Einfügereihenfolge (wie die JS-Map), Index über die 4096 Schlüssel.
    let mut slot = vec![u16::MAX; 4096];
    let mut buckets: Vec<Bucket> = Vec::new();
    for px in rgba.as_chunks::<4>().0.iter() {
        let (r, g, b, a) = (px[0], px[1], px[2], px[3]);
        if a < 128 {
            continue;
        }
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let l = (f64::from(max) + f64::from(min)) / 510.0;
        if !(0.08..=0.92).contains(&l) {
            continue;
        }
        let d = f64::from(max - min) / 255.0;
        let s = if d == 0.0 {
            0.0
        } else {
            d / (1.0 - (2.0 * l - 1.0).abs())
        };
        let w = s + 0.05;
        let key = (usize::from(r >> 4) << 8) | (usize::from(g >> 4) << 4) | usize::from(b >> 4);
        if slot[key] == u16::MAX {
            slot[key] = buckets.len() as u16;
            buckets.push(Bucket {
                w: 0.0,
                r: 0.0,
                g: 0.0,
                b: 0.0,
            });
        }
        let e = &mut buckets[usize::from(slot[key])];
        e.w += w;
        e.r += f64::from(r) * w;
        e.g += f64::from(g) * w;
        e.b += f64::from(b) * w;
    }
    let mut best: Option<&Bucket> = None;
    for e in &buckets {
        if best.is_none_or(|b| e.w > b.w) {
            best = Some(e);
        }
    }
    best.map(|b| {
        [
            js_round_u8(b.r / b.w),
            js_round_u8(b.g / b.w),
            js_round_u8(b.b / b.w),
        ]
    })
}

/// Klemmt eine Farbe für den dunklen Verlauf (Helligkeit und Sättigung begrenzt).
pub fn clamp_ambient(rgb: [u8; 3], clamp: AmbientClamp) -> [u8; 3] {
    let (h, s, l) = rgb_to_hsl(rgb[0], rgb[1], rgb[2]);
    hsl_to_rgb(h, s.min(clamp.s_max), clamp.l_max.min(clamp.l_min.max(l)))
}

/// Pixel → geklemmte Ambient-Farbe (Rückfall [`DEFAULT_AMBIENT`]).
pub fn ambient_from_pixels(rgba: &[u8]) -> [u8; 3] {
    match dominant_color(rgba) {
        Some(dom) => clamp_ambient(dom, AmbientClamp::default()),
        None => DEFAULT_AMBIENT,
    }
}

/// Ambient-Farbe eines dekodierten Bildes (vormultiplizierte Bilder werden zuvor
/// zurückgerechnet).
pub fn ambient_from_image(img: &DecodedImage) -> [u8; 3] {
    if !img.premultiplied {
        return ambient_from_pixels(&img.pixels);
    }
    let mut straight = img.pixels.to_vec();
    for px in straight.as_chunks_mut::<4>().0.iter_mut() {
        let a = u32::from(px[3]);
        if a != 0 && a != 255 {
            for c in &mut px[..3] {
                *c = ((u32::from(*c) * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    ambient_from_pixels(&straight)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use std::sync::Arc;

    #[derive(Deserialize)]
    struct Case {
        name: String,
        rgba: String,
        dominant: Option<[u8; 3]>,
        ambient: [u8; 3],
        clamped: Option<[u8; 3]>,
    }

    #[derive(Deserialize)]
    struct HslCase {
        rgb: [u8; 3],
        hsl: (f64, f64, f64),
        clamped: [u8; 3],
    }

    #[derive(Deserialize)]
    struct Vectors {
        default: [u8; 3],
        cases: Vec<Case>,
        hsl: Vec<HslCase>,
    }

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    fn vectors() -> Vectors {
        serde_json::from_str(include_str!("../tests/data/ambient_vectors.json")).unwrap()
    }

    #[test]
    fn matches_js_vectors() {
        let v = vectors();
        assert_eq!(v.default, DEFAULT_AMBIENT);
        assert!(v.cases.len() >= 15);
        for c in &v.cases {
            let px = unhex(&c.rgba);
            assert_eq!(dominant_color(&px), c.dominant, "dominant {}", c.name);
            assert_eq!(ambient_from_pixels(&px), c.ambient, "ambient {}", c.name);
            if let Some(dom) = c.dominant {
                assert_eq!(
                    Some(clamp_ambient(dom, AmbientClamp::default())),
                    c.clamped,
                    "clamp {}",
                    c.name
                );
            }
        }
    }

    #[test]
    fn hsl_matches_js_vectors() {
        for c in &vectors().hsl {
            assert_eq!(
                rgb_to_hsl(c.rgb[0], c.rgb[1], c.rgb[2]),
                c.hsl,
                "hsl {:?}",
                c.rgb
            );
            assert_eq!(
                clamp_ambient(c.rgb, AmbientClamp::default()),
                c.clamped,
                "clamp {:?}",
                c.rgb
            );
        }
    }

    #[test]
    fn premultiplied_image_gives_same_color() {
        let straight: Vec<u8> = [200u8, 40, 40, 255].repeat(16);
        let img = DecodedImage {
            width: 4,
            height: 4,
            pixels: Arc::from(straight.clone()),
            premultiplied: false,
        };
        assert_eq!(ambient_from_image(&img), ambient_from_pixels(&straight));
        let pre = DecodedImage {
            premultiplied: true,
            ..img
        };
        assert_eq!(ambient_from_image(&pre), ambient_from_pixels(&straight));
    }
}
