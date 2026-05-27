use std::sync::Arc;

use lsystem_core::{Config, Dimensions};
use lsystem_renderer::camera::Camera;
use lsystem_renderer::line_renderer::{
    ColorParams, FrameOutcome, FrameSkipReason, GpuContext, GpuInitError, LinePipeline2D,
    LinePipeline3D, Segment2D, Segment3D, SurfaceFrame, TopologicalDepthSegment2D,
    TopologicalDepthSegment3D,
};
use lsystem_renderer::lsystem_bridge::{
    SegmentData, SegmentData3D, TopologicalDepthSegmentData, TopologicalDepthSegmentData3D,
    color_params_from_config, geometry_to_depth_segments, geometry_to_depth_segments_3d,
    geometry_to_segments, geometry_to_segments_3d,
};

enum ActiveScene {
    TwoD {
        segments: Vec<Segment2D>,
        bounds_min: [f32; 2],
        bounds_max: [f32; 2],
    },
    TwoDWithTopologicalDepth {
        segments: Vec<TopologicalDepthSegment2D>,
        bounds_min: [f32; 2],
        bounds_max: [f32; 2],
        max_topological_depth: u32,
    },
    ThreeD {
        segments: Vec<Segment3D>,
        bounds_min: [f32; 3],
        bounds_max: [f32; 3],
    },
    ThreeDWithTopologicalDepth {
        segments: Vec<TopologicalDepthSegment3D>,
        bounds_min: [f32; 3],
        bounds_max: [f32; 3],
        max_topological_depth: u32,
    },
}

impl ActiveScene {
    fn total_segments(&self) -> u32 {
        let segment_count = match self {
            ActiveScene::TwoD { segments, .. } => segments.len(),
            ActiveScene::TwoDWithTopologicalDepth { segments, .. } => segments.len(),
            ActiveScene::ThreeD { segments, .. } => segments.len(),
            ActiveScene::ThreeDWithTopologicalDepth { segments, .. } => segments.len(),
        };
        segment_count as u32
    }

    fn max_topological_depth(&self) -> u32 {
        match self {
            Self::TwoD { segments, .. } => segments.len().saturating_sub(1) as u32,
            Self::ThreeD { segments, .. } => segments.len().saturating_sub(1) as u32,
            Self::TwoDWithTopologicalDepth {
                max_topological_depth,
                ..
            }
            | Self::ThreeDWithTopologicalDepth {
                max_topological_depth,
                ..
            } => *max_topological_depth,
        }
    }
}

pub struct CanvasRenderer {
    gpu: GpuContext,
    pipeline_2d: LinePipeline2D,
    pipeline_3d: LinePipeline3D,
    camera: Camera,
    scene: ActiveScene,
    color_params: ColorParams,
    hue_offset_degrees: f32,
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
                segments: Vec::new(),
                bounds_min: [-1.0, -1.0],
                bounds_max: [1.0, 1.0],
            },
            color_params: ColorParams::default(),
            hue_offset_degrees: 0.0,
            background: wgpu::Color::BLACK,
            needs_upload: false,
        })
    }

    pub fn set_config_and_render(
        &mut self,
        canvas: &web_sys::HtmlCanvasElement,
        config: &Config,
    ) -> RenderStatus {
        self.rebuild_scene(config);
        self.camera.reset();
        self.render(canvas)
    }

    pub fn set_config_preserving_camera_and_render(
        &mut self,
        canvas: &web_sys::HtmlCanvasElement,
        config: &Config,
    ) -> RenderStatus {
        self.rebuild_scene(config);
        self.render(canvas)
    }

    fn rebuild_scene(&mut self, config: &Config) {
        match config.generation.dimensions {
            Dimensions::ThreeD => {
                if config.generation.has_stack_directives() {
                    let TopologicalDepthSegmentData3D {
                        segments,
                        bounds_min,
                        bounds_max,
                        max_topological_depth,
                    } = geometry_to_depth_segments_3d(
                        lsystem_core::generate_3d_with_topological_depth(&config.generation),
                    );
                    self.scene = ActiveScene::ThreeDWithTopologicalDepth {
                        segments,
                        bounds_min,
                        bounds_max,
                        max_topological_depth,
                    };
                } else {
                    let SegmentData3D {
                        segments,
                        bounds_min,
                        bounds_max,
                    } = geometry_to_segments_3d(lsystem_core::generate_3d(&config.generation));
                    self.scene = ActiveScene::ThreeD {
                        segments,
                        bounds_min,
                        bounds_max,
                    };
                }
            }
            Dimensions::TwoD => {
                if config.generation.has_stack_directives() {
                    let TopologicalDepthSegmentData {
                        segments,
                        bounds_min,
                        bounds_max,
                        max_topological_depth,
                    } = geometry_to_depth_segments(lsystem_core::generate_with_topological_depth(
                        &config.generation,
                    ));
                    self.scene = ActiveScene::TwoDWithTopologicalDepth {
                        segments,
                        bounds_min,
                        bounds_max,
                        max_topological_depth,
                    };
                } else {
                    let SegmentData {
                        segments,
                        bounds_min,
                        bounds_max,
                    } = geometry_to_segments(lsystem_core::generate(&config.generation));
                    self.scene = ActiveScene::TwoD {
                        segments,
                        bounds_min,
                        bounds_max,
                    };
                }
            }
        }

        let colors = config.effective_colors();
        self.color_params = color_params_from_config(
            &colors.line,
            self.scene.total_segments(),
            self.scene.max_topological_depth(),
        );
        self.hue_offset_degrees = 0.0;
        let [r, g, b] = colors.effective_background();
        self.background = wgpu::Color {
            r: r as f64,
            g: g as f64,
            b: b as f64,
            a: 1.0,
        };
        self.needs_upload = true;
    }

    pub fn set_colors_and_render(
        &mut self,
        canvas: &web_sys::HtmlCanvasElement,
        config: &Config,
    ) -> RenderStatus {
        let colors = config.effective_colors();
        self.color_params = color_params_from_config(
            &colors.line,
            self.scene.total_segments(),
            self.scene.max_topological_depth(),
        );
        let [r, g, b] = colors.effective_background();
        self.background = wgpu::Color {
            r: r as f64,
            g: g as f64,
            b: b as f64,
            a: 1.0,
        };
        self.write_color_params();
        self.render(canvas)
    }

    pub fn set_hue_offset_degrees_and_render(
        &mut self,
        canvas: &web_sys::HtmlCanvasElement,
        offset: f32,
    ) -> RenderStatus {
        self.set_hue_offset_degrees(offset);
        self.render(canvas)
    }

    pub fn animate_and_render(
        &mut self,
        canvas: &web_sys::HtmlCanvasElement,
        auto_rotate_degrees: Option<f32>,
        hue_offset_degrees: Option<f32>,
    ) -> RenderStatus {
        if let Some(degrees) = auto_rotate_degrees {
            self.camera.auto_rotate_by(degrees);
        }
        if let Some(offset) = hue_offset_degrees {
            self.set_hue_offset_degrees(offset);
        }
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
            }
            | ActiveScene::TwoDWithTopologicalDepth {
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
            ActiveScene::ThreeD { .. } | ActiveScene::ThreeDWithTopologicalDepth { .. } => {
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
            }
            | ActiveScene::TwoDWithTopologicalDepth {
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
            ActiveScene::ThreeD { .. } | ActiveScene::ThreeDWithTopologicalDepth { .. } => {
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

    pub async fn recover_surface(
        &mut self,
        canvas: web_sys::HtmlCanvasElement,
    ) -> Result<(), GpuInitError> {
        let (width, height, _) = sync_canvas_size(&canvas);
        let gpu =
            GpuContext::new(wgpu::SurfaceTarget::Canvas(canvas.clone()), width, height).await?;
        self.pipeline_2d = LinePipeline2D::new(&gpu.device, gpu.surface_format());
        self.pipeline_3d = LinePipeline3D::new(&gpu.device, gpu.surface_format());
        self.gpu = gpu;
        self.needs_upload = true;
        Ok(())
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
                segments,
                bounds_min,
                bounds_max,
            } => {
                if self.needs_upload {
                    self.pipeline_2d.upload(
                        &self.gpu.device,
                        &self.gpu.queue,
                        segments,
                        self.effective_color_params(),
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
            ActiveScene::TwoDWithTopologicalDepth {
                segments,
                bounds_min,
                bounds_max,
                ..
            } => {
                if self.needs_upload {
                    self.pipeline_2d.upload_with_topological_depth(
                        &self.gpu.device,
                        &self.gpu.queue,
                        segments,
                        self.effective_color_params(),
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
                segments,
                bounds_min,
                bounds_max,
            } => {
                if self.needs_upload {
                    self.pipeline_3d.upload(
                        &self.gpu.device,
                        &self.gpu.queue,
                        segments,
                        self.effective_color_params(),
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
            ActiveScene::ThreeDWithTopologicalDepth {
                segments,
                bounds_min,
                bounds_max,
                ..
            } => {
                if self.needs_upload {
                    self.pipeline_3d.upload_with_topological_depth(
                        &self.gpu.device,
                        &self.gpu.queue,
                        segments,
                        self.effective_color_params(),
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
                ActiveScene::TwoD { .. } | ActiveScene::TwoDWithTopologicalDepth { .. } => {
                    self.pipeline_2d.draw(&mut pass)
                }
                ActiveScene::ThreeD { .. } | ActiveScene::ThreeDWithTopologicalDepth { .. } => {
                    self.pipeline_3d.draw(&mut pass)
                }
            }
        }

        self.gpu
            .end_frame(*frame, encoder, reconfigure_after_present);
    }

    fn set_hue_offset_degrees(&mut self, offset: f32) {
        let offset = offset.rem_euclid(360.0);
        if self.hue_offset_degrees != offset {
            self.hue_offset_degrees = offset;
            if !self.needs_upload {
                self.write_color_params();
            }
        }
    }

    fn effective_color_params(&self) -> ColorParams {
        self.color_params
            .with_hue_offset_degrees(self.hue_offset_degrees)
    }

    fn write_color_params(&self) {
        let color_params = self.effective_color_params();
        match &self.scene {
            ActiveScene::TwoD { .. } | ActiveScene::TwoDWithTopologicalDepth { .. } => {
                self.pipeline_2d
                    .write_color_params(&self.gpu.queue, color_params);
            }
            ActiveScene::ThreeD { .. } | ActiveScene::ThreeDWithTopologicalDepth { .. } => {
                self.pipeline_3d
                    .write_color_params(&self.gpu.queue, color_params);
            }
        }
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
