use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use egui::PaintCallbackInfo;
use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};
use lsystem_core::Geometry;
use wgpu::util::DeviceExt;
use winit::window::Window;

use crate::camera::Transform;

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub(crate) struct Vertex {
    position: [f32; 2],
}

pub(crate) fn geometry_to_vertices(geometry: &Geometry) -> (Vec<Vertex>, [f32; 2], [f32; 2]) {
    let Geometry::D2 { segments } = geometry;

    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    let mut vertices = Vec::with_capacity(segments.len() * 2);

    for [a, b] in segments {
        min_x = min_x.min(a.x).min(b.x);
        min_y = min_y.min(a.y).min(b.y);
        max_x = max_x.max(a.x).max(b.x);
        max_y = max_y.max(a.y).max(b.y);
        vertices.push(Vertex {
            position: [a.x, a.y],
        });
        vertices.push(Vertex {
            position: [b.x, b.y],
        });
    }

    if min_x.is_infinite() {
        min_x = -1.0;
        max_x = 1.0;
        min_y = -1.0;
        max_y = 1.0;
    }

    (vertices, [min_x, min_y], [max_x, max_y])
}

/// Maximum number of line segments that fit in a 256 MiB vertex buffer (wgpu's guaranteed limit).
/// Each segment occupies 2 vertices × `size_of::<Vertex>()` bytes.
pub(crate) const MAX_SEGMENTS: u64 = 268_435_456 / (2 * std::mem::size_of::<Vertex>() as u64);

/// GPU resources for fractal rendering, stored in egui's `CallbackResources` TypeMap.
pub(crate) struct FractalPipelineResources {
    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    vertex_buffer: wgpu::Buffer,
    vertex_count: u32,
    /// Matches `App::geometry_version`; when they differ, the vertex buffer is re-uploaded.
    geometry_version: u64,
}

impl FractalPipelineResources {
    pub(crate) fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::bytes_of(&Transform {
                scale: [1.0, 1.0],
                offset: [0.0, 0.0],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2],
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::LineList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // Placeholder; never bound for drawing (vertex_count stays 0 until the first
        // FractalCallback::prepare swaps in a real buffer).
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: std::mem::size_of::<Vertex>() as u64,
            usage: wgpu::BufferUsages::VERTEX,
            mapped_at_creation: false,
        });

        Self {
            pipeline,
            uniform_buffer,
            bind_group,
            vertex_buffer,
            vertex_count: 0,
            // u64::MAX never matches a real `App::geometry_version`, so the first
            // FractalCallback::prepare call always uploads.
            geometry_version: u64::MAX,
        }
    }
}

/// Per-frame data passed into egui's paint callback system.
pub(crate) struct FractalCallback {
    pub vertices: Arc<Vec<Vertex>>,
    pub transform: Transform,
    pub geometry_version: u64,
}

impl CallbackTrait for FractalCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen_descriptor: &ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let res = callback_resources
            .get_mut::<FractalPipelineResources>()
            .unwrap();

        if self.geometry_version != res.geometry_version {
            if !self.vertices.is_empty() {
                res.vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: None,
                    contents: bytemuck::cast_slice(&self.vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                });
            }
            res.vertex_count = self.vertices.len() as u32;
            res.geometry_version = self.geometry_version;
        }

        queue.write_buffer(&res.uniform_buffer, 0, bytemuck::bytes_of(&self.transform));
        vec![]
    }

    fn paint(
        &self,
        _info: PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        callback_resources: &CallbackResources,
    ) {
        let res = callback_resources
            .get::<FractalPipelineResources>()
            .unwrap();

        // egui_wgpu sets the viewport to our allocated rect before calling paint().
        render_pass.set_pipeline(&res.pipeline);
        render_pass.set_bind_group(0, &res.bind_group, &[]);
        if res.vertex_count > 0 {
            render_pass.set_vertex_buffer(0, res.vertex_buffer.slice(..));
            render_pass.draw(0..res.vertex_count, 0..1);
        }
    }
}

pub(crate) enum FrameOutcome {
    Ready(
        Box<wgpu::SurfaceTexture>,
        wgpu::TextureView,
        wgpu::CommandEncoder,
        bool,
    ),
    Reconfigured,
    Skip,
}

pub struct FractalRenderer {
    surface: wgpu::Surface<'static>,
    pub device: Arc<wgpu::Device>,
    pub queue: Arc<wgpu::Queue>,
    surface_config: wgpu::SurfaceConfiguration,
}

impl FractalRenderer {
    pub(crate) async fn new(window: Arc<Window>) -> Result<Self, ()> {
        let size = window.inner_size();

        let instance = wgpu::Instance::default();
        let surface = instance.create_surface(window).map_err(|_| ())?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .map_err(|_| ())?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default())
            .await
            .map_err(|_| ())?;

        let device = Arc::new(device);
        let queue = Arc::new(queue);

        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .find(|f| f.is_srgb())
            .copied()
            .unwrap_or(caps.formats[0]);

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: caps.present_modes[0],
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &surface_config);

        Ok(Self {
            surface,
            device,
            queue,
            surface_config,
        })
    }

    pub fn surface_format(&self) -> wgpu::TextureFormat {
        self.surface_config.format
    }

    pub fn size(&self) -> (u32, u32) {
        (self.surface_config.width, self.surface_config.height)
    }

    pub fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width == 0 || new_size.height == 0 {
            return;
        }
        self.surface_config.width = new_size.width;
        self.surface_config.height = new_size.height;
        self.surface.configure(&self.device, &self.surface_config);
    }

    pub(crate) fn begin_frame(&mut self) -> FrameOutcome {
        let (frame, reconfigure_after) = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) => (t, false),
            wgpu::CurrentSurfaceTexture::Suboptimal(t) => (t, true),
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.surface_config);
                return FrameOutcome::Reconfigured;
            }
            wgpu::CurrentSurfaceTexture::Timeout
            | wgpu::CurrentSurfaceTexture::Occluded
            | wgpu::CurrentSurfaceTexture::Validation => return FrameOutcome::Skip,
        };

        let view = frame.texture.create_view(&Default::default());
        let encoder = self.device.create_command_encoder(&Default::default());
        // The surface is cleared by the egui render pass (LoadOp::Clear).
        FrameOutcome::Ready(Box::new(frame), view, encoder, reconfigure_after)
    }

    pub(crate) fn end_frame(
        &self,
        frame: wgpu::SurfaceTexture,
        encoder: wgpu::CommandEncoder,
        reconfigure_after: bool,
    ) {
        self.queue.submit([encoder.finish()]);
        frame.present();
        if reconfigure_after {
            self.surface.configure(&self.device, &self.surface_config);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lsystem_core::{Config, generate};

    const EPS: f32 = 1e-5;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < EPS
    }

    fn cfg(toml: &str) -> Config {
        Config::parse(toml).unwrap()
    }

    #[test]
    fn empty_geometry_uses_fallback_bounds() {
        // axiom has no F, so no segments are drawn
        let geom = generate(&cfg(
            "name=\"t\"\naxiom=\"A\"\niterations=0\nangle=90.0\nstep=1.0",
        ));
        let (verts, min, max) = geometry_to_vertices(&geom);
        assert!(verts.is_empty());
        assert!(close(min[0], -1.0) && close(min[1], -1.0));
        assert!(close(max[0], 1.0) && close(max[1], 1.0));
    }

    #[test]
    fn single_segment_produces_two_vertices_and_tight_bounds() {
        // "F" at 0 iterations: one segment from (0,0) to (1,0)
        let geom = generate(&cfg(
            "name=\"t\"\naxiom=\"F\"\niterations=0\nangle=90.0\nstep=1.0",
        ));
        let (verts, min, max) = geometry_to_vertices(&geom);
        assert_eq!(verts.len(), 2);
        assert!(close(verts[0].position[0], 0.0) && close(verts[0].position[1], 0.0));
        assert!(close(verts[1].position[0], 1.0) && close(verts[1].position[1], 0.0));
        assert!(close(min[0], 0.0) && close(min[1], 0.0));
        assert!(close(max[0], 1.0) && close(max[1], 0.0));
    }

    #[test]
    fn bounds_are_tight_over_all_segments() {
        // "F+F-F": three segments covering x=[0,2], y=[0,1]
        let geom = generate(&cfg(
            "name=\"t\"\naxiom=\"F+F-F\"\niterations=0\nangle=90.0\nstep=1.0",
        ));
        let (verts, min, max) = geometry_to_vertices(&geom);
        assert_eq!(verts.len(), 6);
        assert!(close(min[0], 0.0) && close(min[1], 0.0));
        assert!(close(max[0], 2.0) && close(max[1], 1.0));
    }
}
