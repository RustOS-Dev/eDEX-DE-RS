//! wgpu renderer for eDEX-DE scenes.

mod rects;
mod text;

use anyhow::{anyhow, Context, Result};
use glyphon::{Cache, FontSystem, SwashCache, TextAtlas, TextRenderer, Viewport};
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};
use tracing::{info, warn};
use ui::{layout::Metrics, scene::Scene};

use rects::{RectPipeline, ScanParams, ScanlinePipeline};
pub use text::measure_mono;
use text::TextCache;

/// Shared GPU device, font system and glyph cache; one per process.
pub struct GpuContext {
    pub instance: wgpu::Instance,
    pub adapter: Option<wgpu::Adapter>,
    pub device: Option<wgpu::Device>,
    pub queue: Option<wgpu::Queue>,
    pub font_system: FontSystem,
    pub swash_cache: SwashCache,
    glyph_cache: Option<Cache>,
    font_family: Option<String>,
}

impl GpuContext {
    pub fn new(font_family: Option<String>) -> Self {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN | wgpu::Backends::GL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let font_system = FontSystem::new();
        if let Some(fam) = &font_family {
            let available = font_system.db().faces().any(|f| f.families.iter().any(|(n, _)| n == fam));
            if !available {
                warn!(font = %fam, "configured font not found; falling back to the default monospace");
            }
        }
        Self { instance, adapter: None, device: None, queue: None, font_system, swash_cache: SwashCache::new(), glyph_cache: None, font_family }
    }

    pub fn font_family(&self) -> Option<&str> {
        self.font_family.as_deref()
    }

    pub fn set_font_family(&mut self, family: Option<String>) {
        self.font_family = family;
    }

    /// Adapter description for the About page.
    pub fn adapter_info(&self) -> Option<String> {
        self.adapter.as_ref().map(|a| {
            let i = a.get_info();
            format!("{} ({:?}, {:?})", i.name, i.backend, i.device_type)
        })
    }

    fn ensure_device(&mut self, surface: &wgpu::Surface<'static>) -> Result<()> {
        if self.device.is_some() {
            return Ok(());
        }
        let adapter = pollster::block_on(self.instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: Some(surface),
        }))
        .map_err(|e| anyhow!("no compatible GPU adapter: {e}"))?;
        let info = adapter.get_info();
        info!(name = %info.name, backend = ?info.backend, kind = ?info.device_type, "gpu adapter selected");
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("eDEX-DE device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        }))
        .context("failed to create GPU device")?;
        self.glyph_cache = Some(Cache::new(&device));
        self.adapter = Some(adapter);
        self.device = Some(device);
        self.queue = Some(queue);
        Ok(())
    }

    /// Measure font metrics for the given UI and terminal font sizes.
    pub fn metrics(&mut self, ui_font: f32, term_font: f32) -> Metrics {
        let family = self.font_family.clone();
        let line = (ui_font * 1.45).round();
        let cell_h = (term_font * 1.35).round();
        let (cell_w, _) = measure_mono(&mut self.font_system, family.as_deref(), term_font, cell_h);
        Metrics { ui_font, line, cell_w: (cell_w * 100.0).round() / 100.0, cell_h, term_font }
    }
}

/// Renderer bound to one Wayland surface.
pub struct SurfaceRenderer {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    rects: RectPipeline,
    scanlines: ScanlinePipeline,
    atlas: TextAtlas,
    text_renderer: TextRenderer,
    viewport: Viewport,
    text_cache: TextCache,
    scale: f32,
    frames: u64,
}

impl SurfaceRenderer {
    pub fn new(ctx: &mut GpuContext, handles: (RawDisplayHandle, RawWindowHandle), width: u32, height: u32, scale: f32) -> Result<Self> {
        let surface = unsafe {
            ctx.instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: Some(handles.0),
                raw_window_handle: handles.1,
            })
        }
        .context("failed to create wgpu surface")?;
        ctx.ensure_device(&surface)?;
        let adapter = ctx.adapter.as_ref().expect("adapter");
        let device = ctx.device.as_ref().expect("device");
        let queue = ctx.queue.as_ref().expect("queue");
        let caps = surface.get_capabilities(adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| matches!(f, wgpu::TextureFormat::Bgra8UnormSrgb | wgpu::TextureFormat::Rgba8UnormSrgb))
            .or_else(|| caps.formats.first().copied())
            .ok_or_else(|| anyhow!("surface exposes no texture formats"))?;
        let present_mode = [wgpu::PresentMode::Mailbox, wgpu::PresentMode::Immediate, wgpu::PresentMode::Fifo]
            .into_iter()
            .find(|m| caps.present_modes.contains(m))
            .unwrap_or(wgpu::PresentMode::Fifo);
        let alpha_mode = if caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::PreMultiplied) {
            wgpu::CompositeAlphaMode::PreMultiplied
        } else if caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::PostMultiplied) {
            wgpu::CompositeAlphaMode::PostMultiplied
        } else {
            caps.alpha_modes.first().copied().unwrap_or(wgpu::CompositeAlphaMode::Auto)
        };
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: width.max(1),
            height: height.max(1),
            present_mode,
            desired_maximum_frame_latency: 2,
            alpha_mode,
            view_formats: vec![],
        };
        surface.configure(device, &config);
        info!(?format, ?present_mode, ?alpha_mode, width, height, "surface configured");
        let cache = ctx.glyph_cache.as_ref().expect("glyph cache");
        let viewport = Viewport::new(device, cache);
        let mut atlas = TextAtlas::new(device, queue, cache, format);
        let text_renderer = TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        Ok(Self {
            rects: RectPipeline::new(device, format),
            scanlines: ScanlinePipeline::new(device, format),
            surface,
            config,
            atlas,
            text_renderer,
            viewport,
            text_cache: TextCache::new(ctx.font_family.clone()),
            scale,
            frames: 0,
        })
    }

    pub fn resize(&mut self, ctx: &GpuContext, width: u32, height: u32, scale: f32) {
        if width == 0 || height == 0 {
            return;
        }
        if self.config.width != width || self.config.height != height {
            self.config.width = width;
            self.config.height = height;
            if let Some(device) = &ctx.device {
                self.surface.configure(device, &self.config);
            }
        }
        self.scale = scale;
    }

    pub fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    pub fn frames_presented(&self) -> u64 {
        self.frames
    }

    pub fn format(&self) -> wgpu::TextureFormat {
        self.config.format
    }

    /// Render a scene and present it. Returns `Ok(false)` if the swapchain was lost and
    /// reconfigured (the caller should render again next frame).
    pub fn render(&mut self, ctx: &mut GpuContext, scene: &Scene) -> Result<bool> {
        let device = ctx.device.as_ref().context("no device")?;
        let queue = ctx.queue.as_ref().context("no queue")?;
        self.text_cache.set_family(ctx.font_family.clone());
        let physical = (self.config.width, self.config.height);
        let srgb = self.config.format.is_srgb();

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                self.surface.configure(device, &self.config);
                return Ok(false);
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => return Ok(false),
            wgpu::CurrentSurfaceTexture::Validation => return Err(anyhow!("surface validation error")),
        };
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());

        self.rects.upload(device, queue, scene, self.scale, srgb);

        self.text_cache.begin_frame();
        let keys: Vec<u64> = scene.texts.iter().map(|t| self.text_cache.prepare(&mut ctx.font_system, t)).collect();
        self.viewport.update(queue, glyphon::Resolution { width: physical.0, height: physical.1 });
        {
            let areas = scene
                .texts
                .iter()
                .zip(keys.iter())
                .filter_map(|(spec, key)| self.text_cache.area(*key, spec, self.scale, physical));
            if let Err(e) = self.text_renderer.prepare(device, queue, &mut ctx.font_system, &mut self.atlas, &self.viewport, areas, &mut ctx.swash_cache) {
                warn!("text prepare failed: {e:?}");
            }
        }
        self.text_cache.end_frame(240);

        if let Some(s) = scene.scanlines {
            self.scanlines.update(queue, ScanParams { size: [physical.0 as f32, physical.1 as f32], scale: self.scale, intensity: s.intensity, color: s.color });
        }

        let clear = to_linear_if(scene.clear, srgb);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("edex frame") });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("edex pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: clear[0] as f64, g: clear[1] as f64, b: clear[2] as f64, a: clear[3] as f64 }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.rects.draw(&mut pass);
            if let Err(e) = self.text_renderer.render(&self.atlas, &self.viewport, &mut pass) {
                warn!("text render failed: {e:?}");
            }
            if scene.scanlines.is_some() {
                self.scanlines.draw(&mut pass);
            }
        }
        queue.submit(std::iter::once(encoder.finish()));
        frame.present();
        self.atlas.trim();
        self.frames += 1;
        Ok(true)
    }

    pub fn text_cache_len(&self) -> usize {
        self.text_cache.len()
    }

    pub fn rect_capacity(&self) -> usize {
        self.rects.instance_capacity()
    }
}

fn to_linear_if(c: [f32; 4], srgb: bool) -> [f32; 4] {
    if srgb {
        [c[0].powf(2.2), c[1].powf(2.2), c[2].powf(2.2), c[3]]
    } else {
        c
    }
}
