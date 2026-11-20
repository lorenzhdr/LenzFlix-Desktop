//! `WM_NCHITTEST`-Subclass: Rand-Resize, Ziehfläche (`HTCAPTION`) und Maximieren-Knopf
//! (`HTMAXBUTTON`, öffnet das Snap-Layouts-Flyout von Windows 11).
//!
//! Für `HTMAXBUTTON` liefert Windows Mausereignisse als Nicht-Client-Nachrichten; die Subclass
//! reicht sie als synthetische Zeigerereignisse an Slint weiter, damit der Knopf in der UI
//! gewohnt anspricht (Hover, Klick -> `clicked`).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT, TrackMouseEvent,
};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClientRect, IsZoomed, WM_NCHITTEST, WM_NCLBUTTONDBLCLK, WM_NCLBUTTONDOWN, WM_NCLBUTTONUP,
    WM_NCMOUSELEAVE, WM_NCMOUSEMOVE,
};

use crate::hit::{HitRegions, HitResult, hit_test};

const SUBCLASS_ID: usize = 0x4C46_5831; // "LFX1"
const HTMAXBUTTON: usize = 9;

/// Synthetisches Zeigerereignis für die UI (logische Pixel).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum NcPointer {
    Moved(f32, f32),
    Pressed(f32, f32),
    Released(f32, f32),
    Exited,
}

/// Empfänger der synthetischen Zeigerereignisse.
pub(crate) type PointerSink = Box<dyn Fn(NcPointer)>;

/// Zustand, den Subclass und Shell teilen (nur UI-Thread).
pub(crate) struct NcShared {
    pub regions: RefCell<HitRegions>,
    pub scale: Cell<f64>,
    pub pointer: RefCell<Option<PointerSink>>,
    tracking: Cell<bool>,
}

impl NcShared {
    pub fn new(scale: f64) -> Rc<Self> {
        Rc::new(Self {
            regions: RefCell::new(HitRegions::default()),
            scale: Cell::new(scale),
            pointer: RefCell::new(None),
            tracking: Cell::new(false),
        })
    }
}

/// Installierte Subclass; entfernt sich beim Fallen.
pub(crate) struct Subclass {
    hwnd: HWND,
    data: *mut Rc<NcShared>,
}

impl Subclass {
    pub fn install(hwnd: isize, shared: Rc<NcShared>) -> Option<Self> {
        let hwnd = HWND(hwnd as *mut std::ffi::c_void);
        let data = Box::into_raw(Box::new(shared));
        // SAFETY: `data` bleibt bis `Drop` gültig; die Subclass-Prozedur läuft im selben Thread.
        let ok = unsafe { SetWindowSubclass(hwnd, Some(proc), SUBCLASS_ID, data as usize) };
        if ok.as_bool() {
            Some(Self { hwnd, data })
        } else {
            // SAFETY: nie übergeben worden.
            unsafe { drop(Box::from_raw(data)) };
            None
        }
    }
}

impl Drop for Subclass {
    fn drop(&mut self) {
        // SAFETY: gleiche Kennung wie bei `install`; danach nutzt niemand mehr `data`.
        unsafe {
            let _ = RemoveWindowSubclass(self.hwnd, Some(proc), SUBCLASS_ID);
            drop(Box::from_raw(self.data));
        }
    }
}

fn point_from_lparam(l: LPARAM) -> POINT {
    POINT {
        x: (l.0 & 0xFFFF) as i16 as i32,
        y: ((l.0 >> 16) & 0xFFFF) as i16 as i32,
    }
}

/// Bildschirmpunkt (lParam) -> logische Client-Koordinaten.
fn to_logical(hwnd: HWND, l: LPARAM, scale: f64) -> (f32, f32) {
    let mut p = point_from_lparam(l);
    // SAFETY: gültiges Fenster, lokaler Punkt.
    unsafe {
        let _ = ScreenToClient(hwnd, &mut p);
    }
    ((p.x as f64 / scale) as f32, (p.y as f64 / scale) as f32)
}

unsafe extern "system" fn proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    data: usize,
) -> LRESULT {
    // SAFETY: `data` ist der in `install` übergebene Zeiger auf `Rc<NcShared>`.
    let shared: &Rc<NcShared> = unsafe { &*(data as *const Rc<NcShared>) };
    let scale = shared.scale.get().max(0.1);
    match msg {
        WM_NCHITTEST => {
            let (x, y) = to_logical(hwnd, lparam, scale);
            let mut rc = RECT::default();
            // SAFETY: gültiges Fenster, lokales Rechteck.
            let (maximized, ok) = unsafe {
                (
                    IsZoomed(hwnd).as_bool(),
                    GetClientRect(hwnd, &mut rc).is_ok(),
                )
            };
            if ok && let Ok(regions) = shared.regions.try_borrow() {
                let size = (
                    ((rc.right - rc.left) as f64 / scale) as f32,
                    ((rc.bottom - rc.top) as f64 / scale) as f32,
                );
                let r = hit_test(&regions, x, y, size, maximized);
                if r != HitResult::Client {
                    return LRESULT(r.to_win32());
                }
            }
            // SAFETY: Standardweiterleitung.
            unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
        }
        WM_NCMOUSEMOVE => {
            if wparam.0 == HTMAXBUTTON {
                let (x, y) = to_logical(hwnd, lparam, scale);
                if !shared.tracking.replace(true) {
                    let mut t = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE | TME_NONCLIENT,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    // SAFETY: gültige Struktur, eigenes Fenster.
                    unsafe {
                        let _ = TrackMouseEvent(&mut t);
                    }
                }
                emit(shared, NcPointer::Moved(x, y));
                return LRESULT(0);
            }
            if shared.tracking.replace(false) {
                emit(shared, NcPointer::Exited);
            }
            // SAFETY: Standardweiterleitung.
            unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
        }
        WM_NCMOUSELEAVE => {
            if shared.tracking.replace(false) {
                emit(shared, NcPointer::Exited);
            }
            // SAFETY: Standardweiterleitung.
            unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
        }
        WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK if wparam.0 == HTMAXBUTTON => {
            let (x, y) = to_logical(hwnd, lparam, scale);
            emit(shared, NcPointer::Pressed(x, y));
            // Nicht an DefWindowProc geben: es würde eine eigene Knopf-Schleife starten.
            LRESULT(0)
        }
        WM_NCLBUTTONUP if wparam.0 == HTMAXBUTTON => {
            let (x, y) = to_logical(hwnd, lparam, scale);
            emit(shared, NcPointer::Released(x, y));
            LRESULT(0)
        }
        // SAFETY: Standardweiterleitung.
        _ => unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) },
    }
}

fn emit(shared: &NcShared, ev: NcPointer) {
    if let Ok(cb) = shared.pointer.try_borrow()
        && let Some(f) = cb.as_ref()
    {
        f(ev);
    }
}

/// Fragt den Hit-Test direkt ab (Selbsttest der Demo über `SendMessage(WM_NCHITTEST)`).
pub(crate) fn hit_code_at_screen(hwnd: isize, sx: i32, sy: i32) -> isize {
    use windows::Win32::UI::WindowsAndMessaging::SendMessageW;
    let l = LPARAM(((sy as isize & 0xFFFF) << 16) | (sx as isize & 0xFFFF));
    // SAFETY: SendMessage an das eigene Fenster im selben Thread.
    unsafe {
        SendMessageW(
            HWND(hwnd as *mut std::ffi::c_void),
            WM_NCHITTEST,
            None,
            Some(l),
        )
        .0
    }
}
