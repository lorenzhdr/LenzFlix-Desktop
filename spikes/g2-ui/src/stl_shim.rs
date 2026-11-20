//! Nur Spike (Feature `skia`): Die vorgebauten Skia-Bibliotheken wurden mit einer neueren MSVC-STL
//! gebaut als die installierten VS-2022-Build-Tools (14.44) liefern. Die zwölf fehlenden
//! Vektor-Algorithmus-Symbole werden hier skalar nachgebaut, damit der Spike linkt. Kein Produktionscode:
//! die richtige Lösung ist ein neueres MSVC-Toolset (siehe Bericht).
#![allow(clippy::missing_safety_doc)]

use std::ffi::c_void;

unsafe fn scan<T: Copy + PartialEq>(
    first: *const c_void,
    last: *const c_void,
    v: T,
) -> *const c_void {
    let mut p = first as *const T;
    let end = last as *const T;
    while p != end {
        if unsafe { p.read_unaligned() } == v {
            return p as *const c_void;
        }
        p = unsafe { p.add(1) };
    }
    last
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __std_find_trivial_1(
    first: *const c_void,
    last: *const c_void,
    v: u8,
) -> *const c_void {
    unsafe { scan(first, last, v) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __std_find_trivial_4(
    first: *const c_void,
    last: *const c_void,
    v: u32,
) -> *const c_void {
    unsafe { scan(first, last, v) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __std_find_trivial_8(
    first: *const c_void,
    last: *const c_void,
    v: u64,
) -> *const c_void {
    unsafe { scan(first, last, v) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __std_find_last_trivial_1(
    first: *const c_void,
    last: *const c_void,
    v: u8,
) -> *const c_void {
    let mut p = last as *const u8;
    let begin = first as *const u8;
    while p != begin {
        p = unsafe { p.sub(1) };
        if unsafe { *p } == v {
            return p as *const c_void;
        }
    }
    last
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __std_find_first_of_trivial_pos_1(
    first1: *const c_void,
    last1: *const c_void,
    first2: *const c_void,
    last2: *const c_void,
) -> usize {
    let (a, b) = (first1 as *const u8, last1 as *const u8);
    let n = unsafe { b.offset_from(a) } as usize;
    let m = unsafe { (last2 as *const u8).offset_from(first2 as *const u8) } as usize;
    for i in 0..n {
        let x = unsafe { *a.add(i) };
        for j in 0..m {
            if unsafe { *(first2 as *const u8).add(j) } == x {
                return i;
            }
        }
    }
    n
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __std_search_1(
    first1: *const c_void,
    last1: *const c_void,
    first2: *const c_void,
    count2: usize,
) -> *const c_void {
    let (a, b) = (first1 as *const u8, last1 as *const u8);
    let n = unsafe { b.offset_from(a) } as usize;
    if count2 == 0 {
        return first1;
    }
    if count2 > n {
        return last1;
    }
    let needle = unsafe { std::slice::from_raw_parts(first2 as *const u8, count2) };
    let hay = unsafe { std::slice::from_raw_parts(a, n) };
    match hay.windows(count2).position(|w| w == needle) {
        Some(i) => unsafe { a.add(i) as *const c_void },
        None => last1,
    }
}

unsafe fn extreme<T: Copy + PartialOrd>(
    first: *const c_void,
    last: *const c_void,
    want_max: bool,
) -> *const c_void {
    let mut p = first as *const T;
    let end = last as *const T;
    if p == end {
        return last;
    }
    let mut best = p;
    p = unsafe { p.add(1) };
    while p != end {
        let (v, b) = unsafe { (p.read_unaligned(), best.read_unaligned()) };
        if (want_max && v > b) || (!want_max && v < b) {
            best = p;
        }
        p = unsafe { p.add(1) };
    }
    best as *const c_void
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __std_max_element_1u(
    first: *const c_void,
    last: *const c_void,
) -> *const c_void {
    unsafe { extreme::<u8>(first, last, true) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __std_max_element_2u(
    first: *const c_void,
    last: *const c_void,
) -> *const c_void {
    unsafe { extreme::<u16>(first, last, true) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __std_max_element_f_(
    first: *const c_void,
    last: *const c_void,
) -> *const c_void {
    unsafe { extreme::<f32>(first, last, true) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __std_min_element_f_(
    first: *const c_void,
    last: *const c_void,
) -> *const c_void {
    unsafe { extreme::<f32>(first, last, false) }
}

#[repr(C)]
pub struct MinMax {
    min: *const c_void,
    max: *const c_void,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __std_minmax_element_f_(
    first: *const c_void,
    last: *const c_void,
) -> MinMax {
    let min = unsafe { extreme::<f32>(first, last, false) };
    // std::minmax_element: Minimum = erstes kleinstes, Maximum = LETZTES größtes.
    let mut max = first;
    let mut p = first as *const f32;
    let end = last as *const f32;
    while p != end {
        if unsafe { p.read_unaligned() >= (max as *const f32).read_unaligned() } {
            max = p as *const c_void;
        }
        p = unsafe { p.add(1) };
    }
    MinMax { min, max }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __std_reverse_trivially_swappable_8(
    first: *mut c_void,
    last: *mut c_void,
) {
    let mut a = first as *mut u64;
    let mut b = last as *mut u64;
    while a != b {
        b = unsafe { b.sub(1) };
        if a == b {
            break;
        }
        unsafe {
            let (x, y) = (a.read_unaligned(), b.read_unaligned());
            a.write_unaligned(y);
            b.write_unaligned(x);
            a = a.add(1);
        }
    }
}
