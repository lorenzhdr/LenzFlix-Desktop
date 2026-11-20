//! Windows-Shell (Modell A): winit-Fenster, DirectComposition mit Video-Layer und UI-Layer,
//! eigene Slint-Plattform.

mod compose;
mod gpu;
mod input;
mod soft;
mod subclass;

use std::cell::{Cell, RefCell};
use std::pin::Pin;
use std::rc::{Rc, Weak};
use std::time::Instant;

use i_slint_core::InternalToken;
use i_slint_core::cursor::{MouseCursorHandling, MouseCursorInner};
use i_slint_core::item_tree::ItemTreeRef;
use i_slint_core::items::ItemRef;
use i_slint_core::renderer::RendererSealed;
use i_slint_core::window::{InputMethodRequest, WindowAdapterInternal};
use slint::platform::femtovg_renderer::FemtoVGWGPURenderer;
use slint::platform::software_renderer::SoftwareRenderer;
use slint::platform::{
    EventLoopProxy, Platform, PlatformError, PointerEventButton, Renderer, WindowAdapter,
    WindowEvent, WindowProperties,
};
use slint::{
    EventLoopError, LogicalPosition, LogicalSize, PhysicalPosition, PhysicalSize, WindowPosition,
    WindowSize,
};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, Ime, WindowEvent as WEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::platform::windows::WindowAttributesExtWindows;
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::{Window, WindowId};

pub use compose::VideoLayer;
use compose::{Compositor, create_d3d11};
use gpu::{DrawResult, GpuCore, GpuUi};
use soft::SoftUi;
use subclass::{NcPointer, NcShared, Subclass};

use crate::{
    FrameSummary, FrameTimes, HitRegions, ShellError, ShellOptions, UiBackend, UiBackendPref,
};

enum UserEvent {
    Invoke(Box<dyn FnOnce() + Send>),
    Quit,
}

enum Ui {
    Gpu(GpuUi),
    Soft(SoftUi),
}

// Es gibt genau eine Instanz je Prozess; Boxen brächte nur Indirektion.
#[allow(clippy::large_enum_variant)]
enum Renderers {
    Gpu(FemtoVGWGPURenderer),
    Soft(SoftwareRenderer),
}

struct Recorder {
    times: FrameTimes,
    parts: [FrameTimes; 3],
    last: Option<Instant>,
}

/// Gemeinsamer Zustand (nur UI-Thread).
struct Core {
    window: Window,
    hwnd: isize,
    comp: Compositor,
    ui: RefCell<Ui>,
    video: VideoLayer,
    nc: Rc<NcShared>,
    _subclass: Option<Subclass>,
    size: Cell<(u32, u32)>,
    scale: Cell<f64>,
    cursor: Cell<(f32, f32)>,
    redraw_pending: Cell<bool>,
    recorder: RefCell<Option<Recorder>>,
    last_parts: RefCell<[FrameSummary; 3]>,
    backend: UiBackend,
    backend_note: String,
    proxy: winit::event_loop::EventLoopProxy<UserEvent>,
    min_size: Cell<(f32, f32)>,
    quit_on_hide: Cell<bool>,
}

impl Core {
    fn request_redraw(&self) {
        if !self.redraw_pending.replace(true) {
            self.window.request_redraw();
        }
    }
}

struct Adapter {
    window: slint::Window,
    core: Rc<Core>,
    renderer: Renderers,
}

impl Adapter {
    fn dispatch(&self, ev: WindowEvent) {
        self.window.dispatch_event(ev);
    }

    fn logical(&self, x: f32, y: f32) -> LogicalPosition {
        let k = self.core.scale.get() as f32;
        LogicalPosition::new(x / k, y / k)
    }

    fn on_resized(&self, w: u32, h: u32) {
        let c = &self.core;
        if (w, h) == c.size.get() {
            return;
        }
        c.size.set((w, h));
        if w == 0 || h == 0 {
            return;
        }
        match (&mut *c.ui.borrow_mut(), &self.renderer) {
            (Ui::Gpu(g), _) => g.resize(&c.comp, (w, h)),
            (Ui::Soft(s), Renderers::Soft(r)) => s.resize(r, (w, h)),
            _ => {}
        }
        c.video.set_window((w, h), c.scale.get());
        let k = c.scale.get() as f32;
        self.dispatch(WindowEvent::Resized {
            size: LogicalSize::new(w as f32 / k, h as f32 / k),
        });
        // Sofort zeichnen: sonst streckt DWM das alte Bild, solange der Nutzer zieht.
        self.draw();
    }

    fn draw(&self) {
        let c = &self.core;
        c.redraw_pending.set(false);
        let (w, h) = c.size.get();
        if w == 0 || h == 0 {
            return;
        }
        slint::platform::update_timers_and_animations();
        if let Some(rec) = c.recorder.borrow_mut().as_mut() {
            let now = Instant::now();
            if let Some(last) = rec.last {
                rec.times.push((now - last).as_secs_f32() * 1000.0);
            }
            rec.last = Some(now);
        }
        let res = match (&mut *c.ui.borrow_mut(), &self.renderer) {
            (Ui::Gpu(g), Renderers::Gpu(r)) => g.draw(r, &c.comp),
            (Ui::Soft(s), Renderers::Soft(r)) => s.draw(r),
            _ => Ok(DrawResult::Skipped),
        };
        if let (Ui::Gpu(g), Some(rec)) = (&*c.ui.borrow(), c.recorder.borrow_mut().as_mut()) {
            rec.parts[0].push(g.last_ms.0);
            rec.parts[1].push(g.last_ms.1);
            rec.parts[2].push(g.last_ms.2);
        }
        if let Err(e) = res {
            eprintln!("lfx-shell: Zeichnen fehlgeschlagen: {e}");
        }
        if self.window.has_active_animations() {
            c.request_redraw();
        }
    }
}

impl WindowAdapter for Adapter {
    fn window(&self) -> &slint::Window {
        &self.window
    }

    fn size(&self) -> PhysicalSize {
        let (w, h) = self.core.size.get();
        PhysicalSize::new(w, h)
    }

    fn renderer(&self) -> &dyn Renderer {
        match &self.renderer {
            Renderers::Gpu(r) => r,
            Renderers::Soft(r) => r,
        }
    }

    fn set_visible(&self, visible: bool) -> Result<(), PlatformError> {
        self.core.window.set_visible(visible);
        if visible {
            self.core.request_redraw();
        } else if self.core.quit_on_hide.get() {
            let _ = self.core.proxy.send_event(UserEvent::Quit);
        }
        Ok(())
    }

    fn request_redraw(&self) {
        self.core.request_redraw();
    }

    fn set_size(&self, size: WindowSize) {
        let s = size.to_physical(self.core.scale.get() as f32);
        let _ = self
            .core
            .window
            .request_inner_size(winit::dpi::PhysicalSize::new(s.width, s.height));
    }

    fn position(&self) -> Option<PhysicalPosition> {
        self.core
            .window
            .outer_position()
            .ok()
            .map(|p| PhysicalPosition::new(p.x, p.y))
    }

    fn set_position(&self, position: WindowPosition) {
        let p = position.to_physical(self.core.scale.get() as f32);
        self.core
            .window
            .set_outer_position(winit::dpi::PhysicalPosition::new(p.x, p.y));
    }

    fn update_window_properties(&self, p: WindowProperties<'_>) {
        let title = p.title();
        if !title.is_empty() {
            self.core.window.set_title(&title);
        }
        let (mw, mh) = self.core.min_size.get();
        let c = p.layout_constraints();
        let (mw, mh) = c
            .min
            .map_or((mw, mh), |m| (m.width.max(mw), m.height.max(mh)));
        self.core
            .window
            .set_min_inner_size(Some(winit::dpi::LogicalSize::new(mw, mh)));
    }

    fn internal(&self, _: InternalToken) -> Option<&dyn WindowAdapterInternal> {
        Some(self)
    }
}

impl WindowAdapterInternal for Adapter {
    fn set_mouse_cursor(&self, cursor: MouseCursorInner) -> MouseCursorHandling {
        match &cursor {
            MouseCursorInner::BuiltIn(b) => match input::cursor_icon(b) {
                Some(icon) => {
                    self.core.window.set_cursor_visible(true);
                    self.core.window.set_cursor(icon);
                }
                None => self.core.window.set_cursor_visible(false),
            },
            _ => {
                self.core.window.set_cursor_visible(true);
                self.core
                    .window
                    .set_cursor(winit::window::CursorIcon::Default);
            }
        }
        MouseCursorHandling::Platform
    }

    fn input_method_request(&self, req: InputMethodRequest) {
        match req {
            InputMethodRequest::Enable(p) | InputMethodRequest::Update(p) => {
                self.core.window.set_ime_allowed(true);
                self.core.window.set_ime_cursor_area(
                    winit::dpi::LogicalPosition::new(
                        p.cursor_rect_origin.x as f64,
                        p.cursor_rect_origin.y as f64,
                    ),
                    winit::dpi::LogicalSize::new(
                        p.cursor_rect_size.width as f64,
                        p.cursor_rect_size.height as f64,
                    ),
                );
            }
            InputMethodRequest::Disable => self.core.window.set_ime_allowed(false),
            _ => {}
        }
    }

    fn unregister_item_tree(
        &self,
        component: ItemTreeRef,
        items: &mut dyn Iterator<Item = Pin<ItemRef<'_>>>,
    ) {
        let _ = match &self.renderer {
            Renderers::Gpu(r) => r.free_graphics_resources(component, items),
            Renderers::Soft(r) => r.free_graphics_resources(component, items),
        };
    }
}

struct Proxy(winit::event_loop::EventLoopProxy<UserEvent>);

impl EventLoopProxy for Proxy {
    fn quit_event_loop(&self) -> Result<(), EventLoopError> {
        self.0
            .send_event(UserEvent::Quit)
            .map_err(|_| EventLoopError::EventLoopTerminated)
    }

    fn invoke_from_event_loop(
        &self,
        event: Box<dyn FnOnce() + Send>,
    ) -> Result<(), EventLoopError> {
        self.0
            .send_event(UserEvent::Invoke(event))
            .map_err(|_| EventLoopError::EventLoopTerminated)
    }
}

struct LfxPlatform {
    adapter: Rc<Adapter>,
    event_loop: RefCell<Option<EventLoop<UserEvent>>>,
    proxy: winit::event_loop::EventLoopProxy<UserEvent>,
    taken: Cell<bool>,
}

impl Platform for LfxPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        if self.taken.replace(true) {
            return Err(PlatformError::Other(
                "LENZFLIX-Shell unterstützt nur ein Hauptfenster".into(),
            ));
        }
        Ok(self.adapter.clone())
    }

    fn run_event_loop(&self) -> Result<(), PlatformError> {
        let el = self
            .event_loop
            .borrow_mut()
            .take()
            .ok_or_else(|| PlatformError::Other("Ereignisschleife läuft bereits".into()))?;
        let mut app = App {
            adapter: self.adapter.clone(),
        };
        el.run_app(&mut app)
            .map_err(|e| PlatformError::Other(e.to_string()))
    }

    fn new_event_loop_proxy(&self) -> Option<Box<dyn EventLoopProxy>> {
        Some(Box::new(Proxy(self.proxy.clone())))
    }
}

struct App {
    adapter: Rc<Adapter>,
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, _el: &ActiveEventLoop) {}

    fn user_event(&mut self, el: &ActiveEventLoop, ev: UserEvent) {
        match ev {
            UserEvent::Invoke(f) => f(),
            UserEvent::Quit => el.exit(),
        }
    }

    fn window_event(&mut self, _el: &ActiveEventLoop, _id: WindowId, ev: WEvent) {
        let a = &self.adapter;
        let c = &a.core;
        match ev {
            WEvent::CloseRequested => a.dispatch(WindowEvent::CloseRequested),
            WEvent::Resized(s) => a.on_resized(s.width, s.height),
            WEvent::ScaleFactorChanged { scale_factor, .. } => {
                c.scale.set(scale_factor);
                c.nc.scale.set(scale_factor);
                let (w, h) = c.size.get();
                c.video.set_window((w, h), scale_factor);
                a.dispatch(WindowEvent::ScaleFactorChanged {
                    scale_factor: scale_factor as f32,
                });
            }
            WEvent::CursorMoved { position, .. } => {
                c.cursor.set((position.x as f32, position.y as f32));
                a.dispatch(WindowEvent::PointerMoved {
                    position: a.logical(position.x as f32, position.y as f32),
                });
            }
            WEvent::CursorLeft { .. } => a.dispatch(WindowEvent::PointerExited),
            WEvent::MouseInput { state, button, .. } => {
                let (x, y) = c.cursor.get();
                let position = a.logical(x, y);
                let button = input::mouse_button(button);
                a.dispatch(match state {
                    ElementState::Pressed => WindowEvent::PointerPressed { position, button },
                    ElementState::Released => WindowEvent::PointerReleased { position, button },
                });
            }
            WEvent::MouseWheel { delta, .. } => {
                let (dx, dy) = input::scroll_delta(delta, c.scale.get());
                let (x, y) = c.cursor.get();
                a.dispatch(WindowEvent::PointerScrolled {
                    position: a.logical(x, y),
                    delta_x: dx,
                    delta_y: dy,
                });
            }
            WEvent::KeyboardInput { event, .. } => {
                if let Some(e) = input::key_event(&event) {
                    a.dispatch(e);
                }
            }
            WEvent::Ime(Ime::Commit(text)) => {
                let t = slint::SharedString::from(text.as_str());
                a.dispatch(WindowEvent::KeyPressed { text: t.clone() });
                a.dispatch(WindowEvent::KeyReleased { text: t });
            }
            WEvent::Focused(f) => a.dispatch(WindowEvent::WindowActiveChanged(f)),
            WEvent::RedrawRequested => a.draw(),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        slint::platform::update_timers_and_animations();
        if self.adapter.window.has_active_animations() {
            self.adapter.core.request_redraw();
        }
        // Direkt zeichnen statt auf WM_PAINT zu warten: spart die Zustellverzögerung der
        // Nachrichtenschlange und hält den Bildtakt gleichmäßiger.
        if self.adapter.core.redraw_pending.get() {
            self.adapter.draw();
        }
        match slint::platform::duration_until_next_timer_update() {
            Some(d) => el.set_control_flow(ControlFlow::WaitUntil(Instant::now() + d)),
            None => el.set_control_flow(ControlFlow::Wait),
        }
    }
}

/// Besitzer der Fensterumgebung; legt Fenster, Komposition und Slint-Plattform an.
pub struct Shell {
    handle: ShellHandle,
}

/// Griff für Fenstersteuerung aus UI-Callbacks (nur im UI-Thread).
#[derive(Clone)]
pub struct ShellHandle {
    core: Rc<Core>,
    adapter: Weak<Adapter>,
}

impl Shell {
    pub fn new(opts: ShellOptions) -> Result<Self, ShellError> {
        let event_loop = EventLoop::<UserEvent>::with_user_event()
            .build()
            .map_err(|e| ShellError::EventLoop(e.to_string()))?;
        let proxy = event_loop.create_proxy();
        let attrs = Window::default_attributes()
            .with_title(opts.title.clone())
            .with_decorations(false)
            .with_visible(false)
            .with_maximized(opts.maximized)
            .with_inner_size(winit::dpi::LogicalSize::new(
                opts.size.0 as f64,
                opts.size.1 as f64,
            ))
            .with_min_inner_size(winit::dpi::LogicalSize::new(
                opts.min_size.0 as f64,
                opts.min_size.1 as f64,
            ))
            .with_no_redirection_bitmap(true)
            .with_undecorated_shadow(true);
        #[allow(deprecated)]
        let window = event_loop
            .create_window(attrs)
            .map_err(|e| ShellError::Window(e.to_string()))?;
        let hwnd = match window.window_handle().map(|h| h.as_raw()) {
            Ok(RawWindowHandle::Win32(h)) => h.hwnd.get(),
            _ => return Err(ShellError::Window("kein Win32-Fenster".into())),
        };
        let inner = window.inner_size();
        let size = (inner.width, inner.height);
        let scale = window.scale_factor();
        let comp = Compositor::new(hwnd)?;

        // UI-Pfad wählen: GPU (wgpu/DX12 auf dem Visual), sonst Software.
        let mut note = String::new();
        let mut gpu_ids = None;
        let mut gpu_core = None;
        if opts.ui_backend == UiBackendPref::Auto {
            match GpuCore::new() {
                Ok(core) => {
                    gpu_ids = Some(core.adapter_ids());
                    note = format!("GPU-Adapter: {}", core.adapter_name());
                    gpu_core = Some(core);
                }
                Err(e) => note = format!("GPU-Pfad nicht verfügbar ({e}), Software-Rückfall"),
            }
        } else {
            note.push_str("Software-UI angefordert");
        }
        let d3d = create_d3d11(gpu_ids)?;
        let video = VideoLayer::new(&d3d, &comp, opts.video_format, size, scale)?;

        let (ui, renderers, backend) = match gpu_core.map(|c| GpuUi::new(c, &comp, size)) {
            Some(Ok((g, r))) => (Ui::Gpu(g), Renderers::Gpu(r), UiBackend::GpuComposition),
            other => {
                if let Some(Err(e)) = other {
                    note = format!("GPU-Oberfläche fehlgeschlagen ({e}), Software-Rückfall");
                }
                let (s, r) = SoftUi::new(&d3d, &comp, size)?;
                (Ui::Soft(s), Renderers::Soft(r), UiBackend::Software)
            }
        };

        let nc = NcShared::new(scale);
        let subclass = Subclass::install(hwnd, nc.clone());
        let core = Rc::new(Core {
            window,
            hwnd,
            comp,
            ui: RefCell::new(ui),
            video,
            nc: nc.clone(),
            _subclass: subclass,
            size: Cell::new(size),
            scale: Cell::new(scale),
            cursor: Cell::new((0.0, 0.0)),
            redraw_pending: Cell::new(false),
            recorder: RefCell::new(None),
            last_parts: RefCell::new([FrameSummary::default(); 3]),
            backend,
            backend_note: note,
            proxy: proxy.clone(),
            min_size: Cell::new(opts.min_size),
            quit_on_hide: Cell::new(true),
        });
        let adapter = Rc::new_cyclic(|weak: &Weak<Adapter>| Adapter {
            window: slint::Window::new(weak.clone() as Weak<dyn WindowAdapter>),
            core: core.clone(),
            renderer: renderers,
        });
        // Zeigerereignisse des Maximieren-Knopfs (Nicht-Client-Nachrichten) an Slint weitergeben.
        let weak = Rc::downgrade(&adapter);
        *nc.pointer.borrow_mut() = Some(Box::new(move |ev| {
            let Some(a) = weak.upgrade() else { return };
            let button = PointerEventButton::Left;
            a.dispatch(match ev {
                NcPointer::Moved(x, y) => WindowEvent::PointerMoved {
                    position: LogicalPosition::new(x, y),
                },
                NcPointer::Pressed(x, y) => WindowEvent::PointerPressed {
                    position: LogicalPosition::new(x, y),
                    button,
                },
                NcPointer::Released(x, y) => WindowEvent::PointerReleased {
                    position: LogicalPosition::new(x, y),
                    button,
                },
                NcPointer::Exited => WindowEvent::PointerExited,
            });
        }));
        // Slint die Anfangsgröße und Skalierung mitteilen.
        adapter.dispatch(WindowEvent::ScaleFactorChanged {
            scale_factor: scale as f32,
        });
        adapter.dispatch(WindowEvent::Resized {
            size: LogicalSize::new(size.0 as f32 / scale as f32, size.1 as f32 / scale as f32),
        });

        let handle = ShellHandle {
            core: core.clone(),
            adapter: Rc::downgrade(&adapter),
        };
        let platform = LfxPlatform {
            adapter,
            event_loop: RefCell::new(Some(event_loop)),
            proxy,
            taken: Cell::new(false),
        };
        slint::platform::set_platform(Box::new(platform))
            .map_err(|e| ShellError::Platform(format!("{e:?}")))?;
        Ok(Self { handle })
    }

    pub fn handle(&self) -> ShellHandle {
        self.handle.clone()
    }

    /// Unter Windows ohne Wirkung (die Plattform kennt das Fenster ohnehin); gleiche API wie auf
    /// anderen Systemen, wo die Verbindung zum Slint-Fenster nötig ist.
    pub fn attach<C: slint::ComponentHandle + 'static>(&self, _component: &C) {}

    /// Zeigt die Komponente und läuft bis zum Schließen.
    pub fn show_and_run<C: slint::ComponentHandle + 'static>(
        &self,
        component: &C,
    ) -> Result<(), ShellError> {
        component
            .show()
            .map_err(|e| ShellError::Window(e.to_string()))?;
        self.run()
    }

    /// Läuft, bis das Fenster geschlossen oder `slint::quit_event_loop()` aufgerufen wird.
    pub fn run(&self) -> Result<(), ShellError> {
        slint::run_event_loop().map_err(|e| ShellError::EventLoop(e.to_string()))
    }
}

impl ShellHandle {
    /// Startet das Ziehen des Fensters (für Titelleisten ohne `HTCAPTION`-Fläche).
    pub fn drag_window(&self) {
        let _ = self.core.window.drag_window();
    }

    pub fn minimize(&self) {
        self.core.window.set_minimized(true);
    }

    pub fn is_maximized(&self) -> bool {
        self.core.window.is_maximized()
    }

    pub fn toggle_maximize(&self) {
        let w = &self.core.window;
        w.set_maximized(!w.is_maximized());
    }

    /// W5-tray: Fenster verstecken (Infobereich), ohne die Ereignisschleife zu beenden.
    pub fn hide_window(&self) {
        self.core.window.set_visible(false);
    }

    /// W5-tray: Fenster wieder zeigen, aus der Minimierung holen und in den Vordergrund bringen.
    pub fn show_window(&self) {
        let w = &self.core.window;
        w.set_visible(true);
        w.set_minimized(false);
        w.focus_window();
        self.core.request_redraw();
    }

    /// W5-tray: ist das Fenster sichtbar (nicht im Infobereich versteckt)?
    pub fn is_window_visible(&self) -> bool {
        self.core.window.is_visible().unwrap_or(true)
    }

    /// Löst die normale Schließanfrage von Slint aus (`close-requested`).
    pub fn close(&self) {
        if let Some(a) = self.adapter.upgrade() {
            a.dispatch(WindowEvent::CloseRequested);
        }
    }

    /// Meldet Ziehfläche, Bedienelemente, Maximieren-Knopf und Resize-Ränder (logische Pixel).
    pub fn set_hit_regions(&self, regions: HitRegions) {
        *self.core.nc.regions.borrow_mut() = regions;
    }

    /// Die Video-Ebene unter der UI.
    pub fn video_layer(&self) -> Option<VideoLayer> {
        Some(self.core.video.clone())
    }

    pub fn ui_backend(&self) -> UiBackend {
        self.core.backend
    }

    /// Kurzer Text, welcher Pfad gewählt wurde und warum (für Protokoll und Messbericht).
    pub fn ui_backend_note(&self) -> &str {
        &self.core.backend_note
    }

    /// Roher `HWND` (für Selbsttests und native Erweiterungen).
    pub fn hwnd_raw(&self) -> isize {
        self.core.hwnd
    }

    /// Physische Client-Größe und Skalierung.
    pub fn window_info(&self) -> ((u32, u32), f64) {
        (self.core.size.get(), self.core.scale.get())
    }

    /// Bildwiederholrate des aktuellen Monitors in mHz (falls bekannt).
    pub fn refresh_rate_mhz(&self) -> Option<u32> {
        self.core
            .window
            .current_monitor()
            .and_then(|m| m.refresh_rate_millihertz())
    }

    /// Beginnt die Bildzeit-Messung; die ersten `skip` Bilder zählen nicht.
    pub fn start_frame_recording(&self, skip: usize) {
        *self.core.recorder.borrow_mut() = Some(Recorder {
            times: FrameTimes::new(skip),
            parts: [
                FrameTimes::new(skip),
                FrameTimes::new(skip),
                FrameTimes::new(skip),
            ],
            last: None,
        });
    }

    /// Beendet die Messung und liefert die Auswertung.
    pub fn stop_frame_recording(&self) -> FrameSummary {
        let rec = self.core.recorder.borrow_mut().take();
        match rec {
            Some(r) => {
                *self.core.last_parts.borrow_mut() = r.parts.each_ref().map(|p| p.summary());
                r.times.summary()
            }
            None => FrameSummary::default(),
        }
    }

    /// Aufteilung der Bildzeit der letzten Messung (nur GPU-Pfad): Textur holen, Slint rendern,
    /// präsentieren. Die Zahlen sind Zeiten der Aufrufe in ms (`fps` ist hier bedeutungslos).
    pub fn last_recording_parts(&self) -> [FrameSummary; 3] {
        *self.core.last_parts.borrow()
    }

    /// Fragt den Hit-Test für einen logischen Client-Punkt über `SendMessage(WM_NCHITTEST)` ab
    /// (Selbsttest, liefert den echten Win32-Code).
    pub fn probe_hit_code(&self, x: f32, y: f32) -> isize {
        let k = self.core.scale.get();
        let origin = self.core.window.inner_position().unwrap_or_default();
        let sx = origin.x + (x as f64 * k).round() as i32;
        let sy = origin.y + (y as f64 * k).round() as i32;
        subclass::hit_code_at_screen(self.core.hwnd, sx, sy)
    }
}
