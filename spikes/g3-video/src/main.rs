//! Gate G3, Modell A: ein Fenster, DirectComposition-Baum mit zwei Visuals.
//! Unten die Video-Swapchain (mpv rendert per D3D11-Render-API hinein), oben die UI-Swapchain
//! (BGRA-artig, vormultipliziert, gefüllt vom Slint-Software-Renderer).
//!
//! Aufruf: `g3-video [--file <pfad>] [--hdr scrgb|pq] [--billboard] [--duration <s>] [--script]
//!   [--hwdec <wert>] [--opt name=wert]... [--lavfi <ausdruck>]`

#![allow(clippy::too_many_arguments, clippy::collapsible_if)]

mod gfx;
mod ui;
mod uithread;

use std::sync::mpsc;
use std::time::Duration;

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::platform::windows::WindowAttributesExtWindows;
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::{Window, WindowId};

#[derive(Clone, Debug)]
pub struct Config {
    pub file: Option<String>,
    pub lavfi: String,
    pub pq: bool,
    pub billboard: bool,
    pub duration: f64,
    pub script: bool,
    pub hwdec: String,
    pub size: Option<(u32, u32)>,
    pub opts: Vec<(String, String)>,
}

pub enum Msg {
    Resize(u32, u32),
    Scale(f64),
    Pointer(f32, f32),
    Button(bool),
    ToggleBillboard,
    TogglePause,
    Quit,
}

fn parse_args() -> Config {
    let mut c = Config {
        file: None,
        lavfi: "testsrc2=size=3840x2160:rate=60".into(),
        pq: false,
        billboard: false,
        duration: 60.0,
        script: false,
        hwdec: "auto-safe".into(),
        size: None,
        opts: Vec::new(),
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--file" => c.file = it.next(),
            "--lavfi" => c.lavfi = it.next().unwrap_or_default(),
            "--hdr" => c.pq = it.next().as_deref() == Some("pq"),
            "--billboard" => c.billboard = true,
            "--duration" => c.duration = it.next().and_then(|v| v.parse().ok()).unwrap_or(60.0),
            "--script" => c.script = true,
            "--hwdec" => c.hwdec = it.next().unwrap_or_default(),
            "--size" => {
                c.size = it
                    .next()
                    .as_deref()
                    .and_then(|s| s.split_once('x'))
                    .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)));
            }
            "--opt" => {
                if let Some((k, v)) = it.next().as_deref().and_then(|s| s.split_once('=')) {
                    c.opts.push((k.into(), v.into()));
                }
            }
            other => eprintln!("unbekanntes Argument: {other}"),
        }
    }
    c
}

struct App {
    cfg: Config,
    window: Option<Window>,
    tx: Option<mpsc::Sender<Msg>>,
    cursor: (f64, f64),
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("LENZFLIX G3-Spike")
            .with_inner_size(match self.cfg.size {
                Some((w, h)) => winit::dpi::Size::Physical(winit::dpi::PhysicalSize::new(w, h)),
                None => winit::dpi::Size::Logical(LogicalSize::new(1280.0, 720.0)),
            })
            .with_no_redirection_bitmap(true);
        let window = el.create_window(attrs).expect("Fenster");
        let hwnd = match window.window_handle().expect("Handle").as_raw() {
            RawWindowHandle::Win32(h) => h.hwnd.get(),
            _ => panic!("kein Win32-Fenster"),
        };
        let size = window.inner_size();
        let scale = window.scale_factor();
        let (tx, rx) = mpsc::channel();
        let cfg = self.cfg.clone();
        let tx2 = tx.clone();
        std::thread::Builder::new()
            .name("gfx".into())
            .spawn(move || {
                let code = gfx::run(hwnd, size.width, size.height, scale, cfg, rx, tx2);
                std::process::exit(code);
            })
            .expect("gfx-Thread");
        if self.cfg.script {
            let t = tx.clone();
            std::thread::spawn(move || script(hwnd, t));
        }
        // Zeitlimit.
        let t = tx.clone();
        let d = self.cfg.duration;
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs_f64(d));
            let _ = t.send(Msg::Quit);
        });
        self.tx = Some(tx);
        self.window = Some(window);
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, ev: WindowEvent) {
        let Some(tx) = &self.tx else { return };
        match ev {
            WindowEvent::CloseRequested => {
                let _ = tx.send(Msg::Quit);
                el.set_control_flow(ControlFlow::Wait);
            }
            WindowEvent::Resized(s) => {
                let _ = tx.send(Msg::Resize(s.width, s.height));
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                let _ = tx.send(Msg::Scale(scale_factor));
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = (position.x, position.y);
                let _ = tx.send(Msg::Pointer(position.x as f32, position.y as f32));
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                let _ = tx.send(Msg::Pointer(self.cursor.0 as f32, self.cursor.1 as f32));
                let _ = tx.send(Msg::Button(state == ElementState::Pressed));
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                match event.logical_key {
                    Key::Named(NamedKey::Escape) => {
                        let _ = tx.send(Msg::Quit);
                    }
                    Key::Named(NamedKey::Space) => {
                        let _ = tx.send(Msg::TogglePause);
                    }
                    Key::Character(c) if c.eq_ignore_ascii_case("b") => {
                        let _ = tx.send(Msg::ToggleBillboard);
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

/// Fenster-Skript: Resize/Maximize/Restore mehrfach, dazu Billboard-Wechsel.
fn script(hwnd: isize, tx: mpsc::Sender<Msg>) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::*;
    let h = HWND(hwnd as *mut std::ffi::c_void);
    let sleep = |ms: u64| std::thread::sleep(Duration::from_millis(ms));
    sleep(6000);
    let sizes = [
        (1600, 900),
        (900, 600),
        (1920, 1080),
        (1100, 700),
        (2400, 1300),
    ];
    for round in 0..3 {
        for (w, hgt) in sizes {
            // SAFETY: gültiges Fenster-Handle, Aufruf aus fremdem Thread ist erlaubt.
            unsafe {
                let _ = SetWindowPos(h, None, 60, 60, w, hgt, SWP_NOZORDER | SWP_NOACTIVATE);
            }
            sleep(700);
        }
        // SAFETY: wie oben.
        unsafe {
            let _ = ShowWindow(h, SW_MAXIMIZE);
        }
        sleep(1500);
        unsafe {
            let _ = ShowWindow(h, SW_RESTORE);
        }
        sleep(1200);
        unsafe {
            let _ = ShowWindow(h, SW_MINIMIZE);
        }
        sleep(1000);
        unsafe {
            let _ = ShowWindow(h, SW_RESTORE);
        }
        sleep(1000);
        let _ = tx.send(Msg::ToggleBillboard);
        sleep(2500);
        if round == 2 {
            break;
        }
        let _ = tx.send(Msg::ToggleBillboard);
        sleep(1500);
    }
}

fn main() {
    let cfg = parse_args();
    let el = EventLoop::new().expect("Event-Loop");
    el.set_control_flow(ControlFlow::Wait);
    let mut app = App {
        cfg,
        window: None,
        tx: None,
        cursor: (0.0, 0.0),
    };
    el.run_app(&mut app).expect("run");
}
