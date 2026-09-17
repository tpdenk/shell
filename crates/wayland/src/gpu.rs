//! wgpu device, surface and egui renderer bound to one `wl_surface`.

use std::ptr::NonNull;

use anyhow::{Context, Result, anyhow};
use egui_wgpu::{Renderer, RendererOptions, ScreenDescriptor};
use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
};
use smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface;
use smithay_client_toolkit::reexports::client::{Connection, Proxy};

pub(crate) struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    renderer: Renderer,
}

impl Gpu {
    pub(crate) fn new(conn: &Connection, wl_surface: &WlSurface) -> Result<Gpu> {
        let display = NonNull::new(conn.backend().display_ptr().cast())
            .ok_or_else(|| anyhow!("wl_display pointer is null"))?;
        let surface_ptr = NonNull::new(wl_surface.id().as_ptr().cast())
            .ok_or_else(|| anyhow!("wl_surface pointer is null"))?;

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        // SAFETY: both handles point at objects owned by the Wayland connection
        // and layer surface, which `State` keeps alive for as long as `Gpu`.
        let surface = unsafe {
            instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: Some(RawDisplayHandle::Wayland(WaylandDisplayHandle::new(
                    display,
                ))),
                raw_window_handle: RawWindowHandle::Wayland(WaylandWindowHandle::new(surface_ptr)),
            })
        }
        .context("creating the wgpu surface")?;

        // Integrated GPU by default: a bar does not justify waking a dGPU.
        // `WGPU_POWER_PREF=high` overrides at runtime.
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference:
                wgpu::PowerPreference::from_env().unwrap_or(wgpu::PowerPreference::LowPower),
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .context("no Vulkan adapter can present to the surface")?;
        log::info!("using adapter {:?}", adapter.get_info().name);

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("shell"),
            ..Default::default()
        }))
        .context("requesting the wgpu device")?;

        let caps = surface.get_capabilities(&adapter);
        // egui-wgpu wants a non-sRGB format and writes premultiplied alpha.
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .or_else(|| caps.formats.first().copied())
            .ok_or_else(|| anyhow!("surface reports no texture formats"))?;
        let alpha_mode = if caps
            .alpha_modes
            .contains(&wgpu::CompositeAlphaMode::PreMultiplied)
        {
            wgpu::CompositeAlphaMode::PreMultiplied
        } else {
            log::warn!("compositor rejects premultiplied alpha, surface will be opaque");
            wgpu::CompositeAlphaMode::Opaque
        };
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            // Filled in by the first `resize`, before any `render`.
            width: 0,
            height: 0,
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode,
            view_formats: vec![],
        };

        let renderer = Renderer::new(&device, format, RendererOptions::default());

        Ok(Gpu {
            device,
            queue,
            surface,
            config,
            renderer,
        })
    }

    /// Reconfigures the swapchain to `width x height` physical pixels.
    pub(crate) fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        if (self.config.width, self.config.height) == (width, height) {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    pub(crate) fn is_configured(&self) -> bool {
        self.config.width != 0 && self.config.height != 0
    }

    /// Draws one egui frame and presents it. Returns `false` if no frame was
    /// presented (and therefore no `wl_surface.commit` happened).
    pub(crate) fn render(
        &mut self,
        pixels_per_point: f32,
        mut textures_delta: egui::TexturesDelta,
        paint_jobs: &[egui::ClippedPrimitive],
    ) -> bool {
        // Texture uploads do not depend on the swapchain. Apply them first so
        // a failed acquire cannot leave deltas unapplied, which egui treats as
        // a bug (it panics on drop).
        for (id, deltas) in textures_delta.set.drain() {
            for delta in &deltas {
                self.renderer
                    .update_texture(&self.device, &self.queue, id, delta);
            }
        }
        let presented = self.render_frame(pixels_per_point, paint_jobs);
        for id in textures_delta.free.drain() {
            self.renderer.free_texture(&id);
        }
        presented
    }

    fn render_frame(
        &mut self,
        pixels_per_point: f32,
        paint_jobs: &[egui::ClippedPrimitive],
    ) -> bool {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => frame,
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            status
            @ (wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost) => {
                log::debug!("swapchain {status:?}, reconfiguring");
                self.surface.configure(&self.device, &self.config);
                return false;
            }
            status => {
                log::debug!("skipping frame: {status:?}");
                return false;
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let screen = ScreenDescriptor {
            size_in_pixels: [self.config.width, self.config.height],
            pixels_per_point,
        };

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("egui"),
            });
        let user_buffers = self.renderer.update_buffers(
            &self.device,
            &self.queue,
            &mut encoder,
            paint_jobs,
            &screen,
        );
        {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            let mut pass = pass.forget_lifetime();
            self.renderer.render(&mut pass, paint_jobs, &screen);
        }

        self.queue.submit(
            user_buffers
                .into_iter()
                .chain(std::iter::once(encoder.finish())),
        );
        self.queue.present(frame);
        true
    }
}
