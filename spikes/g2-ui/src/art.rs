//! Synthetische Cover: Farbverlauf plus Text "Film 0001" (5x7-Bitmapschrift, nur eigener Code).

use slint::{Rgba8Pixel, SharedPixelBuffer};

const GLYPH_H: usize = 7;

fn glyph(c: char) -> [u8; GLYPH_H] {
    match c {
        'F' => [
            0b11111, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000, 0b10000,
        ],
        'i' => [
            0b00100, 0b00000, 0b01100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
        'l' => [
            0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
        'm' => [
            0b00000, 0b00000, 0b11010, 0b10101, 0b10101, 0b10101, 0b10101,
        ],
        '0' => [
            0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110,
        ],
        '1' => [
            0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
        '2' => [
            0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111,
        ],
        '3' => [
            0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        '4' => [
            0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010,
        ],
        '5' => [
            0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110,
        ],
        '6' => [
            0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110,
        ],
        '7' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000,
        ],
        '8' => [
            0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110,
        ],
        '9' => [
            0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100,
        ],
        _ => [0; GLYPH_H],
    }
}

fn hash(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^ (x >> 16)
}

fn hsv(h: f32, s: f32, v: f32) -> [f32; 3] {
    let k = |n: f32| (n + h * 6.0) % 6.0;
    let f = |n: f32| v - v * s * k(n).min(4.0 - k(n)).clamp(0.0, 1.0);
    [f(5.0), f(3.0), f(1.0)]
}

/// Erzeugt das Cover Nr. `number` (1-basiert); `wide` = 480x270, sonst 300x450 (DPR-2-Größen).
pub fn render(number: u32, wide: bool) -> SharedPixelBuffer<Rgba8Pixel> {
    let (w, h) = if wide {
        (480u32, 270u32)
    } else {
        (300u32, 450u32)
    };
    let mut buf = SharedPixelBuffer::<Rgba8Pixel>::new(w, h);
    let hue = (hash(number) % 360) as f32 / 360.0;
    let top = hsv(hue, 0.65, 0.85);
    let bottom = hsv((hue + 0.12) % 1.0, 0.8, 0.25);
    let pixels = buf.make_mut_slice();
    for y in 0..h as usize {
        let t = y as f32 / (h - 1) as f32;
        let row: [u8; 3] =
            std::array::from_fn(|i| ((top[i] * (1.0 - t) + bottom[i] * t) * 255.0) as u8);
        for x in 0..w as usize {
            // leichte diagonale Streifen, damit die Textur nicht trivial komprimierbar ist
            let s = (((x + y) / 24) % 2) as u8 * 10;
            pixels[y * w as usize + x] = Rgba8Pixel {
                r: row[0].saturating_add(s),
                g: row[1].saturating_add(s),
                b: row[2].saturating_add(s),
                a: 255,
            };
        }
    }
    let text = format!("Film {number:04}");
    let scale = if wide { 7usize } else { 4usize };
    let chars = text.chars().count();
    let tw = chars * 6 * scale - scale;
    let x0 = (w as usize).saturating_sub(tw) / 2;
    let y0 = (h as usize - GLYPH_H * scale) / 2;
    for (ci, c) in text.chars().enumerate() {
        let g = glyph(c);
        for (gy, bits) in g.iter().enumerate() {
            for gx in 0..5 {
                if bits & (1 << (4 - gx)) != 0 {
                    for dy in 0..scale {
                        for dx in 0..scale {
                            let px = x0 + (ci * 6 + gx) * scale + dx;
                            let py = y0 + gy * scale + dy;
                            pixels[py * w as usize + px] = Rgba8Pixel {
                                r: 255,
                                g: 255,
                                b: 255,
                                a: 255,
                            };
                        }
                    }
                }
            }
        }
    }
    buf
}
