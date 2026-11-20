//! Grafik-Thread: D3D11-Gerät, DirectComposition, mpv-Render-Schleife, Slint-OSD, Messung.

use std::ffi::c_void;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::thread::Thread;
use std::time::{Duration, Instant};

use lfx_mpv_sys::ffi::*;
use lfx_mpv_sys::{Api, EndReason, Event, Mpv, RenderContext};
use windows::Win32::Foundation::{HMODULE, HWND};
use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
};
use windows::Win32::Graphics::Direct3D10::ID3D10Multithread;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::DirectComposition::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::core::Interface;

use crate::uithread::{self, UiMsg};
use crate::{Config, Msg};

macro_rules! log {
    ($($a:tt)*) => {{ println!("[{:>8.3}] {}", T0.with(|t| t.elapsed().as_secs_f64()), format!($($a)*)); }};
}

const OCCLUDED: windows::core::HRESULT = windows::core::HRESULT(0x087A0001);

thread_local! { static T0: Instant = Instant::now(); }

/// Gemeinsamer Zustand mit dem mpv-Update-Callback.
struct Wake {
    flag: AtomicBool,
    thread: Thread,
}

unsafe extern "C" fn on_update(ctx: *mut c_void) {
    // SAFETY: `ctx` ist ein `Arc<Wake>`-Rohzeiger, der bis nach dem Freigeben des Kontexts lebt.
    let w = unsafe { &*(ctx as *const Wake) };
    w.flag.store(true, Ordering::Release);
    w.thread.unpark();
}

struct Video {
    sc: IDXGISwapChain1,
    format: DXGI_FORMAT,
    w: u32,
    h: u32,
}

struct Stats {
    video_frames: u64,
    ui_frames: u64,
    present_ms: Vec<f32>,
    render_ms: Vec<f32>,
    intervals_ms: Vec<f32>,
    last_present: Option<Instant>,
    present_errors: u64,
    occluded: u64,
    resize_errors: u64,
    resizes: u64,
    black_checks: u64,
    black_frames: u64,
    render_errors: u64,
    ui_render_ms: Vec<f32>,
    ui_upload_ms: Vec<f32>,
}

fn pct(v: &mut [f32], p: f64) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[((v.len() - 1) as f64 * p) as usize]
}

fn create_video_sc(
    factory: &IDXGIFactory2,
    dev: &ID3D11Device,
    pq: bool,
    w: u32,
    h: u32,
) -> windows::core::Result<Video> {
    let format = if pq {
        DXGI_FORMAT_R10G10B10A2_UNORM
    } else {
        DXGI_FORMAT_R16G16B16A16_FLOAT
    };
    let desc = DXGI_SWAP_CHAIN_DESC1 {
        Width: w,
        Height: h,
        Format: format,
        Stereo: false.into(),
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
        BufferCount: 2,
        Scaling: DXGI_SCALING_STRETCH,
        SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
        AlphaMode: DXGI_ALPHA_MODE_IGNORE,
        Flags: 0,
    };
    // SAFETY: gültiges Gerät und gültige Beschreibung.
    let sc = unsafe { factory.CreateSwapChainForComposition(dev, &desc, None::<&IDXGIOutput>) }?;
    Ok(Video { sc, format, w, h })
}

fn create_ui_sc(
    factory: &IDXGIFactory2,
    dev: &ID3D11Device,
    w: u32,
    h: u32,
) -> windows::core::Result<IDXGISwapChain1> {
    // Hinweis: RGBA statt BGRA, damit Slints Pixelreihenfolge ohne CPU-Umsortierung passt.
    let desc = DXGI_SWAP_CHAIN_DESC1 {
        Width: w,
        Height: h,
        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
        Stereo: false.into(),
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
        BufferCount: 2,
        Scaling: DXGI_SCALING_STRETCH,
        SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
        AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
        Flags: 0,
    };
    // SAFETY: wie oben.
    unsafe { factory.CreateSwapChainForComposition(dev, &desc, None::<&IDXGIOutput>) }
}

/// HDR-Fähigkeit der ersten Ausgabe protokollieren.
fn log_hdr_caps(adapter: &IDXGIAdapter1) {
    // SAFETY: reine Abfragen.
    unsafe {
        let mut i = 0;
        while let Ok(out) = adapter.EnumOutputs(i) {
            match out.cast::<IDXGIOutput6>() {
                Ok(o6) => match o6.GetDesc1() {
                    Ok(d) => log!(
                        "Ausgabe {i}: ColorSpace={} BitsPerColor={} MinLum={:.4} MaxLum={:.1} MaxFullFrameLum={:.1} (nits)",
                        d.ColorSpace.0,
                        d.BitsPerColor,
                        d.MinLuminance,
                        d.MaxLuminance,
                        d.MaxFullFrameLuminance
                    ),
                    Err(e) => log!("Ausgabe {i}: GetDesc1 fehlgeschlagen: {e}"),
                },
                Err(e) => log!("Ausgabe {i}: kein IDXGIOutput6: {e}"),
            }
            i += 1;
        }
    }
}

fn u16_prim(v: f64) -> u16 {
    (v * 50000.0).round() as u16
}

/// HDR10-Metadaten aus mpv `video-params` setzen (nur sinnvoll im PQ-Modus).
fn apply_hdr_metadata(sc: &IDXGISwapChain1, mpv: &Mpv, pq: bool) {
    let prim = mpv
        .get_property_string("video-params/primaries")
        .unwrap_or_default();
    let gamma = mpv
        .get_property_string("video-params/gamma")
        .unwrap_or_default();
    let sig_peak = mpv.get_property_f64("video-params/sig-peak").ok();
    let max_cll = mpv.get_property_f64("video-params/max-cll").ok();
    let max_fall = mpv.get_property_f64("video-params/max-fall").ok();
    log!(
        "video-params: primaries={prim:?} gamma={gamma:?} sig-peak={sig_peak:?} max-cll={max_cll:?} max-fall={max_fall:?}"
    );
    if !pq {
        log!("SetHDRMetaData: scRGB-Modus, Metadaten entfallen (Pfad nur im PQ-Modus aktiv)");
        return;
    }
    // Primärvalenzen (x,y) nach Name; Rückfall BT.2020.
    let (r, g, b, w) = match prim.as_str() {
        "bt.709" => ((0.64, 0.33), (0.30, 0.60), (0.15, 0.06), (0.3127, 0.3290)),
        "display-p3" => ((0.68, 0.32), (0.265, 0.69), (0.15, 0.06), (0.3127, 0.3290)),
        _ => (
            (0.708, 0.292),
            (0.170, 0.797),
            (0.131, 0.046),
            (0.3127, 0.3290),
        ),
    };
    let peak_nits = sig_peak
        .map(|p| p * 203.0)
        .unwrap_or(1000.0)
        .clamp(100.0, 10000.0);
    let md = DXGI_HDR_METADATA_HDR10 {
        RedPrimary: [u16_prim(r.0), u16_prim(r.1)],
        GreenPrimary: [u16_prim(g.0), u16_prim(g.1)],
        BluePrimary: [u16_prim(b.0), u16_prim(b.1)],
        WhitePoint: [u16_prim(w.0), u16_prim(w.1)],
        MaxMasteringLuminance: peak_nits as u32,
        MinMasteringLuminance: 50, // 0,005 nit
        MaxContentLightLevel: max_cll.unwrap_or(peak_nits).clamp(0.0, 65535.0) as u16,
        MaxFrameAverageLightLevel: max_fall.unwrap_or(400.0).clamp(0.0, 65535.0) as u16,
    };
    match sc.cast::<IDXGISwapChain4>() {
        Ok(sc4) => {
            // SAFETY: Zeiger auf eine lokale Struktur passender Größe.
            let r = unsafe {
                sc4.SetHDRMetaData(
                    DXGI_HDR_METADATA_TYPE_HDR10,
                    Some(std::slice::from_raw_parts(
                        &md as *const _ as *const u8,
                        std::mem::size_of::<DXGI_HDR_METADATA_HDR10>(),
                    )),
                )
            };
            log!(
                "SetHDRMetaData(HDR10, maxLum={} nit): {r:?}",
                md.MaxMasteringLuminance
            );
        }
        Err(e) => log!("SetHDRMetaData: kein IDXGISwapChain4: {e}"),
    }
}

/// Liest einen kleinen Mittenausschnitt zurück und meldet, ob er nicht schwarz ist.
fn center_nonblack(
    dev: &ID3D11Device,
    ctx: &ID3D11DeviceContext,
    tex: &ID3D11Texture2D,
    v: &Video,
) -> Option<bool> {
    let bpp = match v.format {
        DXGI_FORMAT_R16G16B16A16_FLOAT => 8,
        _ => 4,
    };
    let desc = D3D11_TEXTURE2D_DESC {
        Width: 8,
        Height: 8,
        MipLevels: 1,
        ArraySize: 1,
        Format: v.format,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_STAGING,
        BindFlags: 0,
        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
        MiscFlags: 0,
    };
    let mut st: Option<ID3D11Texture2D> = None;
    // SAFETY: gültige Beschreibung, Ausgabe-Zeiger gültig.
    unsafe {
        dev.CreateTexture2D(&desc, None, Some(&mut st)).ok()?;
        let st = st?;
        let (cx, cy) = (v.w / 2, v.h / 2);
        let bx = D3D11_BOX {
            left: cx.saturating_sub(4),
            top: cy.saturating_sub(4),
            front: 0,
            right: cx.saturating_sub(4) + 8,
            bottom: cy.saturating_sub(4) + 8,
            back: 1,
        };
        ctx.CopySubresourceRegion(&st, 0, 0, 0, 0, tex, 0, Some(&bx));
        let mut m = D3D11_MAPPED_SUBRESOURCE::default();
        ctx.Map(&st, 0, D3D11_MAP_READ, 0, Some(&mut m)).ok()?;
        let mut any = false;
        for row in 0..8usize {
            let p = (m.pData as *const u8).add(row * m.RowPitch as usize);
            let px = std::slice::from_raw_parts(p, 8 * bpp);
            // Alpha-Kanal (letzte 2 Bytes bei RGBA16F, letzte 2 Bit bei RGB10A2) mitzuzählen ist
            // harmlos: Alpha bleibt bei 1.0 ohnehin ungleich 0, daher nur RGB-Bytes prüfen.
            for chunk in px.chunks(bpp) {
                let rgb = if bpp == 8 { &chunk[..6] } else { &chunk[..3] };
                if rgb.iter().any(|b| *b != 0) {
                    any = true;
                }
            }
        }
        ctx.Unmap(&st, 0);
        Some(any)
    }
}

pub fn run(
    hwnd: isize,
    w0: u32,
    h0: u32,
    scale0: f64,
    cfg: Config,
    rx: Receiver<Msg>,
    _tx: Sender<Msg>,
) -> i32 {
    T0.with(|_| ());
    log!(
        "Start: Fenster {w0}x{h0}, DPR {scale0}, HDR-Modus {}",
        if cfg.pq {
            "PQ/RGB10A2"
        } else {
            "scRGB/RGBA16F"
        }
    );
    match run_inner(hwnd, w0, h0, scale0, cfg, rx) {
        Ok(code) => code,
        Err(e) => {
            log!("FEHLER: {e}");
            2
        }
    }
}

fn run_inner(
    hwnd: isize,
    w0: u32,
    h0: u32,
    scale0: f64,
    cfg: Config,
    rx: Receiver<Msg>,
) -> Result<i32, String> {
    let e = |s: &'static str| move |er: windows::core::Error| format!("{s}: {er}");
    // --- D3D11 ---
    // SAFETY: Initialisierung der Grafik-Objekte; alle Zeiger stammen aus den Aufrufen selbst.
    let (factory, adapter, dev, ctx) = unsafe {
        let factory: IDXGIFactory2 =
            CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0)).map_err(e("CreateDXGIFactory2"))?;
        let adapter = factory.EnumAdapters1(0).map_err(e("EnumAdapters1"))?;
        let ad = adapter.GetDesc1().map_err(e("GetDesc1"))?;
        let name = String::from_utf16_lossy(&ad.Description);
        log!(
            "Adapter: {} (VendorId {:#06x}, VRAM {} MiB)",
            name.trim_end_matches('\0'),
            ad.VendorId,
            ad.DedicatedVideoMemory / (1 << 20)
        );
        let mut dev: Option<ID3D11Device> = None;
        let mut ctx: Option<ID3D11DeviceContext> = None;
        let levels = [D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0];
        D3D11CreateDevice(
            &adapter,
            D3D_DRIVER_TYPE_UNKNOWN,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&levels),
            D3D11_SDK_VERSION,
            Some(&mut dev),
            None,
            Some(&mut ctx),
        )
        .map_err(e("D3D11CreateDevice"))?;
        (
            factory,
            adapter,
            dev.ok_or("kein Gerät")?,
            ctx.ok_or("kein Kontext")?,
        )
    };
    log_hdr_caps(&adapter);
    // SAFETY: Schnittstellenabfrage auf dem Gerät.
    unsafe {
        let mt: ID3D10Multithread = dev.cast().map_err(e("ID3D10Multithread"))?;
        let was = mt.SetMultithreadProtected(true);
        log!("SetMultithreadProtected(TRUE), vorher {}", was.as_bool());
    }

    // --- Swapchains + DirectComposition ---
    let mut video = create_video_sc(&factory, &dev, cfg.pq, w0.max(1), h0.max(1))
        .map_err(e("Video-Swapchain"))?;
    let cs = if cfg.pq {
        DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020
    } else {
        DXGI_COLOR_SPACE_RGB_FULL_G10_NONE_P709
    };
    // SAFETY: Abfragen/Setzen des Farbraums auf der eigenen Swapchain.
    unsafe {
        let sc3: IDXGISwapChain3 = video.sc.cast().map_err(e("IDXGISwapChain3"))?;
        match sc3.CheckColorSpaceSupport(cs) {
            Ok(f) => log!("CheckColorSpaceSupport({}): Flags {:#x}", cs.0, f),
            Err(er) => log!("CheckColorSpaceSupport fehlgeschlagen: {er}"),
        }
        match sc3.SetColorSpace1(cs) {
            Ok(()) => log!("SetColorSpace1({}) ok", cs.0),
            Err(er) => log!("SetColorSpace1 fehlgeschlagen: {er}"),
        }
    }
    let ui_sc = create_ui_sc(&factory, &dev, w0.max(1), h0.max(1)).map_err(e("UI-Swapchain"))?;
    let (dcomp, _target, video_visual, ui_visual) = unsafe {
        let dxgi_dev: IDXGIDevice = dev.cast().map_err(e("IDXGIDevice"))?;
        let dcomp: IDCompositionDevice =
            DCompositionCreateDevice(&dxgi_dev).map_err(e("DCompositionCreateDevice"))?;
        let target = dcomp
            .CreateTargetForHwnd(HWND(hwnd as *mut c_void), true)
            .map_err(e("CreateTargetForHwnd"))?;
        let root = dcomp.CreateVisual().map_err(e("CreateVisual"))?;
        let vv = dcomp.CreateVisual().map_err(e("CreateVisual"))?;
        let uv = dcomp.CreateVisual().map_err(e("CreateVisual"))?;
        vv.SetContent(&video.sc).map_err(e("SetContent video"))?;
        uv.SetContent(&ui_sc).map_err(e("SetContent ui"))?;
        root.AddVisual(&vv, false, None::<&IDCompositionVisual>)
            .map_err(e("AddVisual"))?;
        root.AddVisual(&uv, true, &vv).map_err(e("AddVisual"))?;
        target.SetRoot(&root).map_err(e("SetRoot"))?;
        dcomp.Commit().map_err(e("Commit"))?;
        (dcomp, target, vv, uv)
    };
    let _ = &ui_visual;

    // --- mpv ---
    let api = Arc::new(Api::load().map_err(|er| format!("libmpv: {er}"))?);
    log!(
        "libmpv: {} (API {:#x})",
        api.path().display(),
        api.client_api_version()
    );
    let mpv = Mpv::new(api).map_err(|er| er.to_string())?;
    let set = |k: &str, v: &str| {
        if let Err(er) = mpv.set_option_string(k, v) {
            log!("Option {k}={v}: {er}");
        }
    };
    set("vo", "libmpv");
    set("ao", "null");
    set("terminal", "no");
    set("idle", "yes");
    set("hwdec", &cfg.hwdec);
    set("keep-open", "yes");
    if cfg.file.is_some() {
        set("loop-file", "inf");
    }
    if cfg.pq {
        set("target-trc", "pq");
        set("target-prim", "bt.2020");
    } else {
        set("target-trc", "linear");
        set("target-prim", "bt.709");
    }
    for (k, v) in &cfg.opts {
        set(k, v);
    }
    mpv.initialize().map_err(|er| er.to_string())?;
    let _ = mpv.request_log_messages("warn");
    // SAFETY: `dev` ist ein gültiges Gerät mit aktivem Multithread-Schutz.
    let rc = unsafe { RenderContext::create_d3d11(&mpv, dev.as_raw()) };
    let rc = match rc {
        Ok(rc) => {
            log!("RESULT mpv_render_context_create(d3d11) = OK");
            rc
        }
        Err(er) => {
            log!(
                "RESULT mpv_render_context_create(d3d11) FEHLER code={} text={:?}",
                er.code,
                er.text
            );
            // Meldungen aus dem Log-Puffer ausgeben.
            for _ in 0..50 {
                if let Event::LogMessage {
                    prefix,
                    level,
                    text,
                } = mpv.wait_event(0.05)
                {
                    log!("mpv[{prefix}/{level}] {}", text.trim_end());
                }
            }
            return Ok(3);
        }
    };
    let wake = Arc::new(Wake {
        flag: AtomicBool::new(true),
        thread: std::thread::current(),
    });
    let wake_ptr = Arc::into_raw(wake.clone());
    // SAFETY: `wake_ptr` lebt (Arc-Referenz), bis `rc` gefallen ist und `Arc::from_raw` aufgerufen wird.
    unsafe { rc.set_update_callback(Some(on_update), wake_ptr as *mut c_void) };

    let src = cfg
        .file
        .clone()
        .unwrap_or_else(|| format!("av://lavfi:{}", cfg.lavfi));
    let title = cfg
        .file
        .as_deref()
        .map(|_| "Lokale Datei".to_string())
        .unwrap_or_else(|| "Testbild 4K60 (lavfi)".into());
    log!(
        "Quelle: {}",
        if cfg.file.is_some() {
            "<lokale Datei>"
        } else {
            &src
        }
    );
    mpv.command(&["loadfile", &src])
        .map_err(|er| er.to_string())?;
    let _ = mpv.observe_property(1, "pause", MPV_FORMAT_FLAG);

    // --- UI-Thread (Slint) ---
    let (mut ww, mut wh) = (w0.max(1), h0.max(1));
    let paused_flag = Arc::new(AtomicBool::new(false));
    let (ui_tx, ui_rx) = std::sync::mpsc::channel::<UiMsg>();
    let ui_handle = {
        let (ctx2, sc2, pf) = (ctx.clone(), ui_sc.clone(), paused_flag.clone());
        std::thread::Builder::new()
            .name("ui".into())
            .spawn(move || uithread::run(ctx2, sc2, w0.max(1), h0.max(1), scale0, title, ui_rx, pf))
            .expect("ui-Thread")
    };

    let mut billboard = cfg.billboard;
    let mut want_layout = true;
    let mut stats = Stats {
        video_frames: 0,
        ui_frames: 0,
        present_ms: Vec::new(),
        render_ms: Vec::new(),
        intervals_ms: Vec::new(),
        last_present: None,
        present_errors: 0,
        occluded: 0,
        resize_errors: 0,
        resizes: 0,
        black_checks: 0,
        black_frames: 0,
        render_errors: 0,
        ui_render_ms: Vec::new(),
        ui_upload_ms: Vec::new(),
    };
    let t_start = Instant::now();
    let mut last_log = Instant::now();
    let mut last_ui_tick = Instant::now();
    let mut check_black = true;
    let mut running = true;
    let mut redraw = true;
    let mut meta_pending = true;
    let mut first_frame_logged = false;
    let mut pending_resize: Option<(u32, u32)> = None;
    let mut sec_mark = Instant::now();
    let mut sec_frames = 0u64;
    let mut sec_series: Vec<u64> = Vec::new();

    while running {
        // --- Nachrichten ---
        loop {
            match rx.try_recv() {
                Ok(Msg::Resize(w, h)) => {
                    if w > 0 && h > 0 {
                        pending_resize = Some((w, h));
                    }
                }
                Ok(Msg::Scale(s)) => {
                    let _ = ui_tx.send(UiMsg::Scale(s));
                }
                Ok(Msg::Pointer(x, y)) => {
                    let _ = ui_tx.send(UiMsg::Pointer(x, y));
                }
                Ok(Msg::Button(down)) => {
                    let _ = ui_tx.send(UiMsg::Button(down));
                }
                Ok(Msg::ToggleBillboard) => {
                    billboard = !billboard;
                    want_layout = true;
                    log!("Billboard-Modus: {}", if billboard { "an" } else { "aus" });
                }
                Ok(Msg::TogglePause) => paused_flag.store(true, Ordering::Release),
                Ok(Msg::Quit) => running = false,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    running = false;
                    break;
                }
            }
        }
        if paused_flag.swap(false, Ordering::AcqRel) {
            let _ = mpv.command(&["cycle", "pause"]);
        }

        // --- Größenänderung / Layout ---
        if let Some((w, h)) = pending_resize.take() {
            if (w, h) != (ww, wh) {
                ww = w;
                wh = h;
                want_layout = true;
                let _ = ui_tx.send(UiMsg::Resize(ww, wh));
                stats.resizes += 1;
                log!("Resize -> {ww}x{wh}");
            }
        }
        if want_layout {
            want_layout = false;
            let (vx, vy, vw, vh) = if billboard {
                (
                    (ww as f32 * 0.35) as u32,
                    (wh as f32 * 0.08) as u32,
                    (ww as f32 * 0.60) as u32,
                    (wh as f32 * 0.45) as u32,
                )
            } else {
                (0, 0, ww, wh)
            };
            let (vw, vh) = (vw.max(2), vh.max(2));
            if (vw, vh) != (video.w, video.h) {
                // SAFETY: keine Backbuffer-Referenzen gehalten (GetBuffer-Ergebnisse werden pro Frame gefallen).
                let r = unsafe {
                    video
                        .sc
                        .ResizeBuffers(0, vw, vh, DXGI_FORMAT_UNKNOWN, DXGI_SWAP_CHAIN_FLAG(0))
                };
                match r {
                    Ok(()) => {
                        video.w = vw;
                        video.h = vh;
                    }
                    Err(er) => {
                        stats.resize_errors += 1;
                        log!("FEHLER Video-ResizeBuffers {vw}x{vh}: {er}");
                    }
                }
            }
            // SAFETY: Visual-Eigenschaften der eigenen Visuals.
            unsafe {
                let _ = video_visual.SetOffsetX2(vx as f32);
                let _ = video_visual.SetOffsetY2(vy as f32);
                let clip = D2D_RECT_F {
                    left: 0.0,
                    top: 0.0,
                    right: vw as f32,
                    bottom: vh as f32,
                };
                let _ = video_visual.SetClip2(&clip);
                if let Err(er) = dcomp.Commit() {
                    log!("FEHLER Commit: {er}");
                }
            }
            let _ = ui_tx.send(UiMsg::Billboard(billboard));
            redraw = true;
            check_black = true;
        }

        // --- mpv-Ereignisse ---
        loop {
            match mpv.wait_event(0.0) {
                Event::None => break,
                Event::VideoReconfig => meta_pending = true,
                Event::LogMessage {
                    prefix,
                    level,
                    text,
                } => {
                    log!("mpv[{prefix}/{level}] {}", text.trim_end());
                }
                Event::EndFile { reason } => {
                    log!("Dateiende: {reason:?}");
                    if matches!(reason, EndReason::Error(_)) {
                        running = false;
                    }
                }
                Event::Shutdown => running = false,
                Event::PropertyChange {
                    value: lfx_mpv_sys::PropValue::Flag(p),
                    ..
                } => {
                    let _ = ui_tx.send(UiMsg::Paused(p));
                }
                Event::FileLoaded => log!("Datei geladen"),
                _ => {}
            }
        }
        if meta_pending && mpv.get_property_i64("video-params/w").is_ok() {
            meta_pending = false;
            apply_hdr_metadata(&video.sc, &mpv, cfg.pq);
            let hw = mpv.get_property_string("hwdec-current").unwrap_or_default();
            let vf = mpv.get_property_string("video-format").unwrap_or_default();
            let (vw, vh) = (
                mpv.get_property_i64("video-params/w").unwrap_or(0),
                mpv.get_property_i64("video-params/h").unwrap_or(0),
            );
            log!("Video: {vw}x{vh} Format {vf} hwdec-current={hw:?}");
            let _ = ui_tx.send(UiMsg::Info(format!("{vw}x{vh} {vf} hwdec {hw}")));
        }

        // --- Video ---
        let flags = if wake.flag.swap(false, Ordering::AcqRel) {
            rc.update()
        } else {
            0
        };
        if flags & MPV_RENDER_UPDATE_FRAME != 0 || redraw {
            redraw = false;
            let t0 = Instant::now();
            // SAFETY: Backbuffer 0 der Video-Swapchain; Textur lebt bis zum Ende des Blocks.
            unsafe {
                match video.sc.GetBuffer::<ID3D11Texture2D>(0) {
                    Ok(tex) => {
                        let mut fbo = mpv_d3d11_fbo { tex: tex.as_raw() };
                        let params = [mpv_render_param {
                            type_: MPV_RENDER_PARAM_D3D11_FBO,
                            data: &mut fbo as *mut _ as *mut c_void,
                        }];
                        if let Err(er) = rc.render(&params) {
                            stats.render_errors += 1;
                            if stats.render_errors < 5 {
                                log!("FEHLER render: {er}");
                            }
                        }
                        if check_black && flags != 0 {
                            check_black = false;
                            stats.black_checks += 1;
                            match center_nonblack(&dev, &ctx, &tex, &video) {
                                Some(true) => {}
                                Some(false) => {
                                    stats.black_frames += 1;
                                    log!(
                                        "WARNUNG: Bildmitte schwarz nach Resize/Layout ({}x{})",
                                        video.w,
                                        video.h
                                    );
                                }
                                None => log!("Rücklesen fehlgeschlagen"),
                            }
                        }
                    }
                    Err(er) => {
                        stats.render_errors += 1;
                        log!("FEHLER GetBuffer: {er}");
                    }
                }
            }
            let t1 = Instant::now();
            // SAFETY: Present auf der eigenen Swapchain.
            let hr = unsafe { video.sc.Present(1, DXGI_PRESENT(0)) };
            let t2 = Instant::now();
            if hr == OCCLUDED {
                stats.occluded += 1;
            } else if hr.is_err() {
                stats.present_errors += 1;
                if stats.present_errors < 10 {
                    // SAFETY: Abfrage.
                    let reason = unsafe { dev.GetDeviceRemovedReason() };
                    log!("FEHLER Present: {hr:?}, Device-Removed-Grund {reason:?}");
                }
            }
            rc.report_swap();
            stats.video_frames += 1;
            sec_frames += 1;
            stats.render_ms.push((t1 - t0).as_secs_f32() * 1000.0);
            stats.present_ms.push((t2 - t1).as_secs_f32() * 1000.0);
            if let Some(lp) = stats.last_present {
                stats.intervals_ms.push((t2 - lp).as_secs_f32() * 1000.0);
            }
            stats.last_present = Some(t2);
            if !first_frame_logged {
                first_frame_logged = true;
                log!(
                    "Erstes Video-Bild nach {:.2} s",
                    t_start.elapsed().as_secs_f64()
                );
            }
        }

        // --- UI-Daten an den UI-Thread ---
        if last_ui_tick.elapsed() >= Duration::from_millis(33) {
            last_ui_tick = Instant::now();
            let pos = mpv.get_property_f64("time-pos").unwrap_or(0.0);
            let dur = mpv.get_property_f64("duration").unwrap_or(0.0);
            let _ = ui_tx.send(UiMsg::Progress { pos, dur });
        }
        if sec_mark.elapsed() >= Duration::from_secs(1) {
            sec_mark = Instant::now();
            sec_series.push(sec_frames);
            sec_frames = 0;
        }
        // --- periodisches Protokoll ---
        if last_log.elapsed() >= Duration::from_secs(10) {
            last_log = Instant::now();
            log_props(&mpv, "Zwischenstand");
            log!(
                "  eigene Zähler: Video-Bilder {} UI-Bilder {} Present-Fehler {} Resize-Fehler {}",
                stats.video_frames,
                0,
                stats.present_errors,
                stats.resize_errors
            );
        }

        // Schlafen, bis mpv ein Bild meldet (oder kurz für UI/Nachrichten).
        if !wake.flag.load(Ordering::Acquire) {
            std::thread::park_timeout(Duration::from_millis(3));
        }
        // Echtes Minimieren: Fenster hat Größe 0, wird oben ignoriert.
    }

    // --- Zusammenfassung ---
    let _ = ui_tx.send(UiMsg::Quit);
    let ui_stats = ui_handle.join().expect("ui-Thread");
    stats.ui_frames = ui_stats.frames;
    stats.ui_render_ms = ui_stats.render_ms;
    stats.ui_upload_ms = ui_stats.upload_ms;
    stats.present_errors += ui_stats.errors;
    let secs = t_start.elapsed().as_secs_f64();
    log_props(&mpv, "ENDSTAND mpv");
    let n = stats.video_frames;
    let avg_ms = if n > 0 { secs * 1000.0 / n as f64 } else { 0.0 };
    log!(
        "ENDSTAND Messung: Laufzeit {secs:.1} s, Video-Bilder {n} ({:.2} fps), UI-Bilder {}",
        n as f64 / secs,
        stats.ui_frames
    );
    log!(
        "  Bildabstand ms: mittel {avg_ms:.2}, p50 {:.2}, p99 {:.2}, max {:.2}",
        pct(&mut stats.intervals_ms.clone(), 0.5),
        pct(&mut stats.intervals_ms.clone(), 0.99),
        pct(&mut stats.intervals_ms.clone(), 1.0)
    );
    log!(
        "  Render-Aufruf ms: p50 {:.2}, p99 {:.2}, max {:.2}",
        pct(&mut stats.render_ms.clone(), 0.5),
        pct(&mut stats.render_ms.clone(), 0.99),
        pct(&mut stats.render_ms.clone(), 1.0)
    );
    log!(
        "  Present ms: p50 {:.2}, p99 {:.2}, max {:.2}",
        pct(&mut stats.present_ms.clone(), 0.5),
        pct(&mut stats.present_ms.clone(), 0.99),
        pct(&mut stats.present_ms.clone(), 1.0)
    );
    log!(
        "  Fehler: Present {}, Render {}, Resize {} (Resizes {}), verdeckt {}, Schwarzbild-Prüfungen {} (schwarz: {})",
        stats.present_errors,
        stats.render_errors,
        stats.resize_errors,
        stats.resizes,
        stats.occluded,
        stats.black_checks,
        stats.black_frames
    );
    log!(
        "  UI Slint-Render ms: p50 {:.2}, p99 {:.2}, max {:.2}; Upload+Present ms: p50 {:.2}, p99 {:.2}, max {:.2}",
        pct(&mut stats.ui_render_ms.clone(), 0.5),
        pct(&mut stats.ui_render_ms.clone(), 0.99),
        pct(&mut stats.ui_render_ms.clone(), 1.0),
        pct(&mut stats.ui_upload_ms.clone(), 0.5),
        pct(&mut stats.ui_upload_ms.clone(), 0.99),
        pct(&mut stats.ui_upload_ms.clone(), 1.0)
    );
    log!("  Bilder je Sekunde: {sec_series:?}");
    // Aufräumen: Render-Kontext vor mpv freigeben.
    drop(rc);
    // SAFETY: stammt aus `Arc::into_raw` oben.
    unsafe { drop(Arc::from_raw(wake_ptr)) };
    drop(mpv);
    Ok(0)
}

fn log_props(mpv: &Mpv, label: &str) {
    let i = |n: &str| {
        mpv.get_property_i64(n)
            .map(|v| v.to_string())
            .unwrap_or_else(|_| "-".into())
    };
    let f = |n: &str| {
        mpv.get_property_f64(n)
            .map(|v| format!("{v:.2}"))
            .unwrap_or_else(|_| "-".into())
    };
    log!(
        "{label}: frame-drop-count={} decoder-frame-drop-count={} vo-delayed-frame-count={} estimated-vf-fps={} display-fps={} container-fps={} avsync={}",
        i("frame-drop-count"),
        i("decoder-frame-drop-count"),
        i("vo-delayed-frame-count"),
        f("estimated-vf-fps"),
        f("display-fps"),
        f("container-fps"),
        f("avsync"),
    );
}
