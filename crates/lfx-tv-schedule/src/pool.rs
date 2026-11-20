//! Pools und Eimer-Rotation (rein, deterministisch).
//!
//! * `stream_of_age`: Strom A (Freigabe höchstens 12 oder unbekannt, 05 bis 22 Uhr) oder B (ab 16).
//! * `bucket_count`: größte Zweierpotenz C mit C * capMs <= poolMs (1 bis 4096).
//! * `make_entries`: Einträge nach `bitrev32(h)` sortiert; ein Eimer ist ein zusammenhängender Bereich.
//! * `bucket` / `rotation` / `rotation_with_borrow`: Tages-Eimer `D & (C-1)`, je Zyklus rotiert.

use std::cmp::Ordering;

use crate::hash::{bitrev32, cmp_str, key_hash};
use crate::rating::Age;
use crate::time::HOUR;

/// Kapazität je Sendetag in ms: Strom A 17 h, Strom B 4 h.
pub const CAP_A: i64 = 17 * HOUR;
pub const CAP_B: i64 = 4 * HOUR;
const MAX_C: i64 = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    A,
    B,
}

/// Kapazität eines Stroms.
pub fn cap_of(s: Stream) -> i64 {
    match s {
        Stream::A => CAP_A,
        Stream::B => CAP_B,
    }
}

/// `streamOfAge`.
pub fn stream_of_age(age: Age) -> Stream {
    match age {
        None => Stream::A,
        Some(a) if a <= 12 => Stream::A,
        _ => Stream::B,
    }
}

/// Größte Zweierpotenz C <= poolMs/capMs (mindestens 1, höchstens 4096), nur Ganzzahl-Arithmetik.
pub fn bucket_count(pool_ms: i64, cap_ms: i64) -> i64 {
    let mut c = 1i64;
    while c < MAX_C && (c as i128) * 2 * (cap_ms as i128) <= pool_ms as i128 {
        c *= 2;
    }
    c
}

fn log2i(c: i64) -> u32 {
    31 - (c as u32).leading_zeros()
}

/// Eintrag eines Pools.
#[derive(Debug, Clone)]
pub struct Entry<T> {
    pub id: String,
    pub h: u32,
    pub br: u32,
    pub it: T,
    pub ms: i64,
}

/// Index-Einträge, sortiert nach `bitrev32(h)` (Gleichstand: Id in Code-Unit-Reihenfolge).
pub fn make_entries<T: Clone>(
    key: &str,
    items: &[T],
    id_of: impl Fn(&T) -> String,
    ms_of: impl Fn(&T) -> i64,
) -> Vec<Entry<T>> {
    let mut out: Vec<Entry<T>> = items
        .iter()
        .map(|it| {
            let id = id_of(it);
            let h = key_hash(key, &id);
            Entry {
                br: bitrev32(h),
                h,
                ms: ms_of(it),
                it: it.clone(),
                id,
            }
        })
        .collect();
    out.sort_by(|a, b| a.br.cmp(&b.br).then_with(|| cmp_str(&a.id, &b.id)));
    out
}

fn lower_bound<T>(arr: &[Entry<T>], br: u64) -> usize {
    let (mut lo, mut hi) = (0usize, arr.len());
    while lo < hi {
        let m = (lo + hi) >> 1;
        if u64::from(arr[m].br) < br {
            lo = m + 1;
        } else {
            hi = m;
        }
    }
    lo
}

/// Titel des Tag-Eimers `D & (C-1)`, nach (h, id) sortiert.
pub fn bucket<T: Clone>(sorted: &[Entry<T>], c: i64, d: i64) -> Vec<Entry<T>> {
    let cb = log2i(c);
    let b = (d as i32 as u32) & ((c - 1) as u32);
    let (mut lo, mut hi) = (0usize, sorted.len());
    if cb > 0 {
        let r = u64::from(bitrev32(b) >> (32 - cb));
        let span = 1u64 << (32 - cb);
        lo = lower_bound(sorted, r * span);
        hi = lower_bound(sorted, (r + 1) * span);
    }
    let mut out: Vec<Entry<T>> = sorted[lo..hi].to_vec();
    out.sort_by(|a, x| -> Ordering { a.h.cmp(&x.h).then_with(|| cmp_str(&a.id, &x.id)) });
    out
}

/// Rotierte Liste des Eimers von Tag D (Zyklus `q = D >>> c`, Start beim ersten Titel mit
/// `h >= imul(q, 0x9E3779B9)`).
pub fn rotation<T: Clone>(sorted: &[Entry<T>], c: i64, d: i64) -> Vec<Entry<T>> {
    let list = bucket(sorted, c, d);
    if list.len() < 2 {
        return list;
    }
    let q = (d as u32) >> log2i(c);
    let s = q.wrapping_mul(0x9e37_79b9);
    let mut start = 0;
    while start < list.len() && list[start].h < s {
        start += 1;
    }
    if start == list.len() {
        start = 0;
    }
    if start == 0 {
        list
    } else {
        let mut out = list[start..].to_vec();
        out.extend_from_slice(&list[..start]);
        out
    }
}

/// Eigener Eimer plus geliehener Eimer `D + C/2` (leer bei C = 1).
pub fn rotation_with_borrow<T: Clone>(
    sorted: &[Entry<T>],
    c: i64,
    d: i64,
) -> (Vec<Entry<T>>, Vec<Entry<T>>) {
    let home = rotation(sorted, c, d);
    let borrow = if c > 1 {
        rotation(sorted, c, d + c / 2)
    } else {
        Vec::new()
    };
    (home, borrow)
}
