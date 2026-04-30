use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use lsystem_core::Geometry;
use wgpu::util::DeviceExt;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowId};

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct Vertex {
    position: [f32; 2],
}

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct Transform {
    scale: [f32; 2],
    offset: [f32; 2],
}

fn compute_transform(
    bounds_min: [f32; 2],
    bounds_max: [f32; 2],
    width: u32,
    height: u32,
) -> Transform {
    let geom_w = (bounds_max[0] - bounds_min[0]).max(1.0);
    let geom_h = (bounds_max[1] - bounds_min[1]).max(1.0);
    let cx = (bounds_min[0] + bounds_max[0]) * 0.5;
    let cy = (bounds_min[1] + bounds_max[1]) * 0.5;

    let px_per_unit = (width as f32 / geom_w).min(height as f32 / geom_h) * 0.9;
    let sx = px_per_unit * 2.0 / width as f32;
    let sy = px_per_unit * 2.0 / height as f32;

    Transform {
        scale: [sx, sy],
        offset: [-cx * sx, -cy * sy],
    }
}

struct GpuState {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface_config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    vertex_count: u32,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    bounds_min: [f32; 2],
    bounds_max: [f32; 2],
}

impl GpuState {
    async fn new(
        window: Arc<Window>,
        vertices: &[Vertex],
        bounds_min: [f32; 2],
        bounds_max: [f32; 2],
    ) -> Self {
        let size = window.inner_size();

        let instance = wgpu::Instance::default();
        let surface = instance.create_surface(window).unwrap();
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .expect("no GPU adapter found — is a WebGPU-compatible GPU available?");
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default())
            .await
            .unwrap();

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

        let transform = compute_transform(
            bounds_min,
            bounds_max,
            size.width.max(1),
            size.height.max(1),
        );
        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::bytes_of(&transform),
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

        Self {
            surface,
            device,
            queue,
            surface_config,
            pipeline,
            vertex_buffer,
            vertex_count: vertices.len() as u32,
            uniform_buffer,
            bind_group,
            bounds_min,
            bounds_max,
        }
    }

    fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width == 0 || new_size.height == 0 {
            return;
        }
        self.surface_config.width = new_size.width;
        self.surface_config.height = new_size.height;
        self.surface.configure(&self.device, &self.surface_config);
        let transform = compute_transform(
            self.bounds_min,
            self.bounds_max,
            new_size.width,
            new_size.height,
        );
        self.queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&transform));
    }

    fn render(&mut self) -> bool {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) => t,
            wgpu::CurrentSurfaceTexture::Suboptimal(t) => {
                self.surface.configure(&self.device, &self.surface_config);
                t
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.surface_config);
                return true;
            }
            wgpu::CurrentSurfaceTexture::Timeout
            | wgpu::CurrentSurfaceTexture::Occluded
            | wgpu::CurrentSurfaceTexture::Validation => return false,
        };
        let view = frame.texture.create_view(&Default::default());
        let mut encoder = self.device.create_command_encoder(&Default::default());
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
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            pass.draw(0..self.vertex_count, 0..1);
        }
        self.queue.submit([encoder.finish()]);
        frame.present();
        false
    }
}

pub struct App {
    window: Option<Arc<Window>>,
    state: Option<GpuState>,
    vertices: Vec<Vertex>,
    bounds_min: [f32; 2],
    bounds_max: [f32; 2],
}

impl App {
    pub fn new(geometry: Geometry) -> Self {
        let Geometry::D2 { segments } = geometry;

        let mut min_x = f32::INFINITY;
        let mut min_y = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut max_y = f32::NEG_INFINITY;
        let mut vertices = Vec::with_capacity(segments.len() * 2);

        for [a, b] in &segments {
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

        Self {
            window: None,
            state: None,
            vertices,
            bounds_min: [min_x, min_y],
            bounds_max: [max_x, max_y],
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = Arc::new(
            event_loop
                .create_window(Window::default_attributes().with_title("Grow Your Own Fractal"))
                .unwrap(),
        );
        let state = pollster::block_on(GpuState::new(
            window.clone(),
            &self.vertices,
            self.bounds_min,
            self.bounds_max,
        ));
        window.request_redraw();
        self.window = Some(window);
        self.state = Some(state);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(state) = &mut self.state {
                    state.resize(size);
                }
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => {
                if let (Some(state), Some(window)) = (&mut self.state, &self.window)
                    && state.render()
                {
                    window.request_redraw();
                }
            }
            _ => {}
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

    // --- compute_transform ---

    #[test]
    fn transform_square_geo_square_viewport() {
        let t = compute_transform([-1.0, -1.0], [1.0, 1.0], 100, 100);
        assert!(close(t.scale[0], 0.9), "scale x = {}", t.scale[0]);
        assert!(close(t.scale[1], 0.9), "scale y = {}", t.scale[1]);
        assert!(close(t.offset[0], 0.0));
        assert!(close(t.offset[1], 0.0));
    }

    #[test]
    fn transform_center_maps_to_ndc_origin() {
        // Geometry centered at (3, 2).
        let t = compute_transform([1.0, 0.0], [5.0, 4.0], 200, 200);
        assert!(close(3.0 * t.scale[0] + t.offset[0], 0.0));
        assert!(close(2.0 * t.scale[1] + t.offset[1], 0.0));
    }

    #[test]
    fn transform_width_constrained_fills_horizontal() {
        // 4-wide × 1-tall geometry is width-constrained: horizontal NDC extent = 0.9 × 2.
        let t = compute_transform([0.0, 0.0], [4.0, 1.0], 100, 100);
        assert!(close(4.0 * t.scale[0], 1.8));
        assert!(1.0 * t.scale[1] < 0.9);
    }

    #[test]
    fn transform_height_constrained_fills_vertical() {
        // 1-wide × 4-tall geometry is height-constrained: vertical NDC extent = 0.9 × 2.
        let t = compute_transform([0.0, 0.0], [1.0, 4.0], 100, 100);
        assert!(close(4.0 * t.scale[1], 1.8));
        assert!(1.0 * t.scale[0] < 0.9);
    }

    #[test]
    fn transform_preserves_aspect_ratio_in_landscape_viewport() {
        // 200×100 window, square geometry: scale_x ≠ scale_y, but pixel density per
        // world unit must be equal in both axes so the fractal isn't stretched.
        let t = compute_transform([-1.0, -1.0], [1.0, 1.0], 200, 100);
        let px_per_unit_x = t.scale[0] * 100.0; // (width / 2) NDC-to-pixel factor
        let px_per_unit_y = t.scale[1] * 50.0; //  (height / 2)
        assert!(close(px_per_unit_x, px_per_unit_y));
    }

    #[test]
    fn transform_degenerate_point_geometry_stays_finite() {
        // Zero-size geometry hits the max(1.0) floor; scale must stay finite and positive.
        let t = compute_transform([5.0, 3.0], [5.0, 3.0], 100, 100);
        assert!(t.scale[0].is_finite() && t.scale[0] > 0.0);
        assert!(t.scale[1].is_finite() && t.scale[1] > 0.0);
        // The point itself should map to NDC origin.
        assert!(close(5.0 * t.scale[0] + t.offset[0], 0.0));
        assert!(close(3.0 * t.scale[1] + t.offset[1], 0.0));
    }

    // --- App::new ---

    #[test]
    fn app_empty_geometry_uses_fallback_bounds() {
        // "A" has no rule and draws nothing — zero segments produced.
        let geom = generate(&cfg(
            "name=\"t\"\naxiom=\"A\"\niterations=0\nangle=90.0\nstep=1.0",
        ));
        let app = App::new(geom);
        assert!(app.vertices.is_empty());
        assert!(close(app.bounds_min[0], -1.0));
        assert!(close(app.bounds_min[1], -1.0));
        assert!(close(app.bounds_max[0], 1.0));
        assert!(close(app.bounds_max[1], 1.0));
    }

    #[test]
    fn app_vertices_match_segment_endpoints() {
        // F+F with 90° angle: (0,0)→(1,0) then (1,0)→(1,1).
        let geom = generate(&cfg(
            "name=\"t\"\naxiom=\"F+F\"\niterations=0\nangle=90.0\nstep=1.0",
        ));
        let app = App::new(geom);
        assert_eq!(app.vertices.len(), 4);
        assert!(close(app.vertices[0].position[0], 0.0));
        assert!(close(app.vertices[0].position[1], 0.0));
        assert!(close(app.vertices[1].position[0], 1.0));
        assert!(close(app.vertices[1].position[1], 0.0));
        assert!(close(app.vertices[2].position[0], 1.0));
        assert!(close(app.vertices[2].position[1], 0.0));
        assert!(close(app.vertices[3].position[0], 1.0));
        assert!(close(app.vertices[3].position[1], 1.0));
    }

    #[test]
    fn app_bounds_are_tight_over_all_segments() {
        // F+F-F with 90° angle: east → north → east, spanning x ∈ [0,2], y ∈ [0,1].
        let geom = generate(&cfg(
            "name=\"t\"\naxiom=\"F+F-F\"\niterations=0\nangle=90.0\nstep=1.0",
        ));
        let app = App::new(geom);
        assert_eq!(app.vertices.len(), 6);
        assert!(close(app.bounds_min[0], 0.0));
        assert!(close(app.bounds_max[0], 2.0));
        assert!(close(app.bounds_min[1], 0.0));
        assert!(close(app.bounds_max[1], 1.0));
    }
}
