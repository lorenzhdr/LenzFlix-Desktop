//! UI-Ebene auf der GPU: femtovg (wgpu, DX12) rendert in die Texturen einer Composition-Swapchain,
//! die wgpu per `SurfaceTargetUnsafe::CompositionVisual` an das UI-Visual hängt (premultiplied alpha).

use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use slint::platform::femtovg_renderer::FemtoVGWGPURenderer;
use slint::wgpu_30::wgpu;

use super::compose::Compositor;
use crate::ShellError;

/// wgpu-Futures sind auf nativen Plattformen sofort fertig; mehr als ein Abfragen braucht es nicht.
pub(crate) fn block_on<F: Future>(fut: F) -> F::Output {
    let mut fut = pin!(fut);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(v) = fut.as_mut().poll(&mut cx) {
            return v;
        }
        std::thread::yield_now();
    }
}

/// Instanz, Adapter, Gerät und Queue (noch ohne Oberfläche).
pub(crate) struct GpuCore {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

impl GpuCore {
    pub fn new() -> Result<Self, ShellError> {
        let mut backend_options = wgpu::BackendOptions::from_env_or_default();
        backend_options.dx12.presentation_system = wgpu::Dx12SwapchainKind::DxgiFromVisual;
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle_from_env();
        desc.backends = wgpu::Backends::DX12;
        desc.backend_options = backend_options;
        let instance = wgpu::Instance::new(desc);
        let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
            apply_limit_buckets: false,
        }))
        .map_err(|e| ShellError::Graphics(format!("wgpu-Adapter: {e}")))?;
        let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("lfx-shell-ui"),
            required_features: adapter.features() - wgpu::Features::all_experimental_mask(),
            required_limits: adapter.limits(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            trace: wgpu::Trace::default(),
        }))
        .map_err(|e| ShellError::Graphics(format!("wgpu-Gerät: {e}")))?;
        Ok(Self {
            instance,
            adapter,
            device,
            queue,
        })
    }

    /// Hersteller- und Geräte-Id, damit der D3D11-Video-Pfad dieselbe GPU nimmt.
    pub fn adapter_ids(&self) -> (u32, u32) {
        let i = self.adapter.get_info();
        (i.vendor, i.device)
    }

    pub fn adapter_name(&self) -> String {
        self.adapter.get_info().name
    }
}

/// Ergebnis eines Zeichenversuchs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DrawResult {
    Presented,
    Skipped,
}

pub(crate) struct GpuUi {
    /// Zeiten des letzten Bildes in ms: Textur holen, Slint rendern, präsentieren.
    pub last_ms: (f32, f32, f32),
    core: GpuCore,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
}

impl GpuUi {
    pub fn new(
        core: GpuCore,
        comp: &Compositor,
        size: (u32, u32),
    ) -> Result<(Self, FemtoVGWGPURenderer), ShellError> {
        use windows::core::Interface;
        let visual = comp.ui_visual.as_raw();
        // SAFETY: `visual` ist ein gültiges `IDCompositionVisual`; wgpu zählt den Verweis selbst hoch.
        let surface = unsafe {
            core.instance
                .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::CompositionVisual(visual))
        }
        .map_err(|e| ShellError::Graphics(format!("CompositionVisual-Oberfläche: {e}")))?;
        let caps = surface.get_capabilities(&core.adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| {
                matches!(
                    f,
                    wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm
                )
            })
            .ok_or_else(|| ShellError::Graphics("kein 8-Bit-Oberflächenformat".into()))?;
        if !caps
            .alpha_modes
            .contains(&wgpu::CompositeAlphaMode::PreMultiplied)
        {
            return Err(ShellError::Graphics(
                "Oberfläche bietet kein premultiplied alpha".into(),
            ));
        }
        let mut usage = wgpu::TextureUsages::RENDER_ATTACHMENT;
        if caps.usages.contains(wgpu::TextureUsages::COPY_SRC) {
            usage |= wgpu::TextureUsages::COPY_SRC;
        }
        let config = wgpu::SurfaceConfiguration {
            usage,
            format,
            width: size.0.max(1),
            height: size.1.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: wgpu::CompositeAlphaMode::PreMultiplied,
            color_space: wgpu::SurfaceColorSpace::default(),
            view_formats: vec![],
        };
        {
            let _g = comp.lock.lock().unwrap_or_else(|p| p.into_inner());
            surface.configure(&core.device, &config);
            // SAFETY: Commit auf dem eigenen Gerät.
            unsafe {
                let _ = comp.device.Commit();
            }
        }
        let renderer = FemtoVGWGPURenderer::new(
            core.instance.clone(),
            core.device.clone(),
            core.queue.clone(),
        )
        .map_err(|e| ShellError::Graphics(format!("femtovg-wgpu: {e}")))?;
        Ok((
            Self {
                last_ms: (0.0, 0.0, 0.0),
                core,
                surface,
                config,
            },
            renderer,
        ))
    }

    pub fn resize(&mut self, comp: &Compositor, size: (u32, u32)) {
        let (w, h) = (size.0.max(1), size.1.max(1));
        if (w, h) == (self.config.width, self.config.height) {
            return;
        }
        self.config.width = w;
        self.config.height = h;
        let _g = comp.lock.lock().unwrap_or_else(|p| p.into_inner());
        self.surface.configure(&self.core.device, &self.config);
        // SAFETY: Commit auf dem eigenen Gerät.
        unsafe {
            let _ = comp.device.Commit();
        }
    }

    /// Zeichnet ein Bild: Textur holen, Slint rendern lassen, präsentieren.
    pub fn draw(
        &mut self,
        renderer: &FemtoVGWGPURenderer,
        comp: &Compositor,
    ) -> Result<DrawResult, ShellError> {
        let t0 = std::time::Instant::now();
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) => t,
            wgpu::CurrentSurfaceTexture::Suboptimal(t) => {
                drop(t);
                self.reconfigure(comp);
                match self.surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(t) => t,
                    _ => return Ok(DrawResult::Skipped),
                }
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.reconfigure(comp);
                match self.surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(t) => t,
                    _ => return Ok(DrawResult::Skipped),
                }
            }
            wgpu::CurrentSurfaceTexture::Occluded | wgpu::CurrentSurfaceTexture::Timeout => {
                return Ok(DrawResult::Skipped);
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err(ShellError::Graphics(
                    "wgpu: get_current_texture ungültig".into(),
                ));
            }
        };
        let t1 = std::time::Instant::now();
        renderer
            .render_to_texture(&frame.texture)
            .map_err(|e| ShellError::Graphics(format!("Slint-Render: {e}")))?;
        let t2 = std::time::Instant::now();
        self.core.queue.present(frame);
        let t3 = std::time::Instant::now();
        let ms = |a: std::time::Instant, b: std::time::Instant| (b - a).as_secs_f32() * 1000.0;
        self.last_ms = (ms(t0, t1), ms(t1, t2), ms(t2, t3));
        Ok(DrawResult::Presented)
    }

    fn reconfigure(&self, comp: &Compositor) {
        let _g = comp.lock.lock().unwrap_or_else(|p| p.into_inner());
        self.surface.configure(&self.core.device, &self.config);
        // SAFETY: Commit auf dem eigenen Gerät.
        unsafe {
            let _ = comp.device.Commit();
        }
    }
}
