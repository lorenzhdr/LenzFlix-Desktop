//! UI-Thread: Slint-Software-Renderer, Upload in die UI-Swapchain. Läuft getrennt vom
//! Video-Thread, damit das (CPU-lastige) OSD die Video-Bildrate nicht bremst.

use std::ffi::c_void;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use slint::platform::software_renderer::PremultipliedRgbaColor;
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, PhysicalSize};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_UNKNOWN;
use windows::Win32::Graphics::Dxgi::*;

use crate::ui;

pub enum UiMsg {
    Resize(u32, u32),
    Scale(f64),
    Pointer(f32, f32),
    Button(bool),
    Billboard(bool),
    Paused(bool),
    Info(String),
    Progress { pos: f64, dur: f64 },
    Quit,
}

#[derive(Default)]
pub struct UiStats {
    pub frames: u64,
    pub render_ms: Vec<f32>,
    pub upload_ms: Vec<f32>,
    pub errors: u64,
}

const OCCLUDED: windows::core::HRESULT = windows::core::HRESULT(0x087A0001);

pub fn run(
    ctx: ID3D11DeviceContext,
    sc: IDXGISwapChain1,
    w0: u32,
    h0: u32,
    scale0: f64,
    heading: String,
    rx: Receiver<UiMsg>,
    pause_flag: Arc<AtomicBool>,
) -> UiStats {
    let (swin, osd) = ui::create();
    let mut scale = scale0 as f32;
    let (mut ww, mut wh) = (w0.max(1), h0.max(1));
    swin.set_size(PhysicalSize::new(ww, wh));
    swin.dispatch_event(WindowEvent::ScaleFactorChanged {
        scale_factor: scale,
    });
    osd.set_heading(heading.into());
    osd.show().expect("Osd anzeigen");
    {
        let pf = pause_flag.clone();
        osd.on_toggle_pause(move || pf.store(true, Ordering::Release));
    }
    let mut buf = vec![PremultipliedRgbaColor::default(); (ww * wh) as usize];
    let mut stats = UiStats::default();
    let mut dirty = true;
    let mut billboard = false;
    let mut scroll = 0.0f32;
    let mut last_scroll = Instant::now();
    let mut running = true;

    while running {
        // Auf Nachrichten warten (Billboard: ~60 Hz Animation, sonst ruhiger).
        let wait = if billboard {
            Duration::from_millis(8)
        } else {
            Duration::from_millis(20)
        };
        let mut first = rx.recv_timeout(wait);
        loop {
            match first {
                Ok(UiMsg::Resize(w, h)) => {
                    if (w, h) != (ww, wh) {
                        ww = w;
                        wh = h;
                        // SAFETY: keine Backbuffer-Referenzen gehalten.
                        let r = unsafe {
                            sc.ResizeBuffers(
                                0,
                                ww,
                                wh,
                                DXGI_FORMAT_UNKNOWN,
                                DXGI_SWAP_CHAIN_FLAG(0),
                            )
                        };
                        if let Err(er) = r {
                            stats.errors += 1;
                            println!("FEHLER UI-ResizeBuffers {ww}x{wh}: {er}");
                        }
                        swin.set_size(PhysicalSize::new(ww, wh));
                        buf = vec![PremultipliedRgbaColor::default(); (ww * wh) as usize];
                        dirty = true;
                    }
                }
                Ok(UiMsg::Scale(s)) => {
                    scale = s as f32;
                    swin.dispatch_event(WindowEvent::ScaleFactorChanged {
                        scale_factor: scale,
                    });
                    dirty = true;
                }
                Ok(UiMsg::Pointer(x, y)) => {
                    swin.dispatch_event(WindowEvent::PointerMoved {
                        position: LogicalPosition::new(x / scale, y / scale),
                    });
                }
                Ok(UiMsg::Button(down)) => {
                    // Position kam per Pointer-Nachricht; Slint merkt sie sich.
                    let position = LogicalPosition::new(-1.0, -1.0);
                    swin.dispatch_event(if down {
                        WindowEvent::PointerPressed {
                            position,
                            button: PointerEventButton::Left,
                        }
                    } else {
                        WindowEvent::PointerReleased {
                            position,
                            button: PointerEventButton::Left,
                        }
                    });
                }
                Ok(UiMsg::Billboard(b)) => {
                    billboard = b;
                    osd.set_billboard(b);
                    dirty = true;
                }
                Ok(UiMsg::Paused(p)) => osd.set_paused(p),
                Ok(UiMsg::Info(t)) => osd.set_info(t.into()),
                Ok(UiMsg::Progress { pos, dur }) => {
                    let prog = if dur > 0.0 {
                        (pos / dur) as f32
                    } else {
                        ((pos % 60.0) / 60.0) as f32
                    };
                    osd.set_progress(prog.clamp(0.0, 1.0));
                    osd.set_time_text(
                        format!("{:02}:{:04.1}", (pos / 60.0) as u32, pos % 60.0).into(),
                    );
                }
                Ok(UiMsg::Quit) | Err(RecvTimeoutError::Disconnected) => {
                    running = false;
                    break;
                }
                Err(RecvTimeoutError::Timeout) => break,
            }
            first = match rx.try_recv() {
                Ok(m) => Ok(m),
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    Err(RecvTimeoutError::Disconnected)
                }
            };
        }
        if billboard && last_scroll.elapsed() >= Duration::from_millis(16) {
            last_scroll = Instant::now();
            scroll = (scroll + 4.0) % 400.0;
            osd.set_scroll_x(scroll);
        }
        slint::platform::update_timers_and_animations();
        let t0 = Instant::now();
        let drew = swin.draw_if_needed(|renderer| {
            renderer.render(&mut buf, ww as usize);
        });
        if drew {
            stats.render_ms.push(t0.elapsed().as_secs_f32() * 1000.0);
        }
        if drew || dirty {
            dirty = false;
            let t1 = Instant::now();
            // SAFETY: Puffer ist ww*wh*4 Byte groß (RGBA8), Zeilenabstand ww*4.
            unsafe {
                match sc.GetBuffer::<ID3D11Texture2D>(0) {
                    Ok(tex) => {
                        ctx.UpdateSubresource(
                            &tex,
                            0,
                            None,
                            buf.as_ptr() as *const c_void,
                            ww * 4,
                            0,
                        );
                    }
                    Err(er) => {
                        stats.errors += 1;
                        println!("FEHLER UI-GetBuffer: {er}");
                    }
                }
                let hr = sc.Present(0, DXGI_PRESENT(0));
                if hr.is_err() && hr != OCCLUDED {
                    stats.errors += 1;
                    println!("FEHLER UI-Present: {hr:?}");
                }
            }
            stats.upload_ms.push(t1.elapsed().as_secs_f32() * 1000.0);
            stats.frames += 1;
        }
    }
    stats
}
