//! Rückfall-UI: Slint-Software-Renderer mit Dirty-Rects. Die geänderten Bereiche gehen in eine
//! dauerhafte D3D11-Textur, pro Bild kopiert die GPU diese in den Backbuffer einer
//! Composition-Swapchain (premultiplied), die das UI-Visual zeigt.

use std::ffi::c_void;

use slint::platform::software_renderer::{
    PremultipliedRgbaColor, RepaintBufferType, SoftwareRenderer,
};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;

use super::compose::{Compositor, D3dDevice};
use super::gpu::DrawResult;
use crate::ShellError;

pub(crate) struct SoftUi {
    dev: ID3D11Device,
    ctx: ID3D11DeviceContext,
    sc: IDXGISwapChain1,
    tex: ID3D11Texture2D,
    buf: Vec<PremultipliedRgbaColor>,
    size: (u32, u32),
    full_upload: bool,
}

fn make_swapchain(d3d: &D3dDevice, w: u32, h: u32) -> Result<IDXGISwapChain1, ShellError> {
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
    // SAFETY: gültiges Gerät und gültige Beschreibung.
    unsafe {
        d3d.factory
            .CreateSwapChainForComposition(&d3d.device, &desc, None::<&IDXGIOutput>)
            .map_err(|e| ShellError::Graphics(format!("UI-Swapchain: {e}")))
    }
}

fn make_texture(dev: &ID3D11Device, w: u32, h: u32) -> Result<ID3D11Texture2D, ShellError> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: w,
        Height: h,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: 0,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let mut t: Option<ID3D11Texture2D> = None;
    // SAFETY: gültige Beschreibung, Ausgabezeiger gültig.
    unsafe {
        dev.CreateTexture2D(&desc, None, Some(&mut t))
            .map_err(|e| ShellError::Graphics(format!("UI-Textur: {e}")))?;
    }
    t.ok_or_else(|| ShellError::Graphics("UI-Textur fehlt".into()))
}

impl SoftUi {
    pub fn new(
        d3d: &D3dDevice,
        comp: &Compositor,
        size: (u32, u32),
    ) -> Result<(Self, SoftwareRenderer), ShellError> {
        let (w, h) = (size.0.max(1), size.1.max(1));
        let sc = make_swapchain(d3d, w, h)?;
        {
            let _g = comp.lock.lock().unwrap_or_else(|p| p.into_inner());
            // SAFETY: eigenes Visual und eigene Swapchain.
            unsafe {
                comp.ui_visual
                    .SetContent(&sc)
                    .map_err(|e| ShellError::Graphics(format!("SetContent (UI): {e}")))?;
                let _ = comp.device.Commit();
            }
        }
        let renderer =
            SoftwareRenderer::new_with_repaint_buffer_type(RepaintBufferType::ReusedBuffer);
        let ui = Self {
            dev: d3d.device.clone(),
            ctx: d3d.context.clone(),
            sc,
            tex: make_texture(&d3d.device, w, h)?,
            buf: vec![PremultipliedRgbaColor::default(); (w * h) as usize],
            size: (w, h),
            full_upload: true,
        };
        Ok((ui, renderer))
    }

    pub fn resize(&mut self, renderer: &SoftwareRenderer, size: (u32, u32)) {
        let (w, h) = (size.0.max(1), size.1.max(1));
        if (w, h) == self.size {
            return;
        }
        // SAFETY: kein Backbuffer gehalten (er wird pro Bild geholt und gleich wieder freigegeben).
        let r = unsafe {
            self.sc
                .ResizeBuffers(0, w, h, DXGI_FORMAT_UNKNOWN, DXGI_SWAP_CHAIN_FLAG(0))
        };
        if r.is_err() {
            return;
        }
        if let Ok(t) = make_texture(&self.dev, w, h) {
            self.tex = t;
        }
        self.buf = vec![PremultipliedRgbaColor::default(); (w * h) as usize];
        self.size = (w, h);
        self.full_upload = true;
        renderer.set_repaint_buffer_type(RepaintBufferType::NewBuffer);
        renderer.set_repaint_buffer_type(RepaintBufferType::ReusedBuffer);
    }

    pub fn draw(&mut self, renderer: &SoftwareRenderer) -> Result<DrawResult, ShellError> {
        let (w, h) = self.size;
        let region = renderer.render(&mut self.buf, w as usize);
        let stride = w * 4;
        // SAFETY: der Puffer ist w*h*4 Byte groß; jede Box liegt innerhalb der Textur.
        unsafe {
            if self.full_upload {
                self.full_upload = false;
                self.ctx.UpdateSubresource(
                    &self.tex,
                    0,
                    None,
                    self.buf.as_ptr().cast::<c_void>(),
                    stride,
                    0,
                );
            } else {
                for (pos, sz) in region.iter() {
                    let (x, y) = (pos.x.max(0) as u32, pos.y.max(0) as u32);
                    let (bw, bh) = (sz.width.min(w - x), sz.height.min(h - y));
                    if bw == 0 || bh == 0 {
                        continue;
                    }
                    let b = D3D11_BOX {
                        left: x,
                        top: y,
                        front: 0,
                        right: x + bw,
                        bottom: y + bh,
                        back: 1,
                    };
                    let src = self.buf.as_ptr().add((y * w + x) as usize).cast::<c_void>();
                    self.ctx
                        .UpdateSubresource(&self.tex, 0, Some(&b), src, stride, 0);
                }
            }
            let back = self
                .sc
                .GetBuffer::<ID3D11Texture2D>(0)
                .map_err(|e| ShellError::Graphics(format!("UI-Backbuffer: {e}")))?;
            self.ctx.CopyResource(&back, &self.tex);
            let _ = self.sc.Present(1, DXGI_PRESENT(0));
        }
        Ok(DrawResult::Presented)
    }
}
