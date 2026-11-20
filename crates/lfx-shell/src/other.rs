//! Nicht-Windows-Variante (macOS, CI): normales Slint-winit-Backend, kein Video-Layer.
//!
//! Hier wird nur kompiliert und geprüft, ob der Aufbau trägt; die Laufzeit auf macOS ist
//! unverifiziert (kein Mac am Entwicklungsrechner, nur CI).

use std::cell::RefCell;
use std::rc::Rc;

use slint::ComponentHandle;
use slint::winit_030::{WinitWindowAccessor, winit};

use crate::{
    FrameSummary, FrameTimes, HdrMetadata, HitRegions, OutputHdrInfo, Rect, ShellError,
    ShellOptions, UiBackend, VideoFormat, VideoTarget,
};

/// Video-Layer gibt es nur unter Windows. Der Typ wird hier nie erzeugt (`video_layer()` liefert
/// `None`); er ist bewusst eine Struktur mit privatem Feld statt eines leeren Enums, damit Aufrufer-Code
/// nach `video_layer()?` nicht als unerreichbar gilt (Lint `unreachable_code` unter macOS).
#[derive(Clone, Debug)]
pub struct VideoLayer {
    _never: (),
}

impl VideoLayer {
    pub fn d3d11_device_raw(&self) -> *mut std::ffi::c_void {
        std::ptr::null_mut()
    }
    pub fn acquire_target(&self) -> Option<VideoTarget> {
        None
    }
    pub fn present(&self) {}
    pub fn set_rect(&self, _rect: Option<Rect>) {}
    pub fn set_format(&self, _format: VideoFormat) {}
    pub fn set_hdr_metadata(&self, _md: HdrMetadata) {}
    pub fn output_hdr_info(&self) -> OutputHdrInfo {
        OutputHdrInfo::default()
    }
    pub fn set_visible(&self, _visible: bool) {}
    pub fn take_dirty(&self) -> bool {
        false
    }
}

/// Führt eine Funktion mit dem winit-Fenster aus (hält nur eine schwache Verbindung zur UI).
type WinRunner = Box<dyn Fn(&mut dyn FnMut(&winit::window::Window))>;

struct Inner {
    window: RefCell<Option<WinRunner>>,
    regions: RefCell<HitRegions>,
    frames: RefCell<Option<FrameTimes>>,
}

/// Besitzer der Fensterumgebung.
pub struct Shell {
    handle: ShellHandle,
}

/// Griff für Fenstersteuerung aus UI-Callbacks (nur im UI-Thread).
#[derive(Clone)]
pub struct ShellHandle {
    inner: Rc<Inner>,
}

impl Shell {
    pub fn new(opts: ShellOptions) -> Result<Self, ShellError> {
        let (w, h) = opts.size;
        let (mw, mh) = opts.min_size;
        let title = opts.title.clone();
        slint::BackendSelector::new()
            .backend_name("winit".into())
            .with_winit_window_attributes_hook(move |attrs| {
                attrs
                    .with_decorations(false)
                    .with_title(title.clone())
                    .with_inner_size(winit::dpi::LogicalSize::new(w as f64, h as f64))
                    .with_min_inner_size(winit::dpi::LogicalSize::new(mw as f64, mh as f64))
                    .with_maximized(opts.maximized)
            })
            .select()
            .map_err(|e| ShellError::Platform(e.to_string()))?;
        Ok(Self {
            handle: ShellHandle {
                inner: Rc::new(Inner {
                    window: RefCell::new(None),
                    regions: RefCell::new(HitRegions::default()),
                    frames: RefCell::new(None),
                }),
            },
        })
    }

    pub fn handle(&self) -> ShellHandle {
        self.handle.clone()
    }

    /// Verbindet die Shell mit dem Hauptfenster der UI (nach `Component::new()` aufrufen).
    pub fn attach<C: ComponentHandle + 'static>(&self, component: &C) {
        let weak = component.as_weak();
        *self.handle.inner.window.borrow_mut() = Some(Box::new(move |f| {
            if let Some(c) = weak.upgrade() {
                c.window().with_winit_window(|w| f(w));
            }
        }));
    }

    /// Bequemer Einstieg: zeigt die Komponente und läuft bis zum Schließen.
    pub fn show_and_run<C: ComponentHandle + 'static>(
        &self,
        component: &C,
    ) -> Result<(), ShellError> {
        self.attach(component);
        component
            .show()
            .map_err(|e| ShellError::Window(e.to_string()))?;
        self.run()
    }

    pub fn run(&self) -> Result<(), ShellError> {
        slint::run_event_loop().map_err(|e| ShellError::EventLoop(e.to_string()))
    }
}

impl ShellHandle {
    fn with_window(&self, mut f: impl FnMut(&winit::window::Window)) {
        if let Some(run) = self.inner.window.borrow().as_ref() {
            run(&mut f);
        }
    }

    pub fn drag_window(&self) {
        self.with_window(|w| {
            let _ = w.drag_window();
        });
    }

    pub fn minimize(&self) {
        self.with_window(|w| w.set_minimized(true));
    }

    pub fn is_maximized(&self) -> bool {
        let mut r = false;
        self.with_window(|w| r = w.is_maximized());
        r
    }

    pub fn toggle_maximize(&self) {
        self.with_window(|w| w.set_maximized(!w.is_maximized()));
    }

    pub fn close(&self) {
        let _ = slint::quit_event_loop();
    }

    /// Wird hier nur gespeichert (kein Snap-Flyout außerhalb von Windows).
    pub fn set_hit_regions(&self, regions: HitRegions) {
        *self.inner.regions.borrow_mut() = regions;
    }

    pub fn video_layer(&self) -> Option<VideoLayer> {
        None
    }

    pub fn ui_backend(&self) -> UiBackend {
        UiBackend::Native
    }

    /// Bildzeiten werden außerhalb von Windows nicht erfasst (leere Statistik).
    pub fn start_frame_recording(&self, skip: usize) {
        *self.inner.frames.borrow_mut() = Some(FrameTimes::new(skip));
    }

    pub fn stop_frame_recording(&self) -> FrameSummary {
        self.inner
            .frames
            .borrow_mut()
            .take()
            .unwrap_or_default()
            .summary()
    }

    /// Physische Größe und Skalierung (soweit das winit-Fenster schon existiert).
    pub fn window_info(&self) -> ((u32, u32), f64) {
        let mut r = ((0, 0), 1.0);
        self.with_window(|w| {
            let s = w.inner_size();
            r = ((s.width, s.height), w.scale_factor());
        });
        r
    }

    pub fn refresh_rate_mhz(&self) -> Option<u32> {
        let mut r = None;
        self.with_window(|w| {
            r = w
                .current_monitor()
                .and_then(|m| m.refresh_rate_millihertz())
        });
        r
    }

    /// Nur unter Windows sinnvoll; hier immer `HTCLIENT`.
    pub fn probe_hit_code(&self, _x: f32, _y: f32) -> isize {
        1
    }

    pub fn ui_backend_note(&self) -> &str {
        "Standard-Slint-Backend"
    }
}
