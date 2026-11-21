//! Frame generation (a Horizon setting, off by default): FSR 3.1's frame interpolation
//! makes a frame halfway between each two real ones. The world, without the hand and
//! HUD, is kept at the screen's size each frame; optical flow and the interpolation run
//! on it, the hand and HUD are drawn over the made frame as over the real one, and a
//! present thread shows the made frame, then the real one half a real frame later.
//!
//! The renderer draws into frames of its own while this is on; the present thread
//! copies each to the swapchain, which it alone acquires and presents.

#[cfg(not(target_os = "android"))]
mod interpolation;
#[cfg(not(target_os = "android"))]
mod motion;
#[cfg(not(target_os = "android"))]
mod optical_flow;

use glam::{DMat4, DVec3, Mat4};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

/// Frames to draw into: a pair being drawn, a pair waiting and a pair being shown.
const FRAMES: usize = 6;

/// `HORIZON_FRAMEGEN_DEBUG=generated`: show only made frames (AMD's debug view), so a
/// screenshot or an offscreen frame is the made one; `motion`: the same, each made frame
/// showing what moved on its own in blue (its motion vectors in red and green).
pub(super) fn generated_only() -> bool {
    debug().is_some()
}

fn debug() -> Option<&'static str> {
    static DEBUG: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    DEBUG
        .get_or_init(|| {
            std::env::var("HORIZON_FRAMEGEN_DEBUG")
                .ok()
                .filter(|v| v == "generated" || v == "motion")
        })
        .as_deref()
}

pub struct FrameGen {
    pub on: bool,
    /// The present thread failed to show a frame (or died): until the renderer has
    /// acquired a frame itself, the normal path runs and handles the surface.
    pub paused: bool,
    gpu: Option<Gpu>,
    presenter: Option<Presenter>,
    /// When the last real frame was handed over, and the time between real frames.
    last: Option<Instant>,
    interval: Duration,
    /// The last frame's unjittered `view_proj` and camera.
    previous: Option<(DMat4, DVec3)>,
    /// The last frame's hand vertices and the bob and view they were drawn with.
    hand: Vec<crate::mesher::Vertex>,
    hand_view: Mat4,
    /// Offscreen, the frame the made frame is drawn into.
    offscreen_generated: Option<wgpu::Texture>,
    /// Motion vectors for what moves on its own, made on the first frame.
    #[cfg(not(target_os = "android"))]
    motion: Option<motion::ObjectMotion>,
    /// Real frames and made ones handed to the screen since `shown_since`, and the share
    /// of real frames that had a made frame over the last half second (the F3 screen's),
    /// `None` while off.
    real: u32,
    made: u32,
    shown_since: Instant,
    pub made_share: Option<f32>,
}

/// What runs on the GPU for one screen size.
#[cfg(not(target_os = "android"))]
struct Gpu {
    size: [u32; 2],
    flow: optical_flow::OpticalFlow,
    interpolation: interpolation::Interpolation,
    /// This frame's world at the screen's size, and bindings that copy it or the
    /// interpolation's output (`Post::finish_with`).
    world: wgpu::TextureView,
    world_group: wgpu::BindGroup,
    output_group: wgpu::BindGroup,
    /// The next interpolation has no previous frame.
    reset: bool,
}

#[cfg(target_os = "android")]
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MobileParams {
    reproject: [[f32; 4]; 4],
}

#[cfg(target_os = "android")]
struct MobileInterp {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    uniform: wgpu::Buffer,
}

#[cfg(target_os = "android")]
impl MobileInterp {
    const WGSL: &'static str = r#"
struct PostOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

struct MobileParams {
    reproject: mat4x4<f32>,
};

@group(0) @binding(0) var curr_tex: texture_2d<f32>;
@group(0) @binding(1) var prev_tex: texture_2d<f32>;
@group(0) @binding(2) var depth_tex: texture_depth_2d;
@group(0) @binding(3) var lin_sampler: sampler;
@group(0) @binding(4) var<uniform> params: MobileParams;

@vertex fn vs_post(@builtin(vertex_index) i: u32) -> PostOut {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: PostOut;
    out.clip = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2<f32>(p.x, 1.0 - p.y);
    return out;
}

@fragment fn fs_mobile_interp(in: PostOut) -> @location(0) vec4<f32> {
    let d_dims = vec2<i32>(textureDimensions(depth_tex));
    let d_last = max(d_dims - vec2<i32>(1, 1), vec2<i32>(0, 0));
    let dp = clamp(vec2<i32>(in.uv * vec2<f32>(d_dims)), vec2<i32>(0, 0), d_last);

    var near = textureLoad(depth_tex, dp, 0);
    near = max(near, textureLoad(depth_tex, clamp(dp + vec2<i32>(-1, 0), vec2<i32>(0, 0), d_last), 0));
    near = max(near, textureLoad(depth_tex, clamp(dp + vec2<i32>(1, 0), vec2<i32>(0, 0), d_last), 0));
    near = max(near, textureLoad(depth_tex, clamp(dp + vec2<i32>(0, -1), vec2<i32>(0, 0), d_last), 0));
    near = max(near, textureLoad(depth_tex, clamp(dp + vec2<i32>(0, 1), vec2<i32>(0, 0), d_last), 0));

    let ndc = vec2<f32>(in.uv.x * 2.0 - 1.0, 1.0 - in.uv.y * 2.0);
    let prev_clip = params.reproject * vec4<f32>(ndc, near, 1.0);
    if prev_clip.w <= 1e-5 {
        return textureSampleLevel(curr_tex, lin_sampler, in.uv, 0.0);
    }
    let prev_uv = vec2<f32>(0.5, -0.5) * (prev_clip.xy / prev_clip.w) + vec2<f32>(0.5, 0.5);
    let mv = prev_uv - in.uv;
    if length(mv) > 0.25 {
        return textureSampleLevel(curr_tex, lin_sampler, in.uv, 0.0);
    }

    let uv_curr = in.uv - 0.5 * mv;
    let uv_prev = in.uv + 0.5 * mv;
    let curr_in = all(uv_curr >= vec2<f32>(0.0)) && all(uv_curr <= vec2<f32>(1.0));
    let prev_in = all(uv_prev >= vec2<f32>(0.0)) && all(uv_prev <= vec2<f32>(1.0));

    let c_curr = textureSampleLevel(curr_tex, lin_sampler, clamp(uv_curr, vec2<f32>(0.0), vec2<f32>(1.0)), 0.0);
    let c_prev = textureSampleLevel(prev_tex, lin_sampler, clamp(uv_prev, vec2<f32>(0.0), vec2<f32>(1.0)), 0.0);

    if !prev_in {
        return c_curr;
    }
    if !curr_in {
        return c_prev;
    }

    let diff = length(c_curr.rgb - c_prev.rgb);
    let w_curr = mix(0.5, 0.85, smoothstep(0.15, 0.45, diff));
    return vec4<f32>(mix(c_prev.rgb, c_curr.rgb, w_curr), c_curr.a);
}
"#;

    fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Mobile frame interpolation"),
            source: wgpu::ShaderSource::Wgsl(Self::WGSL.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Mobile frame interpolation"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Mobile frame interpolation"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Mobile frame interpolation"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_post"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_mobile_interp"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Mobile frame interpolation"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Mobile frame interpolation params"),
            size: std::mem::size_of::<MobileParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            layout,
            pipeline,
            sampler,
            uniform,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        curr: &wgpu::TextureView,
        prev: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        reproject: Mat4,
        dst: &wgpu::TextureView,
    ) {
        let params = MobileParams {
            reproject: reproject.to_cols_array_2d(),
        };
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&params));
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Mobile frame interpolation"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(curr),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(prev),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(depth),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: self.uniform.as_entire_binding(),
                },
            ],
        });
        let attachments = [Some(wgpu::RenderPassColorAttachment {
            view: dst,
            resolve_target: None,
            depth_slice: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
        })];
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Mobile frame interpolation"),
            color_attachments: &attachments,
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// Lightweight 1-pass mobile reprojection GPU state for Android.
#[cfg(target_os = "android")]
struct Gpu {
    size: [u32; 2],
    worlds: [wgpu::TextureView; 2],
    world_groups: [wgpu::BindGroup; 2],
    interp: MobileInterp,
    curr_idx: usize,
    reset: bool,
}

/// This frame's camera, for the interpolation.
pub struct Camera {
    /// The world's projection this frame (with TAA's jitter) and without the jitter.
    pub jittered: Mat4,
    pub view_proj: Mat4,
    pub depth: wgpu::TextureView,
    pub render_size: [u32; 2],
    pub jitter: [f32; 2],
    /// This frame's unjittered clip space to the previous frame's, `None` without one.
    pub reproject: Option<Mat4>,
    pub near: f32,
    pub far: f32,
    pub fov_y: f32,
}

impl FrameGen {
    pub fn new() -> Self {
        Self {
            on: false,
            paused: false,
            gpu: None,
            presenter: None,
            last: None,
            interval: Duration::from_millis(16),
            previous: None,
            hand: Vec::new(),
            hand_view: Mat4::IDENTITY,
            offscreen_generated: None,
            #[cfg(not(target_os = "android"))]
            motion: None,
            real: 0,
            made: 0,
            shown_since: Instant::now(),
            made_share: None,
        }
    }

    /// Ends frame generation's frames and present thread: before the surface is
    /// reconfigured or the renderer acquires from it itself.
    pub fn stop(&mut self) {
        self.presenter = None;
        self.made_share = None;
        (self.real, self.made) = (0, 0);
        self.last = None;
        self.previous = None;
        self.hand.clear();
        #[cfg(not(target_os = "android"))]
        if let Some(motion) = &mut self.motion {
            motion.reset();
        }
        if let Some(gpu) = &mut self.gpu {
            gpu.reset = true;
        }
    }

    /// The hand for the made frame, appended to `out`: each vertex halfway between where
    /// it was on screen last frame and where it is now (the hand swings and bobs on its
    /// own, so the world's interpolation can't move it), in this frame's space
    /// (`view`: this frame's bob and view). This frame's hand where the two differ in
    /// shape (another item, the first frame).
    pub fn halfway_hand(
        &mut self,
        hand: &[crate::mesher::Vertex],
        view: Mat4,
        out: &mut Vec<crate::mesher::Vertex>,
    ) {
        let same = self.hand.len() == hand.len()
            && self
                .hand
                .iter()
                .zip(hand)
                .all(|(a, b)| a.layer == b.layer && a.uv == b.uv);
        if same {
            let back = view.inverse();
            out.extend(self.hand.iter().zip(hand).map(|(before, now)| {
                let then = self.hand_view.transform_point3(before.pos.into());
                let mid = (then + view.transform_point3(now.pos.into())) * 0.5;
                crate::mesher::Vertex {
                    pos: back.transform_point3(mid).into(),
                    ..*now
                }
            }));
        } else {
            out.extend_from_slice(hand);
        }
        self.hand.clear();
        self.hand.extend_from_slice(hand);
        self.hand_view = view;
    }

    /// This frame's entity vertices (relative to `camera`) and the entity ids of their
    /// spans (`Frame::entity_motion`), for their own motion vectors; `atlas`: the
    /// entities' textures. Before `generate`, every frame.
    pub fn track_entities(
        &mut self,
        device: &wgpu::Device,
        atlas: &wgpu::TextureView,
        entities: &[crate::mesher::Vertex],
        spans: &[(u32, u64)],
        camera: DVec3,
    ) {
        #[cfg(not(target_os = "android"))]
        self.motion
            .get_or_insert_with(|| motion::ObjectMotion::new(device, atlas.clone()))
            .track(entities, spans, camera);
        #[cfg(target_os = "android")]
        let _ = (device, atlas, entities, spans, camera);
    }

    /// This frame's unjittered clip space to the last frame's, from their `view_proj`s
    /// (relative to `camera`); `None` on the first frame.
    pub fn camera_motion(&mut self, view_proj: Mat4, camera: DVec3) -> Option<Mat4> {
        let current = view_proj.as_dmat4();
        let previous = self.previous.replace((current, camera));
        previous.map(|(p, pc)| super::world_post::reprojection(p, pc, current, camera).as_mat4())
    }

    /// Frames to draw the real and the made frame into, from the present thread's
    /// (started for this `config`'s surface) or, offscreen, the offscreen target and
    /// one of its own. `None` once the present thread failed or died: it is stopped and
    /// frame generation `paused`.
    pub fn frames(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface: Option<&Arc<wgpu::Surface<'static>>>,
        offscreen: Option<&wgpu::Texture>,
        window: &Arc<winit::window::Window>,
        config: &wgpu::SurfaceConfiguration,
    ) -> Option<(wgpu::Texture, wgpu::Texture)> {
        let Some(surface) = surface else {
            let real = offscreen.cloned().unwrap_or_else(|| frame(device, config));
            if self
                .offscreen_generated
                .as_ref()
                .is_none_or(|t| t.size() != real.size())
            {
                self.offscreen_generated = Some(frame(device, config));
            }
            let generated = self.offscreen_generated.clone().expect("made above");
            return Some((real, generated));
        };
        if self.presenter.as_ref().is_some_and(|p| p.config != *config) {
            self.presenter = None;
        }
        let presenter = self.presenter.get_or_insert_with(|| {
            Presenter::start(device, queue, surface.clone(), window.clone(), config)
        });
        // A closed channel: the thread died.
        let pair = if presenter.failed() {
            None
        } else {
            presenter.free.recv().ok().zip(presenter.free.recv().ok())
        };
        if pair.is_none() {
            self.stop();
            self.paused = true;
        }
        pair
    }

    /// The world's target at the screen's size this frame, where `Post::finish` draws.
    #[cfg(not(target_os = "android"))]
    pub fn world(
        &mut self,
        device: &wgpu::Device,
        post: &super::world_post::Post,
        config: &wgpu::SurfaceConfiguration,
    ) -> wgpu::TextureView {
        let size = [config.width, config.height];
        if self.gpu.as_ref().is_none_or(|g| g.size != size) {
            let world = device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("Frame generation world"),
                    size: wgpu::Extent3d {
                        width: size[0],
                        height: size[1],
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: config.format.remove_srgb_suffix(),
                    // COPY_SRC: the interpolation keeps it as the previous frame.
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                })
                .create_view(&Default::default());
            let interpolation = interpolation::Interpolation::new(device, size);
            self.gpu = Some(Gpu {
                size,
                flow: optical_flow::OpticalFlow::new(device, size),
                world_group: post.bind(device, &world),
                output_group: post.bind(device, interpolation.output()),
                interpolation,
                world,
                reset: true,
            });
        }
        self.gpu.as_ref().expect("made above").world.clone()
    }

    /// The world's target at the screen's size this frame (ping-ponged on Android so
    /// 1-pass mobile reprojection needs zero texture copies).
    #[cfg(target_os = "android")]
    pub fn world(
        &mut self,
        device: &wgpu::Device,
        post: &super::world_post::Post,
        config: &wgpu::SurfaceConfiguration,
    ) -> wgpu::TextureView {
        let size = [config.width, config.height];
        let format = config.format.remove_srgb_suffix();
        if self.gpu.as_ref().is_none_or(|g| g.size != size) {
            let make = || {
                device
                    .create_texture(&wgpu::TextureDescriptor {
                        label: Some("Frame generation world"),
                        size: wgpu::Extent3d {
                            width: size[0],
                            height: size[1],
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                            | wgpu::TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    })
                    .create_view(&Default::default())
            };
            let worlds = [make(), make()];
            let world_groups = [post.bind(device, &worlds[0]), post.bind(device, &worlds[1])];
            self.gpu = Some(Gpu {
                size,
                worlds,
                world_groups,
                interp: MobileInterp::new(device, format),
                curr_idx: 0,
                reset: true,
            });
        }
        let gpu = self.gpu.as_ref().expect("made above");
        gpu.worlds[gpu.curr_idx].clone()
    }

    /// Copy the world (`world`) onto `screen` for the real frame, and draw the made
    /// frame's world onto `generated`. Returns false when there is no made frame (the
    /// first after a reset): `generated` then holds this frame's world too.
    #[cfg(not(target_os = "android"))]
    #[allow(clippy::too_many_arguments)]
    pub fn generate(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        post: &super::world_post::Post,
        screen: &wgpu::TextureView,
        generated: &wgpu::TextureView,
        camera: &Camera,
    ) -> bool {
        let gpu = self.gpu.as_mut().expect("world() ran this frame");
        post.copy_to(encoder, screen, &gpu.world_group);
        let reset = gpu.reset || camera.reproject.is_none();
        let reproject = camera.reproject.unwrap_or(Mat4::IDENTITY);
        let object_motion = self
            .motion
            .as_mut()
            .expect("track_entities() ran this frame")
            .draw(
                device,
                queue,
                encoder,
                &camera.depth,
                camera.render_size,
                camera.jittered,
                camera.view_proj,
                reproject * camera.view_proj,
            );
        gpu.flow.dispatch(device, queue, encoder, &gpu.world, reset);
        gpu.interpolation.dispatch(
            device,
            queue,
            encoder,
            &interpolation::Inputs {
                color: &gpu.world,
                depth: &camera.depth,
                render_size: camera.render_size,
                jitter: camera.jitter,
                reproject,
                object_motion: &object_motion,
                near: camera.near,
                far: camera.far,
                fov_y: camera.fov_y,
                reset,
                flow_vectors: gpu.flow.vectors(),
                scene_change: gpu.flow.scene_change(),
            },
        );
        gpu.reset = false;
        if debug() == Some("motion") {
            post.copy_to(encoder, generated, &post.bind(device, &object_motion));
            return true;
        }
        if reset {
            post.copy_to(encoder, generated, &gpu.world_group);
        } else {
            post.copy_to(encoder, generated, &gpu.output_group);
        }
        !reset
    }

    /// Lightweight 1-pass mobile reprojection interpolation for Android GPUs.
    #[cfg(target_os = "android")]
    #[allow(clippy::too_many_arguments)]
    pub fn generate(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        post: &super::world_post::Post,
        screen: &wgpu::TextureView,
        generated: &wgpu::TextureView,
        camera: &Camera,
    ) -> bool {
        let gpu = self.gpu.as_mut().expect("world() ran this frame");
        let curr = gpu.curr_idx;
        let prev = 1 - curr;
        post.copy_to(encoder, screen, &gpu.world_groups[curr]);
        let reset = gpu.reset || camera.reproject.is_none();
        if reset {
            post.copy_to(encoder, generated, &gpu.world_groups[curr]);
        } else {
            gpu.interp.draw(
                device,
                queue,
                encoder,
                &gpu.worlds[curr],
                &gpu.worlds[prev],
                &camera.depth,
                camera.reproject.unwrap_or(Mat4::IDENTITY),
                generated,
            );
        }
        gpu.curr_idx = prev;
        gpu.reset = false;
        !reset
    }

    /// Hand this frame's pair to the present thread, after the frame's submission: the
    /// made frame (when `made`) goes first.
    pub fn present(&mut self, real: wgpu::Texture, generated: wgpu::Texture, made: bool) {
        let now = Instant::now();
        if let Some(last) = self.last {
            // A real frame's time, smoothed: the made frame shows for half of it.
            let max_dt = if cfg!(target_os = "android") {
                Duration::from_millis(50)
            } else {
                Duration::from_millis(250)
            };
            let dt = (now - last).min(max_dt);
            self.interval = self.interval.mul_f32(0.9) + dt.mul_f32(0.1);
        }
        self.last = Some(now);
        self.real += 1;
        self.made += u32::from(made && !generated_only());
        if now - self.shown_since >= Duration::from_millis(500) {
            self.made_share = Some(self.made as f32 / self.real as f32);
            (self.real, self.made, self.shown_since) = (0, 0, now);
        }
        let Some(send) = self.presenter.as_ref().and_then(|p| p.send.as_ref()) else {
            return;
        };
        let pair = if generated_only() {
            Pair {
                first: None,
                second: generated,
                spare: Some(real),
                interval: self.interval,
            }
        } else if made {
            Pair {
                first: Some(generated),
                second: real,
                spare: None,
                interval: self.interval,
            }
        } else {
            Pair {
                first: None,
                second: real,
                spare: Some(generated),
                interval: self.interval,
            }
        };
        let _ = send.send(pair);
    }
}

/// A frame to draw into, like the swapchain's.
fn frame(device: &wgpu::Device, config: &wgpu::SurfaceConfiguration) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Frame generation frame"),
        size: wgpu::Extent3d {
            width: config.width,
            height: config.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: config.format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &config.view_formats,
    })
}

/// One real frame's frames for the present thread: `first` (the made frame) now,
/// `second` (the real one) half an `interval` later; `spare` goes straight back.
struct Pair {
    first: Option<wgpu::Texture>,
    second: wgpu::Texture,
    spare: Option<wgpu::Texture>,
    interval: Duration,
}

/// The thread that copies frames to the swapchain and presents them on time.
struct Presenter {
    config: wgpu::SurfaceConfiguration,
    send: Option<mpsc::SyncSender<Pair>>,
    free: mpsc::Receiver<wgpu::Texture>,
    /// The surface gave something other than a frame: the renderer takes it back.
    failed: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

struct BlitPresenter {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
}

impl BlitPresenter {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Frame generation blit"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../world_post.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Frame generation blit"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Frame generation blit"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Frame generation blit"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_post"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_copy"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Frame generation blit"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        Self {
            layout,
            pipeline,
            sampler,
        }
    }

    fn blit(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        src: &wgpu::Texture,
        dst: &wgpu::Texture,
    ) {
        let src_view = src.create_view(&Default::default());
        let dst_view = dst.create_view(&Default::default());
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Frame generation blit"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&src_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        let attachments = [Some(wgpu::RenderPassColorAttachment {
            view: &dst_view,
            resolve_target: None,
            depth_slice: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
        })];
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Frame generation blit"),
            color_attachments: &attachments,
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
    }
}

impl Presenter {
    fn start(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface: Arc<wgpu::Surface<'static>>,
        window: Arc<winit::window::Window>,
        config: &wgpu::SurfaceConfiguration,
    ) -> Self {
        // One pair waiting at most: the renderer then waits as for a swapchain image.
        let (send, receive) = mpsc::sync_channel::<Pair>(1);
        let (give, free) = mpsc::channel();
        for _ in 0..FRAMES {
            let _ = give.send(frame(device, config));
        }
        let can_copy = config.usage.contains(wgpu::TextureUsages::COPY_DST);
        let blit = (!can_copy).then(|| BlitPresenter::new(device, config.format));
        let expected_size = wgpu::Extent3d {
            width: config.width,
            height: config.height,
            depth_or_array_layers: 1,
        };
        let (device, queue) = (device.clone(), queue.clone());
        let vsync = config.present_mode == wgpu::PresentMode::Fifo;
        let failed = Arc::new(AtomicBool::new(false));
        let failed_here = failed.clone();
        let thread = thread::Builder::new()
            .name("Frame generation present".into())
            .spawn(move || {
                let show = |texture: &wgpu::Texture| {
                    if failed_here.load(Ordering::Relaxed) {
                        return;
                    }
                    // Anything but a frame (outdated, lost, occluded...): the renderer
                    // stops this thread and acquires itself, handling it as it does
                    // without frame generation. A suboptimal frame still shows.
                    let target = match surface.get_current_texture() {
                        wgpu::CurrentSurfaceTexture::Success(t) => t,
                        wgpu::CurrentSurfaceTexture::Suboptimal(t) => {
                            if !cfg!(target_os = "android") || t.texture.size() != expected_size {
                                failed_here.store(true, Ordering::Relaxed);
                            }
                            t
                        }
                        _ => {
                            failed_here.store(true, Ordering::Relaxed);
                            return;
                        }
                    };
                    let mut encoder = device.create_command_encoder(&Default::default());
                    if let Some(blit) = &blit {
                        blit.blit(&device, &mut encoder, texture, &target.texture);
                    } else {
                        encoder.copy_texture_to_texture(
                            texture.as_image_copy(),
                            target.texture.as_image_copy(),
                            target.texture.size(),
                        );
                    }
                    queue.submit([encoder.finish()]);
                    if vsync {
                        window.pre_present_notify();
                    }
                    queue.present(target);
                };
                while let Ok(pair) = receive.recv() {
                    if let Some(first) = pair.first {
                        let shown = Instant::now();
                        show(&first);
                        let _ = give.send(first);
                        // ponytail: sleep pacing; a spin for the last half millisecond
                        // if it shows uneven on some systems.
                        let due = shown + pair.interval / 2;
                        if let Some(wait) = due.checked_duration_since(Instant::now()) {
                            thread::sleep(wait);
                        }
                    }
                    show(&pair.second);
                    let _ = give.send(pair.second);
                    if let Some(spare) = pair.spare {
                        let _ = give.send(spare);
                    }
                }
            })
            .expect("spawn the frame generation present thread");
        Self {
            config: config.clone(),
            send: Some(send),
            free,
            failed,
            thread: Some(thread),
        }
    }

    /// The surface failed it or the thread died (a panic).
    fn failed(&self) -> bool {
        self.failed.load(Ordering::Relaxed) || self.thread.as_ref().is_none_or(|t| t.is_finished())
    }
}

impl Drop for Presenter {
    fn drop(&mut self) {
        self.send = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesher::Vertex;
    use glam::Vec3;

    fn vertex(pos: Vec3) -> Vertex {
        Vertex {
            pos: pos.into(),
            ..bytemuck::Zeroable::zeroed()
        }
    }

    /// A swing lands halfway; a hand still on screen while the camera turns stays still;
    /// another item shows as it is.
    #[test]
    fn the_made_frame_s_hand_is_halfway_on_screen() {
        let mut fg = FrameGen::new();
        let mut out = Vec::new();
        fg.halfway_hand(
            &[vertex(Vec3::new(0.0, 0.0, -1.0))],
            Mat4::IDENTITY,
            &mut out,
        );
        fg.halfway_hand(
            &[vertex(Vec3::new(0.4, 0.0, -1.0))],
            Mat4::IDENTITY,
            &mut out,
        );
        assert!((Vec3::from(out[1].pos) - Vec3::new(0.2, 0.0, -1.0)).length() < 1e-6);
        // The camera turned: the vertex turned with it, the same on screen.
        let turned = Mat4::from_rotation_y(0.3);
        let on_screen = Vec3::new(0.4, 0.0, -1.0);
        let pos = turned.inverse().transform_point3(on_screen);
        fg.halfway_hand(&[vertex(pos)], turned, &mut out);
        assert!((Vec3::from(out[2].pos) - pos).length() < 1e-6);
        let mut other = vertex(Vec3::ZERO);
        other.uv = [0.5, 0.5];
        fg.halfway_hand(&[other], turned, &mut out);
        assert_eq!(out[3].pos, [0.0; 3]);
    }
}
