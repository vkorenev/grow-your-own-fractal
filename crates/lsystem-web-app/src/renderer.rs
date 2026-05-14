use std::sync::Arc;

use lsystem_core::Config;
use lsystem_renderer::camera::Camera;
use lsystem_renderer::line_renderer::{
    ColorParams, FrameOutcome, FrameSkipReason, GpuContext, GpuInitError, LinePipeline2D,
    LinePipeline3D, SurfaceFrame, Vertex2D, Vertex3D,
};
use lsystem_renderer::lsystem_bridge::{
    VertexData, VertexData3D, color_params_from_config, geometry_to_vertices,
    geometry_to_vertices_3d,
};

enum ActiveScene {
    TwoD {
        vertices: Vec<Vertex2D>,
        bounds_min: [f32; 2],
        bounds_max: [f32; 2],
    },
    ThreeD {
        vertices: Vec<Vertex3D>,
        bounds_min: [f32; 3],
        bounds_max: [f32; 3],
    },
}

pub struct CanvasRenderer {
    gpu: GpuContext,
    pipeline_2d: LinePipeline2D,
    pipeline_3d: LinePipeline3D,
    camera: Camera,
    scene: ActiveScene,
    color_params: ColorParams,
    background: wgpu::Color,
    needs_upload: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderStatus {
    Rendered,
    SurfaceLost,
    Skipped(FrameSkipReason),
}

impl CanvasRenderer {
    pub async fn new(canvas: web_sys::HtmlCanvasElement) -> Result<Self, GpuInitError> {
        let (width, height, _) = sync_canvas_size(&canvas);
        let gpu = GpuContext::new(wgpu::SurfaceTarget::Canvas(canvas), width, height).await?;
        let pipeline_2d = LinePipeline2D::new(&gpu.device, gpu.surface_format());
        let pipeline_3d = LinePipeline3D::new(&gpu.device, gpu.surface_format());
        Ok(Self {
            gpu,
            pipeline_2d,
            pipeline_3d,
            camera: Camera::new(),
            scene: ActiveScene::TwoD {
                vertices: Vec::new(),
                bounds_min: [-1.0, -1.0],
                bounds_max: [1.0, 1.0],
            },
            color_params: ColorParams::default(),
            background: wgpu::Color::BLACK,
            needs_upload: false,
        })
    }

    pub fn set_config_and_render(
        &mut self,
        canvas: &web_sys::HtmlCanvasElement,
        config: &Config,
    ) -> RenderStatus {
        let total_segments;
        if config.dimensions == 3 {
            let VertexData3D {
                vertices,
                bounds_min,
                bounds_max,
            } = geometry_to_vertices_3d(lsystem_core::generate_3d(config));
            total_segments = (vertices.len() / 2) as u32;
            self.scene = ActiveScene::ThreeD {
                vertices,
                bounds_min,
                bounds_max,
            };
        } else {
            let VertexData {
                vertices,
                bounds_min,
                bounds_max,
            } = geometry_to_vertices(lsystem_core::generate(config));
            total_segments = (vertices.len() / 2) as u32;
            self.scene = ActiveScene::TwoD {
                vertices,
                bounds_min,
                bounds_max,
            };
        }

        self.color_params = color_params_from_config(&config.colors.line, total_segments);
        let [r, g, b] = config.colors.background;
        self.background = wgpu::Color {
            r: r as f64,
            g: g as f64,
            b: b as f64,
            a: 1.0,
        };
        self.camera.reset();
        self.needs_upload = true;
        self.render(canvas)
    }

    pub fn drag_and_render(
        &mut self,
        canvas: &web_sys::HtmlCanvasElement,
        css_dx: f32,
        css_dy: f32,
    ) -> RenderStatus {
        match &self.scene {
            ActiveScene::TwoD {
                bounds_min,
                bounds_max,
                ..
            } => {
                let (_, _, dpr) = sync_canvas_size(canvas);
                let width = canvas.width().max(1);
                let height = canvas.height().max(1);
                self.camera.pan_by_pixels(
                    css_dx * dpr,
                    css_dy * dpr,
                    *bounds_min,
                    *bounds_max,
                    width,
                    height,
                );
            }
            ActiveScene::ThreeD { .. } => {
                let (_, _, dpr) = sync_canvas_size(canvas);
                self.camera.orbit_by_pixels(css_dx * dpr, css_dy * dpr);
            }
        }
        self.render(canvas)
    }

    pub fn zoom_and_render(
        &mut self,
        canvas: &web_sys::HtmlCanvasElement,
        delta_y: f32,
        delta_mode: u32,
        client_x: f32,
        client_y: f32,
    ) -> RenderStatus {
        let (_, _, dpr) = sync_canvas_size(canvas);
        let rect = canvas.get_bounding_client_rect();
        let pixel_delta_y = normalize_wheel_delta_y(delta_y, delta_mode, rect.height() as f32);
        let factor = 1.1_f32.powf(-pixel_delta_y / 100.0);
        match &self.scene {
            ActiveScene::TwoD {
                bounds_min,
                bounds_max,
                ..
            } => {
                let cursor = [
                    (client_x as f64 - rect.left()) as f32 * dpr,
                    (client_y as f64 - rect.top()) as f32 * dpr,
                ];
                self.camera.zoom_toward_cursor(
                    factor,
                    cursor,
                    *bounds_min,
                    *bounds_max,
                    canvas.width().max(1),
                    canvas.height().max(1),
                );
            }
            ActiveScene::ThreeD { .. } => {
                self.camera.zoom_3d(factor);
            }
        }
        self.render(canvas)
    }

    pub fn orbit_and_render(
        &mut self,
        canvas: &web_sys::HtmlCanvasElement,
        d_az: f32,
        d_el: f32,
    ) -> RenderStatus {
        self.camera.orbit_by(d_az, d_el);
        self.render(canvas)
    }

    pub fn roll_and_render(
        &mut self,
        canvas: &web_sys::HtmlCanvasElement,
        degrees: f32,
    ) -> RenderStatus {
        self.camera.roll_by(degrees);
        self.render(canvas)
    }

    pub fn auto_rotate_and_render(
        &mut self,
        canvas: &web_sys::HtmlCanvasElement,
        degrees: f32,
    ) -> RenderStatus {
        self.camera.auto_rotate_by(degrees);
        self.render(canvas)
    }

    pub fn reset_and_render(&mut self, canvas: &web_sys::HtmlCanvasElement) -> RenderStatus {
        self.camera.reset();
        self.render(canvas)
    }

    pub fn render(&mut self, canvas: &web_sys::HtmlCanvasElement) -> RenderStatus {
        let (width, height, _) = sync_canvas_size(canvas);
        if self.gpu.size() != (width, height) {
            self.gpu.resize(width, height);
        }

        match self.gpu.begin_frame() {
            FrameOutcome::Skipped(reason) => RenderStatus::Skipped(reason),
            FrameOutcome::SurfaceLost => RenderStatus::SurfaceLost,
            FrameOutcome::Ready(frame) => {
                self.render_frame(width, height, frame);
                RenderStatus::Rendered
            }
        }
    }

    pub async fn recover_surface_and_render(
        &mut self,
        canvas: web_sys::HtmlCanvasElement,
    ) -> Result<RenderStatus, GpuInitError> {
        let (width, height, _) = sync_canvas_size(&canvas);
        let gpu =
            GpuContext::new(wgpu::SurfaceTarget::Canvas(canvas.clone()), width, height).await?;
        self.pipeline_2d = LinePipeline2D::new(&gpu.device, gpu.surface_format());
        self.pipeline_3d = LinePipeline3D::new(&gpu.device, gpu.surface_format());
        self.gpu = gpu;
        self.needs_upload = true;
        Ok(self.render(&canvas))
    }

    pub fn device_queue(&self) -> (Arc<wgpu::Device>, Arc<wgpu::Queue>) {
        (Arc::clone(&self.gpu.device), Arc::clone(&self.gpu.queue))
    }

    pub fn camera(&self) -> Camera {
        self.camera.clone()
    }

    fn render_frame(&mut self, width: u32, height: u32, frame: SurfaceFrame) {
        let SurfaceFrame {
            frame,
            view,
            mut encoder,
            reconfigure_after_present,
        } = frame;

        match &self.scene {
            ActiveScene::TwoD {
                vertices,
                bounds_min,
                bounds_max,
            } => {
                if self.needs_upload {
                    self.pipeline_2d.upload(
                        &self.gpu.device,
                        &self.gpu.queue,
                        vertices,
                        self.color_params,
                    );
                    self.needs_upload = false;
                }
                self.pipeline_2d.write_transform(
                    &self.gpu.queue,
                    self.camera.compute_transform(
                        *bounds_min,
                        *bounds_max,
                        width.max(1),
                        height.max(1),
                    ),
                );
            }
            ActiveScene::ThreeD {
                vertices,
                bounds_min,
                bounds_max,
            } => {
                if self.needs_upload {
                    self.pipeline_3d.upload(
                        &self.gpu.device,
                        &self.gpu.queue,
                        vertices,
                        self.color_params,
                    );
                    self.needs_upload = false;
                }
                self.pipeline_3d.write_mvp(
                    &self.gpu.queue,
                    self.camera.compute_mvp_3d(
                        *bounds_min,
                        *bounds_max,
                        width.max(1),
                        height.max(1),
                    ),
                );
            }
        }

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("web_app_fractal_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(self.background),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });
            match &self.scene {
                ActiveScene::TwoD { .. } => self.pipeline_2d.draw(&mut pass),
                ActiveScene::ThreeD { .. } => self.pipeline_3d.draw(&mut pass),
            }
        }

        self.gpu
            .end_frame(*frame, encoder, reconfigure_after_present);
    }
}

fn sync_canvas_size(canvas: &web_sys::HtmlCanvasElement) -> (u32, u32, f32) {
    let dpr = web_sys::window()
        .map(|window| window.device_pixel_ratio() as f32)
        .unwrap_or(1.0)
        .max(1.0);
    let rect = canvas.get_bounding_client_rect();
    let width = ((rect.width() as f32 * dpr).round() as u32).max(1);
    let height = ((rect.height() as f32 * dpr).round() as u32).max(1);
    if canvas.width() != width {
        canvas.set_width(width);
    }
    if canvas.height() != height {
        canvas.set_height(height);
    }
    (width, height, dpr)
}

fn normalize_wheel_delta_y(delta_y: f32, delta_mode: u32, page_height: f32) -> f32 {
    match delta_mode {
        0 => delta_y,
        1 => delta_y * 16.0,
        2 => delta_y * page_height.max(1.0),
        _ => delta_y,
    }
}
