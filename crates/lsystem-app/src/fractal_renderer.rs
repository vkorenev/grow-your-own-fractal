use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
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
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    vertex_count: u32,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl FractalRenderer {
    pub(crate) async fn new(
        window: Arc<Window>,
        vertices: &[Vertex],
        initial_transform: &Transform,
    ) -> Result<Self, ()> {
        let size = window.inner_size();

        let instance = wgpu::Instance::default();
        let surface = instance.create_surface(window).unwrap();
        let Ok(adapter) = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
        else {
            return Err(());
        };
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default())
            .await
            .unwrap();

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

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::bytes_of(initial_transform),
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

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Ok(Self {
            surface,
            device,
            queue,
            surface_config,
            pipeline,
            vertex_buffer,
            vertex_count: vertices.len() as u32,
            uniform_buffer,
            bind_group,
        })
    }

    pub fn surface_format(&self) -> wgpu::TextureFormat {
        self.surface_config.format
    }

    pub fn size(&self) -> (u32, u32) {
        (self.surface_config.width, self.surface_config.height)
    }

    pub(crate) fn upload_transform(&self, transform: &Transform) {
        self.queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(transform));
    }

    pub(crate) fn upload_vertices(&mut self, vertices: &[Vertex]) {
        self.vertex_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytemuck::cast_slice(vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
        self.vertex_count = vertices.len() as u32;
    }

    pub fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width == 0 || new_size.height == 0 {
            return;
        }
        self.surface_config.width = new_size.width;
        self.surface_config.height = new_size.height;
        self.surface.configure(&self.device, &self.surface_config);
    }

    pub(crate) fn begin_frame(&mut self, panel_px: u32) -> FrameOutcome {
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
        let mut encoder = self.device.create_command_encoder(&Default::default());

        let (w, h) = self.size();
        let effective_w = w.saturating_sub(panel_px).max(1);
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.05,
                            g: 0.05,
                            b: 0.1,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });
            pass.set_viewport(panel_px as f32, 0.0, effective_w as f32, h as f32, 0.0, 1.0);
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            pass.draw(0..self.vertex_count, 0..1);
        }

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
