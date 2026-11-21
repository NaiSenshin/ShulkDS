//! Splitscreen and secondary-display composition: several players' renderers draw offscreen on one
//! device, or a secondary dual-screen HUD surface (`DualWindow` / Android `Presentation`) draws
//! directly on the primary renderer's `wgpu::Device` with its own swapchain format and depth target.
use super::Renderer;
use crate::Result;
use std::sync::Arc;
use winit::window::Window;

pub struct Compositor {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    present_modes: Vec<wgpu::PresentMode>,
    depth: Option<wgpu::TextureView>,
    hud_view_format: Option<wgpu::TextureFormat>,
    suspended: bool,
}

/// Choose a surface configuration format and `view_formats` list so a `TextureView` of `desired`
/// format (the `HudRenderer` pipeline's target format) can be created on the swapchain frames.
pub fn select_hud_surface_format(
    desired: wgpu::TextureFormat,
    offered: &[wgpu::TextureFormat],
) -> Result<(wgpu::TextureFormat, Vec<wgpu::TextureFormat>)> {
    if offered.contains(&desired) {
        return Ok((desired, Vec::new()));
    }
    let desired_base = desired.remove_srgb_suffix();
    if let Some(&alt) = offered
        .iter()
        .find(|fmt| fmt.remove_srgb_suffix() == desired_base)
    {
        return Ok((alt, vec![desired]));
    }
    Err(
        format!("HUD surface needs a {desired:?}-compatible format; this one offers {offered:?}")
            .into(),
    )
}

pub(crate) fn hud_surface_drawable(size: [u32; 2], max_dim: u32) -> bool {
    super::drawable_size(size[0], size[1], max_dim)
}

pub(crate) fn make_compositor_depth_texture(
    device: &wgpu::Device,
    size: [u32; 2],
) -> wgpu::Texture {
    let max = device.limits().max_texture_dimension_2d;
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Compositor HUD depth"),
        size: wgpu::Extent3d {
            width: size[0].clamp(1, max),
            height: size[1].clamp(1, max),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    })
}

impl Renderer {
    /// This renderer's `wgpu::Instance`, used to attach secondary display surfaces on the same backend.
    pub fn instance(&self) -> &wgpu::Instance {
        &self.instance
    }

    /// A compositor for `window` on this renderer's device, presenting on the surface it gave up.
    pub fn compositor(&self, window: Arc<Window>) -> Result<Compositor> {
        // The surface this renderer set aside for its successor (`release_surface`), else a new
        // one: a window can't keep two.
        let spare = self
            .spare_surface
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let surface = match spare {
            Some(surface) => surface,
            None => self.instance.create_surface(window.clone())?,
        };
        Compositor::new(
            surface,
            &self.adapter,
            &self.device,
            &self.queue,
            window.inner_size().into(),
        )
    }

    /// A HUD-capable secondary surface compositor for a desktop `Window`.
    pub fn hud_compositor(&self, window: Arc<Window>) -> Result<Compositor> {
        let size = window.inner_size().into();
        let surface = self.instance.create_surface(window)?;
        self.hud_compositor_from_surface(surface, size)
    }

    /// A HUD-capable secondary surface compositor for an arbitrary `wgpu::Surface<'static>`
    /// (such as an Android `Presentation` `ANativeWindow` surface).
    pub fn hud_compositor_from_surface(
        &self,
        surface: wgpu::Surface<'static>,
        size: [u32; 2],
    ) -> Result<Compositor> {
        Compositor::new_for_hud(
            surface,
            &self.adapter,
            &self.device,
            &self.queue,
            self.config.format.remove_srgb_suffix(),
            size,
        )
    }

    /// The last offscreen frame (`--offscreen`), for the compositor.
    pub fn offscreen_frame(&self) -> Option<&wgpu::Texture> {
        self.offscreen.as_ref()
    }

    /// Splitscreen turned off: a compositor's surface, set aside for the renderer that draws on
    /// the window again (it takes it, as after `release_surface`).
    pub fn take_back(&self, compositor: Compositor) {
        *self
            .spare_surface
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(compositor.surface);
    }
}

impl Compositor {
    /// Present offscreen BGRA frames on a compatible surface using their existing device.
    /// The surface must support COPY_DST and Bgra8Unorm; unsupported surfaces return an error.
    pub fn new(
        surface: wgpu::Surface<'static>,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        size: [u32; 2],
    ) -> Result<Self> {
        let caps = surface.get_capabilities(adapter);
        // ponytail: a plain texture copy needs the offscreen frames' format; a blit pass would
        // take any surface format (Rgba8Unorm on some Linux drivers).
        if !caps.usages.contains(wgpu::TextureUsages::COPY_DST) {
            return Err("surface does not support copying offscreen frames".into());
        }
        let format = wgpu::TextureFormat::Bgra8Unorm;
        if !caps.formats.contains(&format) {
            return Err(format!(
                "splitscreen needs a {format:?} surface; this one offers {:?}",
                caps.formats
            )
            .into());
        }
        let max = device.limits().max_texture_dimension_2d;
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_DST,
            format,
            width: size[0].clamp(1, max),
            height: size[1].clamp(1, max),
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: wgpu::CompositeAlphaMode::Opaque,
            view_formats: vec![],
            color_space: wgpu::SurfaceColorSpace::default(),
        };
        surface.configure(device, &config);
        Ok(Compositor {
            surface,
            device: device.clone(),
            queue: queue.clone(),
            config,
            present_modes: caps.present_modes,
            depth: None,
            hud_view_format: None,
            suspended: !hud_surface_drawable(size, max),
        })
    }

    /// Create a secondary HUD surface compositor compatible with `desired_format` (`Rgba8Unorm` on
    /// Android Vulkan/GLES, `Bgra8Unorm`/`Rgba8Unorm` on desktop) and owning a dedicated
    /// `Depth32Float` depth view matched to `size`.
    pub fn new_for_hud(
        surface: wgpu::Surface<'static>,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        desired_format: wgpu::TextureFormat,
        size: [u32; 2],
    ) -> Result<Self> {
        let caps = surface.get_capabilities(adapter);
        if !caps.usages.contains(wgpu::TextureUsages::RENDER_ATTACHMENT) {
            return Err("HUD surface does not support RENDER_ATTACHMENT".into());
        }
        let (format, view_formats) = select_hud_surface_format(desired_format, &caps.formats)?;
        let alpha_mode = if caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::Opaque) {
            wgpu::CompositeAlphaMode::Opaque
        } else {
            caps.alpha_modes
                .first()
                .copied()
                .unwrap_or(wgpu::CompositeAlphaMode::Auto)
        };
        let max = device.limits().max_texture_dimension_2d;
        let suspended = !hud_surface_drawable(size, max);
        let present_mode = [wgpu::PresentMode::Mailbox, wgpu::PresentMode::Immediate]
            .into_iter()
            .find(|m| caps.present_modes.contains(m))
            .unwrap_or(wgpu::PresentMode::Fifo);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size[0].clamp(1, max),
            height: size[1].clamp(1, max),
            present_mode,
            desired_maximum_frame_latency: 2,
            alpha_mode,
            view_formats,
            color_space: wgpu::SurfaceColorSpace::default(),
        };
        if !suspended {
            surface.configure(device, &config);
        }
        let depth = make_compositor_depth_texture(device, [config.width, config.height])
            .create_view(&Default::default());
        Ok(Compositor {
            surface,
            device: device.clone(),
            queue: queue.clone(),
            config,
            present_modes: caps.present_modes,
            depth: Some(depth),
            hud_view_format: Some(desired_format),
            suspended,
        })
    }

    /// True if this compositor belongs to `device` (false after a renderer device rebuild).
    pub fn same_device(&self, device: &wgpu::Device) -> bool {
        self.device == *device
    }

    /// Vsync on: presents wait for the display. Off: they don't (Mailbox, else Immediate).
    pub fn set_vsync(&mut self, vsync: bool) {
        let mode = [wgpu::PresentMode::Mailbox, wgpu::PresentMode::Immediate]
            .into_iter()
            .find(|m| !vsync && self.present_modes.contains(m))
            .unwrap_or(wgpu::PresentMode::Fifo);
        if mode != self.config.present_mode {
            self.config.present_mode = mode;
            if !self.suspended {
                self.surface.configure(&self.device, &self.config);
            }
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        let max = self.device.limits().max_texture_dimension_2d;
        let was_suspended = self.suspended;
        self.suspended = !hud_surface_drawable([width, height], max);
        if !self.suspended {
            let changed =
                was_suspended || self.config.width != width || self.config.height != height;
            self.config.width = width;
            self.config.height = height;
            if changed {
                self.surface.configure(&self.device, &self.config);
                if self.hud_view_format.is_some() || self.depth.is_some() {
                    self.depth = Some(
                        make_compositor_depth_texture(&self.device, [width, height])
                            .create_view(&Default::default()),
                    );
                }
            }
        }
    }

    #[inline]
    pub fn size(&self) -> [u32; 2] {
        [self.config.width, self.config.height]
    }

    /// Acquire the current surface frame, call `render_fn(&target_view, [width, height])`, and present.
    pub fn present_surface<F>(&mut self, render_fn: F) -> bool
    where
        F: FnOnce(&wgpu::TextureView, [u32; 2]),
    {
        if self.suspended {
            return false;
        }
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f)
            | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return false;
            }
            _ => return false,
        };
        let target = frame.texture.create_view(&wgpu::TextureViewDescriptor {
            format: self.hud_view_format,
            ..Default::default()
        });
        render_fn(&target, [self.config.width, self.config.height]);
        self.queue.present(frame);
        true
    }

    /// Acquire the current HUD surface frame with a matching `Depth32Float` depth view, invoke
    /// `render_fn(&target_view, &depth_view, [width, height])`, and present.
    pub fn present_hud_surface<F>(&mut self, render_fn: F) -> bool
    where
        F: FnOnce(&wgpu::TextureView, &wgpu::TextureView, [u32; 2]),
    {
        if self.suspended {
            return false;
        }
        let size = [self.config.width, self.config.height];
        if self.depth.is_none() {
            self.depth = Some(
                make_compositor_depth_texture(&self.device, size).create_view(&Default::default()),
            );
        }
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) => f,
            wgpu::CurrentSurfaceTexture::Suboptimal(f) => {
                // Present this frame, then reconfigure on next acquire if needed.
                f
            }
            wgpu::CurrentSurfaceTexture::Outdated
            | wgpu::CurrentSurfaceTexture::Lost
            | wgpu::CurrentSurfaceTexture::Validation => {
                self.surface.configure(&self.device, &self.config);
                return false;
            }
            _ => return false,
        };
        let target = frame.texture.create_view(&wgpu::TextureViewDescriptor {
            format: self.hud_view_format,
            ..Default::default()
        });
        if let Some(depth) = &self.depth {
            render_fn(&target, depth, size);
        }
        self.queue.present(frame);
        true
    }

    /// Copy each view's frame to its top-left corner on the window and present. Frames that
    /// don't fit are clipped; parts of the window no view covers (a view before its first frame,
    /// one still at its old size after a resize) are black. Nothing when the window has no frame
    /// to give (occluded, resized, timed out). Returns true when a frame was submitted.
    pub fn present(&mut self, views: &[(&wgpu::Texture, [u32; 2])]) -> bool {
        if self.suspended {
            return false;
        }
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f)
            | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return false;
            }
            _ => return false,
        };
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Splitscreen compositor"),
            });
        let target = frame.texture.create_view(&Default::default());
        drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Splitscreen compositor clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        }));
        for &(texture, [x, y]) in views {
            let [width, height] = clipped_size(
                [texture.width(), texture.height()],
                [x, y],
                [self.config.width, self.config.height],
            );
            if width == 0 || height == 0 {
                continue;
            }
            encoder.copy_texture_to_texture(
                texture.as_image_copy(),
                wgpu::TexelCopyTextureInfo {
                    origin: wgpu::Origin3d { x, y, z: 0 },
                    ..frame.texture.as_image_copy()
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
        }
        self.queue.submit([encoder.finish()]);
        self.queue.present(frame);
        true
    }
}

/// Copies never reach past the surface, including while a resized view still has its old frame.
fn clipped_size(view: [u32; 2], at: [u32; 2], surface: [u32; 2]) -> [u32; 2] {
    [
        view[0].min(surface[0].saturating_sub(at[0])),
        view[1].min(surface[1].saturating_sub(at[1])),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_are_clipped_to_the_window_during_resize() {
        assert_eq!(clipped_size([640, 480], [0, 0], [1280, 720]), [640, 480]);
        assert_eq!(
            clipped_size([640, 480], [1000, 600], [1280, 720]),
            [280, 120]
        );
        assert_eq!(clipped_size([640, 480], [1280, 720], [1280, 720]), [0, 0]);
        assert_eq!(
            clipped_size([640, 480], [u32::MAX, 0], [1280, 720]),
            [0, 480]
        );
    }

    #[test]
    fn select_hud_surface_format_accepts_direct_rgba8_match() {
        let (fmt, views) = select_hud_surface_format(
            wgpu::TextureFormat::Rgba8Unorm,
            &[
                wgpu::TextureFormat::Rgba8Unorm,
                wgpu::TextureFormat::Rgba8UnormSrgb,
            ],
        )
        .unwrap();
        assert_eq!(fmt, wgpu::TextureFormat::Rgba8Unorm);
        assert!(views.is_empty());
    }

    #[test]
    fn select_hud_surface_format_handles_srgb_view_format_pair() {
        let (fmt, views) = select_hud_surface_format(
            wgpu::TextureFormat::Rgba8Unorm,
            &[wgpu::TextureFormat::Rgba8UnormSrgb],
        )
        .unwrap();
        assert_eq!(fmt, wgpu::TextureFormat::Rgba8UnormSrgb);
        assert_eq!(views, vec![wgpu::TextureFormat::Rgba8Unorm]);

        let err = select_hud_surface_format(
            wgpu::TextureFormat::Rgba8Unorm,
            &[wgpu::TextureFormat::Bgra8Unorm],
        );
        assert!(err.is_err());
    }

    #[test]
    fn compositor_hud_depth_resizes_and_zero_size_suspends_without_panic() {
        let instance = crate::gfx::test_gpu::instance();
        let Ok(adapter) =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
        else {
            // Even without a host GPU adapter, verify pure drawable suspension helper.
            assert!(!hud_surface_drawable([0, 0], 8192));
            assert!(!hud_surface_drawable([1280, 0], 8192));
            assert!(hud_surface_drawable([1280, 960], 8192));
            return;
        };
        let (device, _queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
        let max = device.limits().max_texture_dimension_2d;
        assert!(!hud_surface_drawable([0, 0], max));
        let depth_zero = make_compositor_depth_texture(&device, [0, 0]);
        assert_eq!(depth_zero.width(), 1);
        assert_eq!(depth_zero.height(), 1);
        assert_eq!(depth_zero.format(), wgpu::TextureFormat::Depth32Float);

        assert!(hud_surface_drawable([1280, 960], max));
        let depth_duo = make_compositor_depth_texture(&device, [1280, 960]);
        assert_eq!(depth_duo.width(), 1280);
        assert_eq!(depth_duo.height(), 960);
        assert_eq!(depth_duo.format(), wgpu::TextureFormat::Depth32Float);
    }
}
