//! Render-seitige Videoausgabe: mpv zeichnet in eine von der App gelieferte Textur bzw. einen FBO.
//!
//! Nur Rohzeiger und Rückrufe, keine Abhängigkeit auf Fenster- oder UI-Code. Die App (später
//! `lfx-shell`) liefert das D3D11-Gerät und die Zieltextur bzw. den GL-Kontext und präsentiert
//! selbst. mpv ändert den Zustand des Immediate-Contexts, ohne ihn zurückzusetzen.

use std::ffi::{CStr, c_char, c_int, c_void};
use std::ptr;
use std::sync::Arc;

use lfx_mpv_sys::ffi::{
    MPV_RENDER_API_TYPE_OPENGL, MPV_RENDER_PARAM_D3D11_FBO, MPV_RENDER_PARAM_FLIP_Y,
    MPV_RENDER_PARAM_OPENGL_FBO, MPV_RENDER_PARAM_OPENGL_INIT_PARAMS, MPV_RENDER_UPDATE_FRAME,
    mpv_d3d11_fbo, mpv_opengl_fbo, mpv_opengl_init_params, mpv_render_param,
};
use lfx_mpv_sys::{Mpv, RenderContext};

use crate::player::Player;
use crate::state::PlayerError;

/// Wird aus mpv-Threads gerufen, wenn ein neues Bild bereitsteht. Darf weder blockieren noch
/// mpv aufrufen; nur ein Signal (z. B. Flag setzen, Render-Thread wecken).
pub type FrameReady = Arc<dyn Fn() + Send + Sync + 'static>;

/// Ziel eines Render-Aufrufs.
#[derive(Debug, Clone, Copy)]
pub enum RenderTarget {
    /// `ID3D11Texture2D*` (DEFAULT-Nutzung, Render-Target-Bindung, kein MSAA/Array/Mip, kein sRGB;
    /// erlaubt: RGBA16F, RGB10A2, BGRA8, RGBA8). Größe nur zur Information.
    D3d11 {
        texture: *mut c_void,
        width: u32,
        height: u32,
    },
    /// OpenGL-Framebuffer (0 = Standard-Framebuffer des aktuellen Kontexts).
    OpenGl {
        fbo: i32,
        width: u32,
        height: u32,
        /// Bild an der Horizontalen spiegeln (nötig beim Standard-Framebuffer).
        flip_y: bool,
    },
}

/// Videoausgabe, die ein neues Bild in ein Ziel zeichnet.
pub trait VideoOutput {
    /// Gibt es ein neues Bild? Ruft `mpv_render_context_update`; nach einem Frame-Signal aufrufen.
    fn update(&mut self) -> bool;
    /// Zeichnet das aktuelle Bild in `target`. Der Zieltyp muss zur Implementierung passen.
    fn render(&mut self, target: RenderTarget) -> Result<(), PlayerError>;
    /// Nach dem Präsentieren melden (hilft mpv beim Bildtakt).
    fn report_swap(&mut self);
}

fn to_err(e: lfx_mpv_sys::MpvError) -> PlayerError {
    PlayerError {
        code: e.code,
        text: e.text,
    }
}

/// Rückruf-Box für den Update-Callback von mpv.
struct Signal {
    cb: *mut FrameReady,
}

unsafe extern "C" fn update_trampoline(ctx: *mut c_void) {
    // SAFETY: `ctx` ist der Zeiger auf die Box, die bis nach dem Entfernen des Callbacks lebt.
    let cb = unsafe { &*(ctx as *const FrameReady) };
    cb();
}

impl Signal {
    fn install(ctx: &RenderContext, cb: FrameReady) -> Self {
        let raw = Box::into_raw(Box::new(cb));
        // SAFETY: Der Callback blockiert nicht und ruft mpv nicht; `raw` lebt bis `Drop`.
        unsafe { ctx.set_update_callback(Some(update_trampoline), raw.cast()) };
        Self { cb: raw }
    }

    fn remove(&mut self, ctx: &RenderContext) {
        if self.cb.is_null() {
            return;
        }
        // SAFETY: Callback zuerst entfernen, dann die Box freigeben.
        unsafe {
            ctx.set_update_callback(None, ptr::null_mut());
            drop(Box::from_raw(self.cb));
        }
        self.cb = ptr::null_mut();
    }
}

/// mpv-Render-API `d3d11`: mpv rendert in eine von außen gegebene Textur.
pub struct D3d11Output {
    signal: Signal,
    // Reihenfolge: Kontext vor dem mpv-Handle freigeben.
    ctx: RenderContext,
    _mpv: Arc<Mpv>,
}

// SAFETY: Render-Aufrufe erfolgen nur über `&mut self` (ein Render-Thread zugleich).
unsafe impl Send for D3d11Output {}

impl D3d11Output {
    /// Erzeugt den Render-Kontext.
    ///
    /// Nur unter Windows verfügbar; anderswo meldet mpv einen Fehler (kein `d3d11`-Typ).
    ///
    /// # Safety
    /// `device` muss ein gültiger `ID3D11Device*` mit aktivem Multithread-Schutz sein
    /// (`ID3D10Multithread::SetMultithreadProtected(TRUE)`, nötig für d3d11va) und mindestens
    /// so lange leben wie die Ausgabe. Der Player muss mit `VideoMode::Render` laufen.
    pub unsafe fn new(
        player: &Player,
        device: *mut c_void,
        frame_ready: FrameReady,
    ) -> Result<Self, PlayerError> {
        let mpv = player.mpv();
        // SAFETY: Vertrag der Aufrufer-Funktion.
        let ctx = unsafe { RenderContext::create_d3d11(&mpv, device) }.map_err(to_err)?;
        let signal = Signal::install(&ctx, frame_ready);
        Ok(Self {
            signal,
            ctx,
            _mpv: mpv,
        })
    }
}

impl VideoOutput for D3d11Output {
    fn update(&mut self) -> bool {
        self.ctx.update() & MPV_RENDER_UPDATE_FRAME != 0
    }

    fn render(&mut self, target: RenderTarget) -> Result<(), PlayerError> {
        let RenderTarget::D3d11 { texture, .. } = target else {
            return Err(PlayerError {
                code: 0,
                text: "Zieltyp passt nicht zur D3D11-Ausgabe".into(),
            });
        };
        let mut fbo = mpv_d3d11_fbo { tex: texture };
        let params = [mpv_render_param {
            type_: MPV_RENDER_PARAM_D3D11_FBO,
            data: ptr::from_mut(&mut fbo).cast(),
        }];
        // SAFETY: `texture` ist laut Vertrag des Aufrufers eine gültige Zieltextur.
        unsafe { self.ctx.render(&params) }.map_err(to_err)
    }

    fn report_swap(&mut self) {
        self.ctx.report_swap();
    }
}

impl Drop for D3d11Output {
    fn drop(&mut self) {
        self.signal.remove(&self.ctx);
    }
}

/// Adresse einer GL-Funktion nachschlagen (vom Fenstersystem, z. B. `wglGetProcAddress`).
pub type GetProcAddress = Box<dyn Fn(&str) -> *const c_void + Send + 'static>;

unsafe extern "C" fn gl_proc_trampoline(ctx: *mut c_void, name: *const c_char) -> *mut c_void {
    // SAFETY: `ctx` zeigt auf die Box in `GlOutput`; `name` ist ein C-String von mpv.
    let f = unsafe { &*(ctx as *const GetProcAddress) };
    let name = unsafe { CStr::from_ptr(name) }.to_string_lossy();
    f(&name) as *mut c_void
}

/// mpv-Render-API `opengl` (macOS, Rückfall).
pub struct GlOutput {
    signal: Signal,
    ctx: RenderContext,
    // Die Adressfunktion muss so lange leben wie der Kontext (mpv ruft sie auch später).
    _proc: Box<GetProcAddress>,
    _mpv: Arc<Mpv>,
}

// SAFETY: wie bei `D3d11Output`; der GL-Kontext gehört dem Render-Thread des Aufrufers.
unsafe impl Send for GlOutput {}

impl GlOutput {
    /// Erzeugt den Render-Kontext.
    ///
    /// # Safety
    /// Muss mit aktuellem OpenGL-Kontext des Render-Threads aufgerufen werden; `render` und
    /// `drop` ebenfalls in diesem Kontext.
    pub unsafe fn new(
        player: &Player,
        get_proc_address: GetProcAddress,
        frame_ready: FrameReady,
    ) -> Result<Self, PlayerError> {
        let mpv = player.mpv();
        let proc = Box::new(get_proc_address);
        let mut init = mpv_opengl_init_params {
            get_proc_address: Some(gl_proc_trampoline),
            get_proc_address_ctx: ptr::from_ref::<GetProcAddress>(&*proc).cast_mut().cast(),
        };
        let extra = [mpv_render_param {
            type_: MPV_RENDER_PARAM_OPENGL_INIT_PARAMS,
            data: ptr::from_mut(&mut init).cast(),
        }];
        // SAFETY: Parameter gültig, GL-Kontext laut Vertrag aktuell.
        let ctx = unsafe { RenderContext::create(&mpv, MPV_RENDER_API_TYPE_OPENGL, &extra) }
            .map_err(to_err)?;
        let signal = Signal::install(&ctx, frame_ready);
        Ok(Self {
            signal,
            ctx,
            _proc: proc,
            _mpv: mpv,
        })
    }
}

impl VideoOutput for GlOutput {
    fn update(&mut self) -> bool {
        self.ctx.update() & MPV_RENDER_UPDATE_FRAME != 0
    }

    fn render(&mut self, target: RenderTarget) -> Result<(), PlayerError> {
        let RenderTarget::OpenGl {
            fbo,
            width,
            height,
            flip_y,
        } = target
        else {
            return Err(PlayerError {
                code: 0,
                text: "Zieltyp passt nicht zur OpenGL-Ausgabe".into(),
            });
        };
        let mut f = mpv_opengl_fbo {
            fbo,
            w: width as c_int,
            h: height as c_int,
            internal_format: 0,
        };
        let mut flip: c_int = flip_y.into();
        let params = [
            mpv_render_param {
                type_: MPV_RENDER_PARAM_OPENGL_FBO,
                data: ptr::from_mut(&mut f).cast(),
            },
            mpv_render_param {
                type_: MPV_RENDER_PARAM_FLIP_Y,
                data: ptr::from_mut(&mut flip).cast(),
            },
        ];
        // SAFETY: Parameter zeigen auf lebende Stack-Werte; GL-Kontext laut Vertrag aktuell.
        unsafe { self.ctx.render(&params) }.map_err(to_err)
    }

    fn report_swap(&mut self) {
        self.ctx.report_swap();
    }
}

impl Drop for GlOutput {
    fn drop(&mut self) {
        self.signal.remove(&self.ctx);
    }
}
