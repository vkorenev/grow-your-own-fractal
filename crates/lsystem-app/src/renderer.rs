use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use egui_wgpu::ScreenDescriptor;
use lsystem_core::Geometry;
use wgpu::util::DeviceExt;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::Key;
use winit::window::{Window, WindowId};

use crate::camera::{Camera, Transform};
use crate::input::InputState;
use crate::ui::UiState;

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct Vertex {
    position: [f32; 2],
}

fn geometry_to_vertices(geometry: &Geometry) -> (Vec<Vertex>, [f32; 2], [f32; 2]) {
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
    egui_renderer: egui_wgpu::Renderer,
}

impl GpuState {
    async fn new(window: Arc<Window>, vertices: &[Vertex], initial_transform: &Transform) -> Self {
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
            .expect("no GPU adapter found");
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

        let egui_renderer =
            egui_wgpu::Renderer::new(&device, format, egui_wgpu::RendererOptions::default());

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
            egui_renderer,
        }
    }

    fn size(&self) -> (u32, u32) {
        (self.surface_config.width, self.surface_config.height)
    }

    fn upload_transform(&self, transform: &Transform) {
        self.queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(transform));
    }

    fn upload_vertices(&mut self, vertices: &[Vertex]) {
        self.vertex_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytemuck::cast_slice(vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
        self.vertex_count = vertices.len() as u32;
    }

    fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width == 0 || new_size.height == 0 {
            return;
        }
        self.surface_config.width = new_size.width;
        self.surface_config.height = new_size.height;
        self.surface.configure(&self.device, &self.surface_config);
    }

    fn render(
        &mut self,
        egui_ctx: &egui::Context,
        egui_winit: &mut egui_winit::State,
        window: &Window,
        ui: &mut UiState,
        panel_px: u32,
    ) -> bool {
        let raw_input = egui_winit.take_egui_input(window);
        egui_ctx.begin_pass(raw_input);
        ui.draw(egui_ctx);
        let full_output = egui_ctx.end_pass();
        egui_winit.handle_platform_output(window, full_output.platform_output);

        let (w, h) = self.size();
        let effective_w = w.saturating_sub(panel_px).max(1);
        let screen_desc = ScreenDescriptor {
            size_in_pixels: [w, h],
            pixels_per_point: full_output.pixels_per_point,
        };
        let tris = egui_ctx.tessellate(full_output.shapes, full_output.pixels_per_point);
        for (id, delta) in &full_output.textures_delta.set {
            self.egui_renderer
                .update_texture(&self.device, &self.queue, *id, delta);
        }

        let (frame, reconfigure) = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) => (t, false),
            wgpu::CurrentSurfaceTexture::Suboptimal(t) => (t, true),
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
            pass.set_viewport(panel_px as f32, 0.0, effective_w as f32, h as f32, 0.0, 1.0);
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            pass.draw(0..self.vertex_count, 0..1);
        }

        self.egui_renderer.update_buffers(
            &self.device,
            &self.queue,
            &mut encoder,
            &tris,
            &screen_desc,
        );
        {
            let mut egui_pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    occlusion_query_set: None,
                    timestamp_writes: None,
                    multiview_mask: None,
                })
                // forget_lifetime is safe here: egui_pass is dropped before
                // encoder.finish(), which is the invariant wgpu requires.
                .forget_lifetime();
            self.egui_renderer
                .render(&mut egui_pass, &tris, &screen_desc);
        }

        for id in &full_output.textures_delta.free {
            self.egui_renderer.free_texture(id);
        }

        self.queue.submit([encoder.finish()]);
        frame.present();

        if reconfigure {
            self.surface.configure(&self.device, &self.surface_config);
        }
        reconfigure
    }
}

pub struct App {
    window: Option<Arc<Window>>,
    state: Option<GpuState>,
    bounds_min: [f32; 2],
    bounds_max: [f32; 2],
    camera: Camera,
    input: InputState,
    egui_ctx: egui::Context,
    egui_winit: Option<egui_winit::State>,
    ui: UiState,
}

impl App {
    pub fn new() -> Self {
        let ui = UiState::new();
        Self {
            window: None,
            state: None,
            bounds_min: [-1.0, -1.0],
            bounds_max: [1.0, 1.0],
            camera: Camera::new(),
            input: InputState::new(),
            egui_ctx: egui::Context::default(),
            egui_winit: None,
            ui,
        }
    }

    /// Physical pixel width of the egui side panel, used to restrict the camera viewport.
    fn panel_px(&self) -> u32 {
        let scale = self
            .window
            .as_ref()
            .map(|w| w.scale_factor() as f32)
            .unwrap_or(1.0);
        self.ui.panel_width.mul_add(scale, 0.5) as u32
    }

    fn regenerate_if_dirty(&mut self) {
        if !self.ui.dirty {
            return;
        }
        self.ui.dirty = false;
        let Some(cfg) = self.ui.effective_config() else {
            return;
        };
        let geometry = lsystem_core::generate(&cfg);
        let (vertices, bounds_min, bounds_max) = geometry_to_vertices(&geometry);
        self.bounds_min = bounds_min;
        self.bounds_max = bounds_max;
        self.camera.reset();
        if let Some(state) = &mut self.state {
            state.upload_vertices(&vertices);
        }
        self.upload_camera_transform();
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn upload_camera_transform(&self) {
        if let Some(state) = &self.state {
            let (w, h) = state.size();
            let effective_w = w.saturating_sub(self.panel_px()).max(1);
            let transform =
                self.camera
                    .compute_transform(self.bounds_min, self.bounds_max, effective_w, h);
            state.upload_transform(&transform);
        }
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = Arc::new(
            event_loop
                .create_window(Window::default_attributes().with_title("Grow Your Own Fractal"))
                .unwrap(),
        );

        let egui_winit = egui_winit::State::new(
            self.egui_ctx.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            None,
            None,
        );
        self.egui_winit = Some(egui_winit);

        let size = window.inner_size();

        let initial_vertices: Vec<Vertex> = Vec::new();
        let initial_transform = self.camera.compute_transform(
            self.bounds_min,
            self.bounds_max,
            size.width.max(1),
            size.height.max(1),
        );
        let state = pollster::block_on(GpuState::new(
            window.clone(),
            &initial_vertices,
            &initial_transform,
        ));
        self.window = Some(window);
        self.state = Some(state);

        // Trigger initial geometry generation.
        self.ui.dirty = true;
        self.regenerate_if_dirty();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        // Unconditionally clear drag on left-button release so it can never get stuck,
        // even when egui consumes the release event.
        if matches!(
            &event,
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            }
        ) {
            self.input.on_left_button(false);
        }

        // Feed event to egui first.
        let egui_consumed =
            if let (Some(egui_winit), Some(window)) = (&mut self.egui_winit, &self.window) {
                let response = egui_winit.on_window_event(window, &event);
                // response.repaint keeps is_pointer_over_egui() fresh between clicks.
                if response.repaint || response.consumed {
                    window.request_redraw();
                }
                response.consumed
            } else {
                false
            };
        if egui_consumed {
            return;
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),

            WindowEvent::Resized(size) => {
                if let Some(state) = &mut self.state {
                    state.resize(size);
                }
                self.upload_camera_transform();
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }

            // Only start a drag when the cursor is not over the egui panel.
            // Releases are already handled unconditionally above.
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } if !self.egui_ctx.is_pointer_over_egui() => {
                self.input.on_left_button(true);
            }

            WindowEvent::CursorMoved { position, .. } => {
                let x = position.x as f32;
                let y = position.y as f32;
                if let Some([dx, dy]) = self.input.on_cursor_moved(x, y) {
                    if let Some(state) = &self.state {
                        let (w, h) = state.size();
                        let effective_w = w.saturating_sub(self.panel_px()).max(1);
                        self.camera.pan_by_pixels(
                            dx,
                            dy,
                            self.bounds_min,
                            self.bounds_max,
                            effective_w,
                            h,
                        );
                    }
                    self.upload_camera_transform();
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                }
            }

            WindowEvent::MouseWheel { delta, .. } if !self.egui_ctx.is_pointer_over_egui() => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(pos) => pos.y as f32 / 20.0,
                };
                let factor = 1.1_f32.powf(lines);
                if let Some(state) = &self.state {
                    let (w, h) = state.size();
                    let panel_px = self.panel_px();
                    let effective_w = w.saturating_sub(panel_px).max(1);
                    // Cursor position relative to the canvas viewport.
                    let cursor = [
                        (self.input.cursor_pos[0] - panel_px as f32).max(0.0),
                        self.input.cursor_pos[1],
                    ];
                    self.camera.zoom_toward_cursor(
                        factor,
                        cursor,
                        self.bounds_min,
                        self.bounds_max,
                        effective_w,
                        h,
                    );
                }
                self.upload_camera_transform();
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }

            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed && !event.repeat =>
            {
                if let Key::Character(ch) = &event.logical_key
                    && ch.eq_ignore_ascii_case("f")
                {
                    self.camera.reset();
                    self.upload_camera_transform();
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                }
            }

            WindowEvent::RedrawRequested => {
                self.regenerate_if_dirty();
                let panel_px = self.panel_px();
                if let (Some(state), Some(egui_winit), Some(window)) =
                    (&mut self.state, &mut self.egui_winit, &self.window)
                {
                    if state.render(&self.egui_ctx, egui_winit, window, &mut self.ui, panel_px) {
                        window.request_redraw();
                    }
                    // Request another frame if egui wants to animate.
                    if self.egui_ctx.has_requested_repaint() {
                        window.request_redraw();
                    }
                }
                // Dirty flag may have been set by slider interaction during draw.
                if self.ui.dirty
                    && let Some(window) = &self.window
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
