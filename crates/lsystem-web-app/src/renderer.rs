use std::sync::Arc;

use lsystem_core::Config;
use lsystem_renderer::camera::Camera;
use lsystem_renderer::line_renderer::{
    ColorParams, FrameOutcome, GpuContext, LinePipeline, Vertex,
};
use lsystem_renderer::lsystem_bridge::{
    VertexData, color_params_from_config, geometry_to_vertices,
};

pub struct CanvasRenderer {
    gpu: GpuContext,
    pipeline: LinePipeline,
    camera: Camera,
    vertices: Vec<Vertex>,
    bounds_min: [f32; 2],
    bounds_max: [f32; 2],
    color_params: ColorParams,
    background: wgpu::Color,
    needs_upload: bool,
}

impl CanvasRenderer {
    pub async fn new(canvas: web_sys::HtmlCanvasElement) -> Result<Self, ()> {
        let (width, height, _) = sync_canvas_size(&canvas);
        let gpu = GpuContext::new(wgpu::SurfaceTarget::Canvas(canvas), width, height).await?;
        let pipeline = LinePipeline::new(&gpu.device, gpu.surface_format());
        Ok(Self {
            gpu,
            pipeline,
            camera: Camera::new(),
            vertices: Vec::new(),
            bounds_min: [-1.0, -1.0],
            bounds_max: [1.0, 1.0],
            color_params: ColorParams::default(),
            background: wgpu::Color::BLACK,
            needs_upload: false,
        })
    }

    pub fn set_config_and_render(&mut self, canvas: &web_sys::HtmlCanvasElement, config: &Config) {
        let VertexData {
            vertices,
            bounds_min,
            bounds_max,
        } = geometry_to_vertices(lsystem_core::generate(config));

        let total_segments = (vertices.len() / 2) as u32;
        self.color_params = color_params_from_config(&config.colors.line, total_segments);
        let [r, g, b] = config.colors.background;
        self.background = wgpu::Color {
            r: r as f64,
            g: g as f64,
            b: b as f64,
            a: 1.0,
        };
        self.vertices = vertices;
        self.bounds_min = bounds_min;
        self.bounds_max = bounds_max;
        self.camera.reset();
        self.needs_upload = true;
        self.render(canvas);
    }

    pub fn pan_and_render(
        &mut self,
        canvas: &web_sys::HtmlCanvasElement,
        css_dx: f32,
        css_dy: f32,
    ) {
        let (_, _, dpr) = sync_canvas_size(canvas);
        let width = canvas.width().max(1);
        let height = canvas.height().max(1);
        self.camera.pan_by_pixels(
            css_dx * dpr,
            css_dy * dpr,
            self.bounds_min,
            self.bounds_max,
            width,
            height,
        );
        self.render(canvas);
    }

    pub fn zoom_and_render(
        &mut self,
        canvas: &web_sys::HtmlCanvasElement,
        delta_y: f32,
        delta_mode: u32,
        client_x: f32,
        client_y: f32,
    ) {
        let (_, _, dpr) = sync_canvas_size(canvas);
        let rect = canvas.get_bounding_client_rect();
        let cursor = [
            (client_x as f64 - rect.left()) as f32 * dpr,
            (client_y as f64 - rect.top()) as f32 * dpr,
        ];
        let pixel_delta_y = normalize_wheel_delta_y(delta_y, delta_mode, rect.height() as f32);
        let factor = 1.1_f32.powf(-pixel_delta_y / 100.0);
        self.camera.zoom_toward_cursor(
            factor,
            cursor,
            self.bounds_min,
            self.bounds_max,
            canvas.width().max(1),
            canvas.height().max(1),
        );
        self.render(canvas);
    }

    pub fn reset_and_render(&mut self, canvas: &web_sys::HtmlCanvasElement) {
        self.camera.reset();
        self.render(canvas);
    }

    pub fn render(&mut self, canvas: &web_sys::HtmlCanvasElement) {
        let (width, height, _) = sync_canvas_size(canvas);
        if self.gpu.size() != (width, height) {
            self.gpu.resize(width, height);
        }

        match self.gpu.begin_frame() {
            FrameOutcome::Skip => {}
            FrameOutcome::SurfaceLost => {
                log::error!("WebGPU surface was lost");
            }
            FrameOutcome::Ready(frame, view, mut encoder, reconfigure_after) => {
                if self.needs_upload {
                    self.pipeline.upload(
                        &self.gpu.device,
                        &self.gpu.queue,
                        &self.vertices,
                        self.color_params,
                    );
                    self.needs_upload = false;
                }
                self.pipeline.write_transform(
                    &self.gpu.queue,
                    self.camera.compute_transform(
                        self.bounds_min,
                        self.bounds_max,
                        width.max(1),
                        height.max(1),
                    ),
                );

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
                    self.pipeline.draw(&mut pass);
                }

                self.gpu.end_frame(*frame, encoder, reconfigure_after);
            }
        }
    }

    pub fn device_queue(&self) -> (Arc<wgpu::Device>, Arc<wgpu::Queue>) {
        (Arc::clone(&self.gpu.device), Arc::clone(&self.gpu.queue))
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
