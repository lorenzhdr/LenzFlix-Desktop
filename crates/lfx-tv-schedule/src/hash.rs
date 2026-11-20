//! Hash und PRNG (32 Bit, deterministisch). Zeichenketten zählen als UTF-16-Code-Units wie in JS.

use std::cmp::Ordering;

/// FNV-1a über UTF-16-Code-Units.
pub fn fnv1a_units(units: &[u16]) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for &c in units {
        h ^= u32::from(c);
        h = h.wrapping_mul(16_777_619);
    }
    h
}

/// FNV-1a über die UTF-16-Code-Units der Zeichenkette.
pub fn fnv1a(s: &str) -> u32 {
    let u: Vec<u16> = s.encode_utf16().collect();
    fnv1a_units(&u)
}

/// Murmur3-Finalizer.
pub fn fmix32(mut h: u32) -> u32 {
    h ^= h >> 16;
    h = h.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0xc2b2_ae35);
    h ^= h >> 16;
    h
}

/// Bits von 32 bis 0 spiegeln.
pub fn bitrev32(x: u32) -> u32 {
    x.reverse_bits()
}

/// `x >>> 0` für beliebige JS-Zahlen (ToUint32).
pub fn to_uint32(x: f64) -> u32 {
    if !x.is_finite() {
        return 0;
    }
    let m = 4_294_967_296.0;
    (((x.trunc() % m) + m) % m) as u32
}

/// Mulberry32-PRNG.
#[derive(Debug, Clone)]
pub struct Mulberry32 {
    a: u32,
}

/// `mulberry32(seed)`; der Seed wird wie `seed >>> 0` gelesen.
pub fn mulberry32(seed: f64) -> Mulberry32 {
    Mulberry32 { a: to_uint32(seed) }
}

impl Mulberry32 {
    /// Nächster Wert in [0, 1).
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> f64 {
        self.a = self.a.wrapping_add(0x6d2b_79f5);
        let a = self.a;
        let mut t = (a ^ (a >> 15)).wrapping_mul(1 | a);
        t = t.wrapping_add((t ^ (t >> 7)).wrapping_mul(61 | t)) ^ t;
        f64::from(t ^ (t >> 14)) / 4_294_967_296.0
    }
}

/// `fmix32(fnv1a(key + '\u0001' + id))` über Code-Units.
pub fn key_hash_units(key: &[u16], id: &[u16]) -> u32 {
    let mut v: Vec<u16> = Vec::with_capacity(key.len() + id.len() + 1);
    v.extend_from_slice(key);
    v.push(1);
    v.extend_from_slice(id);
    fmix32(fnv1a_units(&v))
}

/// `fmix32(fnv1a(key + '\u0001' + id))`.
pub fn key_hash(key: &str, id: &str) -> u32 {
    let mut s = String::with_capacity(key.len() + id.len() + 1);
    s.push_str(key);
    s.push('\u{1}');
    s.push_str(id);
    fmix32(fnv1a(&s))
}

/// Vergleich in Code-Unit-Reihenfolge (nicht UTF-8/Codepoint!).
pub fn cmp_units(a: &[u16], b: &[u16]) -> Ordering {
    a.cmp(b)
}

/// `cmpStr`: Vergleich in Code-Unit-Reihenfolge.
pub fn cmp_str(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}
