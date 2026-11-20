//! DirectComposition-Baum und Video-Layer (Windows).
//!
//! Baum: Wurzel -> [Video-Visual (unten), UI-Visual (oben)]. Das Video-Visual zeigt eine
//! D3D11-Composition-Swapchain auf einem eigenen Gerät (BGRA + VIDEO, Multithread-Schutz an), die der
//! Player (mpv `render_d3d11`) füllt. Größen-/Formatwünsche werden erst übernommen, wenn kein
//! Backbuffer ausgeliehen ist (`ResizeBuffers` scheitert sonst).

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

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
use windows::core::{IUnknown, Interface};

use crate::video::{HdrMetadata, OutputHdrInfo, VideoFormat, VideoTarget};
use crate::{Rect, ShellError};

fn gfx<E: std::fmt::Display>(what: &'static str) -> impl FnOnce(E) -> ShellError {
    move |e| ShellError::Graphics(format!("{what}: {e}"))
}

/// DirectComposition-Gerät samt Wurzel und den beiden Visuals. Alle Aufrufe laufen über die
/// gemeinsame Sperre, weil Video- und UI-Thread Änderungen einreichen.
pub(crate) struct Compositor {
    pub device: IDCompositionDevice,
    _target: IDCompositionTarget,
    pub video_visual: IDCompositionVisual,
    pub ui_visual: IDCompositionVisual,
    pub lock: Arc<Mutex<()>>,
}

impl Compositor {
    pub fn new(hwnd: isize) -> Result<Self, ShellError> {
        // SAFETY: gültiges Fenster-Handle; alle Objekte stammen aus den Aufrufen selbst.
        unsafe {
            let device: IDCompositionDevice = DCompositionCreateDevice2(None::<&IUnknown>)
                .map_err(gfx("DCompositionCreateDevice2"))?;
            let target = device
                .CreateTargetForHwnd(HWND(hwnd as *mut c_void), true)
                .map_err(gfx("CreateTargetForHwnd"))?;
            let root = device.CreateVisual().map_err(gfx("CreateVisual"))?;
            let video_visual = device.CreateVisual().map_err(gfx("CreateVisual"))?;
            let ui_visual = device.CreateVisual().map_err(gfx("CreateVisual"))?;
            root.AddVisual(&video_visual, false, None::<&IDCompositionVisual>)
                .map_err(gfx("AddVisual"))?;
            root.AddVisual(&ui_visual, true, &video_visual)
                .map_err(gfx("AddVisual"))?;
            target.SetRoot(&root).map_err(gfx("SetRoot"))?;
            device.Commit().map_err(gfx("Commit"))?;
            Ok(Self {
                device,
                _target: target,
                video_visual,
                ui_visual,
                lock: Arc::new(Mutex::new(())),
            })
        }
    }
}

/// Wählt den DXGI-Adapter: bevorzugt den mit passender Hersteller-/Geräte-Id (gleiche GPU wie wgpu),
/// sonst den ersten Hardware-Adapter.
fn pick_adapter(
    factory: &IDXGIFactory2,
    prefer: Option<(u32, u32)>,
) -> Result<IDXGIAdapter1, ShellError> {
    let mut first_hw: Option<IDXGIAdapter1> = None;
    let mut i = 0;
    // SAFETY: reine Abfragen auf der Fabrik.
    unsafe {
        while let Ok(a) = factory.EnumAdapters1(i) {
            i += 1;
            let Ok(d) = a.GetDesc1() else { continue };
            if d.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
                continue;
            }
            if prefer == Some((d.VendorId, d.DeviceId)) {
                return Ok(a);
            }
            if first_hw.is_none() {
                first_hw = Some(a);
            }
        }
    }
    first_hw.ok_or_else(|| ShellError::Graphics("kein Hardware-Grafikadapter".into()))
}

pub(crate) struct D3dDevice {
    pub factory: IDXGIFactory2,
    pub device: ID3D11Device,
    pub context: ID3D11DeviceContext,
}

pub(crate) fn create_d3d11(prefer: Option<(u32, u32)>) -> Result<D3dDevice, ShellError> {
    // SAFETY: Initialisierung; Ausgabezeiger zeigen auf lokale Optionen.
    unsafe {
        let factory: IDXGIFactory2 =
            CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0)).map_err(gfx("CreateDXGIFactory2"))?;
        let adapter = pick_adapter(&factory, prefer)?;
        let mut dev: Option<ID3D11Device> = None;
        let mut ctx: Option<ID3D11DeviceContext> = None;
        let levels = [D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0];
        D3D11CreateDevice(
            &adapter,
            D3D_DRIVER_TYPE_UNKNOWN,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
            Some(&levels),
            D3D11_SDK_VERSION,
            Some(&mut dev),
            None,
            Some(&mut ctx),
        )
        .map_err(gfx("D3D11CreateDevice"))?;
        let device = dev.ok_or_else(|| ShellError::Graphics("kein D3D11-Gerät".into()))?;
        let context = ctx.ok_or_else(|| ShellError::Graphics("kein D3D11-Kontext".into()))?;
        let mt: ID3D10Multithread = device.cast().map_err(gfx("ID3D10Multithread"))?;
        // mpv/d3d11va nutzen das Gerät aus mehreren Threads.
        let _ = mt.SetMultithreadProtected(true);
        Ok(D3dDevice {
            factory,
            device,
            context,
        })
    }
}

struct VlState {
    dev: ID3D11Device,
    ctx: ID3D11DeviceContext,
    factory: IDXGIFactory2,
    sc: IDXGISwapChain1,
    visual: IDCompositionVisual,
    format: VideoFormat,
    size: (u32, u32),
    window_px: (u32, u32),
    scale: f64,
    rect: Option<Rect>,
    visible: bool,
    want_format: VideoFormat,
    hdr: Option<HdrMetadata>,
    acquired: Option<ID3D11Texture2D>,
    needs_apply: bool,
}

struct VlInner {
    state: Mutex<VlState>,
    comp_lock: Arc<Mutex<()>>,
    comp_device: IDCompositionDevice,
    dirty: AtomicBool,
}

// SAFETY: Alle COM-Objekte hinter `VlInner` werden nur unter `state` bzw. `comp_lock` benutzt; das
// D3D11-Gerät ist Multithread-geschützt, DirectComposition-Aufrufe sind über `comp_lock` serialisiert.
unsafe impl Send for VlInner {}
unsafe impl Sync for VlInner {}

/// Video-Ebene unter der UI. Klonbar und thread-sicher; der Player ruft sie aus seinem Thread.
#[derive(Clone)]
pub struct VideoLayer {
    inner: Arc<VlInner>,
}

fn create_swapchain(
    factory: &IDXGIFactory2,
    dev: &ID3D11Device,
    format: VideoFormat,
    w: u32,
    h: u32,
) -> Result<IDXGISwapChain1, ShellError> {
    let desc = DXGI_SWAP_CHAIN_DESC1 {
        Width: w,
        Height: h,
        Format: DXGI_FORMAT(format.dxgi_format() as i32),
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
    unsafe {
        factory
            .CreateSwapChainForComposition(dev, &desc, None::<&IDXGIOutput>)
            .map_err(gfx("CreateSwapChainForComposition (Video)"))
    }
}

fn apply_color_space(sc: &IDXGISwapChain1, format: VideoFormat) {
    if let Ok(sc3) = sc.cast::<IDXGISwapChain3>() {
        // SAFETY: Farbraum der eigenen Swapchain.
        unsafe {
            let _ = sc3.SetColorSpace1(DXGI_COLOR_SPACE_TYPE(format.dxgi_color_space()));
        }
    }
}

fn apply_hdr_metadata(sc: &IDXGISwapChain1, md: &HdrMetadata) {
    let u = HdrMetadata::chroma_units;
    let p = &md.primaries;
    let raw = DXGI_HDR_METADATA_HDR10 {
        RedPrimary: [u(p[0].0), u(p[0].1)],
        GreenPrimary: [u(p[1].0), u(p[1].1)],
        BluePrimary: [u(p[2].0), u(p[2].1)],
        WhitePoint: [u(p[3].0), u(p[3].1)],
        MaxMasteringLuminance: md.max_mastering_nits.clamp(0.0, 10000.0) as u32,
        MinMasteringLuminance: (md.min_mastering_nits * 10000.0).clamp(0.0, 65535.0) as u32,
        MaxContentLightLevel: md.max_cll.clamp(0.0, 65535.0) as u16,
        MaxFrameAverageLightLevel: md.max_fall.clamp(0.0, 65535.0) as u16,
    };
    if let Ok(sc4) = sc.cast::<IDXGISwapChain4>() {
        // SAFETY: Zeiger auf lokale Struktur passender Größe.
        unsafe {
            let bytes = std::slice::from_raw_parts(
                (&raw as *const DXGI_HDR_METADATA_HDR10).cast::<u8>(),
                std::mem::size_of::<DXGI_HDR_METADATA_HDR10>(),
            );
            let _ = sc4.SetHDRMetaData(DXGI_HDR_METADATA_TYPE_HDR10, Some(bytes));
        }
    }
}

impl VideoLayer {
    pub(crate) fn new(
        d3d: &D3dDevice,
        comp: &Compositor,
        format: VideoFormat,
        window_px: (u32, u32),
        scale: f64,
    ) -> Result<Self, ShellError> {
        let size = (window_px.0.max(2), window_px.1.max(2));
        let sc = create_swapchain(&d3d.factory, &d3d.device, format, size.0, size.1)?;
        apply_color_space(&sc, format);
        {
            let _g = comp.lock.lock().unwrap_or_else(|p| p.into_inner());
            // SAFETY: eigenes Visual und eigene Swapchain.
            unsafe {
                comp.video_visual
                    .SetContent(&sc)
                    .map_err(gfx("SetContent (Video)"))?;
                comp.device.Commit().map_err(gfx("Commit"))?;
            }
        }
        let st = VlState {
            dev: d3d.device.clone(),
            ctx: d3d.context.clone(),
            factory: d3d.factory.clone(),
            sc,
            visual: comp.video_visual.clone(),
            format,
            size,
            window_px,
            scale,
            rect: None,
            visible: true,
            want_format: format,
            hdr: None,
            acquired: None,
            needs_apply: false,
        };
        Ok(Self {
            inner: Arc::new(VlInner {
                state: Mutex::new(st),
                comp_lock: comp.lock.clone(),
                comp_device: comp.device.clone(),
                dirty: AtomicBool::new(true),
            }),
        })
    }

    fn lock(&self) -> MutexGuard<'_, VlState> {
        self.inner.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Roher `ID3D11Device*` (geborgt, lebt so lange wie der Layer), für `mpv_render_context_create`.
    pub fn d3d11_device_raw(&self) -> *mut c_void {
        self.lock().dev.as_raw()
    }

    /// Leiht den aktuellen Backbuffer aus. Bis `present()` bleibt er gültig; Größen- und
    /// Formatwünsche werden vorher übernommen. `None`, wenn der Puffer nicht verfügbar ist.
    pub fn acquire_target(&self) -> Option<VideoTarget> {
        let mut s = self.lock();
        if s.acquired.is_some() {
            // Doppelter Aufruf: denselben Puffer erneut melden.
        } else {
            self.apply_pending(&mut s);
            // SAFETY: Backbuffer 0 der eigenen Swapchain.
            let tex = unsafe { s.sc.GetBuffer::<ID3D11Texture2D>(0) }.ok()?;
            s.acquired = Some(tex);
        }
        let tex = s.acquired.as_ref()?;
        Some(VideoTarget {
            texture_raw: tex.as_raw(),
            width: s.size.0,
            height: s.size.1,
        })
    }

    /// Zeigt das gefüllte Bild (vsync) und gibt den Backbuffer frei.
    pub fn present(&self) {
        let sc = {
            let mut s = self.lock();
            s.acquired = None;
            s.sc.clone()
        };
        // Present außerhalb der Sperre: es blockiert bis zum nächsten vsync.
        // SAFETY: Present auf der eigenen Swapchain (Gerät ist Multithread-geschützt).
        unsafe {
            let _ = sc.Present(1, DXGI_PRESENT(0));
        }
        let mut s = self.lock();
        if s.needs_apply && s.acquired.is_none() {
            self.apply_pending(&mut s);
        }
    }

    /// Rechteck in logischen Pixeln; `None` = Vollbild. Wirkt sofort, sonst nach dem nächsten Bild.
    pub fn set_rect(&self, rect: Option<Rect>) {
        let mut s = self.lock();
        if s.rect != rect {
            s.rect = rect;
            s.needs_apply = true;
            if s.acquired.is_none() {
                self.apply_pending(&mut s);
            }
        }
    }

    pub fn set_format(&self, format: VideoFormat) {
        let mut s = self.lock();
        if s.want_format != format {
            s.want_format = format;
            s.needs_apply = true;
            if s.acquired.is_none() {
                self.apply_pending(&mut s);
            }
        }
    }

    /// HDR10-Metadaten setzen (wirken nur im PQ-Format; werden nach Formatwechseln erneut gesetzt).
    pub fn set_hdr_metadata(&self, md: HdrMetadata) {
        let mut s = self.lock();
        s.hdr = Some(md);
        if s.format == VideoFormat::Rgb10A2Pq {
            apply_hdr_metadata(&s.sc, &md);
        }
    }

    pub fn set_visible(&self, visible: bool) {
        let mut s = self.lock();
        if s.visible != visible {
            s.visible = visible;
            s.needs_apply = true;
            if s.acquired.is_none() {
                self.apply_pending(&mut s);
            }
        }
    }

    /// Ob sich Größe, Format oder Rechteck seit dem letzten Aufruf geändert haben (neu zeichnen!).
    pub fn take_dirty(&self) -> bool {
        self.inner.dirty.swap(false, Ordering::AcqRel)
    }

    /// HDR-Eigenschaften der Ausgabe, auf der das Fenster liegt (`IDXGIOutput6::GetDesc1`).
    pub fn output_hdr_info(&self) -> OutputHdrInfo {
        let sc = self.lock().sc.clone();
        // SAFETY: reine Abfragen.
        unsafe {
            let Ok(out) = sc.GetContainingOutput() else {
                return OutputHdrInfo::default();
            };
            let Ok(o6) = out.cast::<IDXGIOutput6>() else {
                return OutputHdrInfo::default();
            };
            let Ok(d) = o6.GetDesc1() else {
                return OutputHdrInfo::default();
            };
            OutputHdrInfo {
                hdr_enabled: d.ColorSpace == DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020,
                bits_per_color: d.BitsPerColor,
                min_luminance_nits: d.MinLuminance,
                max_luminance_nits: d.MaxLuminance,
                max_full_frame_nits: d.MaxFullFrameLuminance,
            }
        }
    }

    /// Fenstergröße/Skalierung aus der Shell (nicht für den Player).
    pub(crate) fn set_window(&self, px: (u32, u32), scale: f64) {
        let mut s = self.lock();
        if s.window_px != px || (s.scale - scale).abs() > f64::EPSILON {
            s.window_px = px;
            s.scale = scale;
            s.needs_apply = true;
            if s.acquired.is_none() {
                self.apply_pending(&mut s);
            }
        }
    }

    /// Übernimmt Rechteck, Größe, Format. Nur aufrufen, wenn kein Backbuffer ausgeliehen ist.
    fn apply_pending(&self, s: &mut VlState) {
        s.needs_apply = false;
        let (off, size) = match s.rect {
            Some(r) => {
                let k = s.scale as f32;
                (
                    (r.x * k).round(),
                    (r.y * k).round(),
                    (
                        (r.w * k).round().max(2.0) as u32,
                        (r.h * k).round().max(2.0) as u32,
                    ),
                )
            }
            None => (0.0, 0.0, (s.window_px.0.max(2), s.window_px.1.max(2))),
        }
        .into_parts();
        let format_changed = s.format != s.want_format;
        if size != s.size || format_changed {
            // SAFETY: es ist kein Backbuffer ausgeliehen.
            let r = unsafe {
                s.sc.ResizeBuffers(
                    0,
                    size.0,
                    size.1,
                    if format_changed {
                        DXGI_FORMAT(s.want_format.dxgi_format() as i32)
                    } else {
                        DXGI_FORMAT_UNKNOWN
                    },
                    DXGI_SWAP_CHAIN_FLAG(0),
                )
            };
            if r.is_ok() {
                s.size = size;
                if format_changed {
                    s.format = s.want_format;
                    apply_color_space(&s.sc, s.format);
                    if s.format == VideoFormat::Rgb10A2Pq
                        && let Some(md) = s.hdr
                    {
                        apply_hdr_metadata(&s.sc, &md);
                    }
                }
            } else {
                // Ausgeliehene Referenzen innerhalb von mpv o. Ä.: beim nächsten Bild erneut versuchen.
                s.needs_apply = true;
            }
        }
        let _g = self
            .inner
            .comp_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        // SAFETY: Eigenschaften des eigenen Visuals.
        unsafe {
            let _ = s.visual.SetOffsetX2(off.0);
            let _ = s.visual.SetOffsetY2(off.1);
            let clip = D2D_RECT_F {
                left: 0.0,
                top: 0.0,
                right: if s.visible { s.size.0 as f32 } else { 0.0 },
                bottom: if s.visible { s.size.1 as f32 } else { 0.0 },
            };
            let _ = s.visual.SetClip2(&clip);
            let _ = self.inner.comp_device.Commit();
        }
        let _ = (&s.factory, &s.ctx);
        self.inner.dirty.store(true, Ordering::Release);
    }
}

/// Hilfstyp, damit `apply_pending` Versatz und Größe aus einem Ausdruck entpackt.
trait IntoParts {
    fn into_parts(self) -> ((f32, f32), (u32, u32));
}

impl IntoParts for (f32, f32, (u32, u32)) {
    fn into_parts(self) -> ((f32, f32), (u32, u32)) {
        ((self.0, self.1), self.2)
    }
}
