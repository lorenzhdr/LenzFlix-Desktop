//! Ambient-Farbe aus dem Billboard-Bild (Port von `core/color.js`, reine Funktionen).
//!
//! Dominante, gesättigte Farbe aus RGBA-Pixeln (4 Bit je Kanal quantisiert, Gewicht = Häufigkeit mal
//! Sättigung), danach für den dunklen Verlauf geklemmt (Helligkeit 0,18 bis 0,24, Sättigung höchstens 0,55).

use std::collections::BTreeMap;

/// Neutrale Standardfarbe (Dunkelgrau-Teal).
pub const DEFAULT_AMBIENT: [u8; 3] = [27, 40, 41];

fn rgb_to_hsl(r: u8, g: u8, b: u8) -> (f64, f64, f64) {
    let (r, g, b) = (
        f64::from(r) / 255.0,
        f64::from(g) / 255.0,
        f64::from(b) / 255.0,
    );
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;
    if d == 0.0 {
        return (0.0, 0.0, l);
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let mut h = if max == r {
        ((g - b) / d) % 6.0
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    h *= 60.0;
    if h < 0.0 {
        h += 360.0;
    }
    (h, s, l)
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> [u8; 3] {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h.rem_euclid(360.0) / 60.0;
    let x = c * (1.0 - ((hp % 2.0) - 1.0).abs());
    let (r, g, b) = match hp {
        v if v < 1.0 => (c, x, 0.0),
        v if v < 2.0 => (x, c, 0.0),
        v if v < 3.0 => (0.0, c, x),
        v if v < 4.0 => (0.0, x, c),
        v if v < 5.0 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    [r, g, b].map(|v| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8)
}

#[derive(Default)]
struct Bucket {
    w: f64,
    r: f64,
    g: f64,
    b: f64,
}

/// Dominante, gesättigte Farbe; `None`, wenn keine verwertbaren Pixel da sind.
pub fn dominant_color(rgba: &[u8]) -> Option<[u8; 3]> {
    let mut buckets: BTreeMap<u16, Bucket> = BTreeMap::new();
    for px in rgba.as_chunks::<4>().0 {
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
        let key = (u16::from(r >> 4) << 8) | (u16::from(g >> 4) << 4) | u16::from(b >> 4);
        let e = buckets.entry(key).or_default();
        e.w += w;
        e.r += f64::from(r) * w;
        e.g += f64::from(g) * w;
        e.b += f64::from(b) * w;
    }
    // Das Web nimmt bei Gleichstand die Einfügereihenfolge; hier die Schlüsselreihenfolge (praktisch gleich).
    let best = buckets.values().fold(None::<&Bucket>, |acc, e| match acc {
        Some(a) if a.w >= e.w => Some(a),
        _ => Some(e),
    })?;
    Some([
        (best.r / best.w).round() as u8,
        (best.g / best.w).round() as u8,
        (best.b / best.w).round() as u8,
    ])
}

/// Klemmt eine Farbe für den dunklen Ambient-Verlauf.
pub fn clamp_ambient([r, g, b]: [u8; 3]) -> [u8; 3] {
    let (h, s, l) = rgb_to_hsl(r, g, b);
    hsl_to_rgb(h, s.min(0.55), l.clamp(0.18, 0.24))
}

/// Pixel zu geklemmter Ambient-Farbe (Rückfall: [`DEFAULT_AMBIENT`]).
pub fn ambient_from_pixels(rgba: &[u8]) -> [u8; 3] {
    dominant_color(rgba).map_or(DEFAULT_AMBIENT, clamp_ambient)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_and_transparent_pixels_do_not_count() {
        let px = [0, 0, 0, 255, 255, 255, 255, 255, 10, 20, 30, 0];
        assert_eq!(dominant_color(&px), None);
        assert_eq!(ambient_from_pixels(&px), DEFAULT_AMBIENT);
    }

    #[test]
    fn saturated_color_wins_and_is_clamped_dark() {
        let mut px = Vec::new();
        for _ in 0..10 {
            px.extend_from_slice(&[200, 40, 40, 255]);
        }
        for _ in 0..30 {
            px.extend_from_slice(&[128, 128, 128, 255]);
        }
        let dom = dominant_color(&px).unwrap();
        assert!(
            dom[0] > 150 && dom[1] < 80,
            "gesättigte Farbe gewinnt: {dom:?}"
        );
        let c = clamp_ambient(dom);
        let (_, s, l) = rgb_to_hsl(c[0], c[1], c[2]);
        assert!((0.17..=0.25).contains(&l), "Helligkeit {l}");
        assert!(s <= 0.56, "Sättigung {s}");
    }
}
