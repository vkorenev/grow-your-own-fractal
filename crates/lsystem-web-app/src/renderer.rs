use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use web_time::{Duration, Instant};

use glam::Vec2;
use lsystem_core::{AnyCompiledGeneration, Config};
use lsystem_renderer::camera::Camera;
use lsystem_renderer::line_renderer::{
    ColorParams, FrameOutcome, FrameSkipReason, GpuContext, GpuInitError, LINE_DEPTH_FORMAT,
    LinePipeline2D, LinePipeline3D, SurfaceFrame,
};
use lsystem_renderer::lsystem_bridge::{color_params_from_config, rgb_to_wgpu_color};
use lsystem_renderer::scene_upload::{
    GenerationMethod, SceneUploadError, SegmentLayout, UploadedScene2D, UploadedScene3D,
    upload_scene,
};

/// Successful scene metadata by dimension, or the absence of any upload.
enum ActiveScene {
    NoUpload,
    TwoD(UploadedScene2D),
    ThreeD(UploadedScene3D),
}

impl ActiveScene {
    fn total_segments(&self) -> u32 {
        match self {
            Self::NoUpload => 0,
            Self::TwoD(scene) => scene.total_segments(),
            Self::ThreeD(scene) => scene.total_segments(),
        }
    }

    fn max_topological_depth(&self) -> Option<u32> {
        match self {
            Self::NoUpload => None,
            Self::TwoD(scene) => scene.layout().max_topological_depth(),
            Self::ThreeD(scene) => scene.layout().max_topological_depth(),
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
    upload_started: Option<Instant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderStatus {
    Rendered,
    SurfaceLost,
    Skipped(FrameSkipReason),
}

pub(crate) struct RebuildRenderOutcome {
    pub(crate) render_status: RenderStatus,
    pub(crate) rebuild_result: Result<(), SceneUploadError>,
}

impl RenderStatus {
    /// Returns the skip reason if the frame was skipped for a reason other than
    /// the routine timeout/occlusion cases.
    pub fn unexpected_skip_reason(self) -> Option<FrameSkipReason> {
        match self {
            Self::Skipped(reason)
                if !matches!(reason, FrameSkipReason::Timeout | FrameSkipReason::Occluded) =>
            {
                Some(reason)
            }
            _ => None,
        }
    }
}

impl CanvasRenderer {
    pub async fn new(canvas: web_sys::HtmlCanvasElement) -> Result<Self, GpuInitError> {
        let (width, height, _) = sync_canvas_size(&canvas);
        let gpu = GpuContext::new(wgpu::SurfaceTarget::Canvas(canvas), width, height).await?;
        let pipeline_2d = LinePipeline2D::new(&gpu.device, gpu.surface_format());
        let pipeline_3d =
            LinePipeline3D::new(&gpu.device, gpu.surface_format(), Some(LINE_DEPTH_FORMAT));
        Ok(Self {
            gpu,
            pipeline_2d,
            pipeline_3d,
            camera: Camera::new(),
            scene: ActiveScene::NoUpload,
            color_params: ColorParams::default(),
            hue_offset_degrees: 0.0,
            background: wgpu::Color::BLACK,
            upload_started: None,
        })
    }

    pub fn set_config_and_render(
        &mut self,
        canvas: &web_sys::HtmlCanvasElement,
        config: &Config,
    ) -> RebuildRenderOutcome {
        let rebuild_result = self.rebuild_scene(config);
        if rebuild_result.is_ok() {
            self.camera.reset();
        }
        RebuildRenderOutcome {
            render_status: self.render(canvas),
            rebuild_result,
        }
    }

    pub fn set_config_preserving_camera_and_render(
        &mut self,
        canvas: &web_sys::HtmlCanvasElement,
        config: &Config,
    ) -> RebuildRenderOutcome {
        let rebuild_result = self.rebuild_scene(config);
        RebuildRenderOutcome {
            render_status: self.render(canvas),
            rebuild_result,
        }
    }

    fn rebuild_scene(&mut self, config: &Config) -> Result<(), SceneUploadError> {
        let started = Instant::now();
        let line = &config.colors.line;
        let result = match config.generation.compile() {
            AnyCompiledGeneration::ThreeD(generation) => upload_scene(
                &mut self.pipeline_3d,
                &self.gpu.device,
                &self.gpu.queue,
                generation,
                line,
                SegmentLayout::TopologicalDepth,
            )
            .map(|scene| {
                let method = scene.method();
                (ActiveScene::ThreeD(scene), method)
            }),
            AnyCompiledGeneration::TwoD(generation) => upload_scene(
                &mut self.pipeline_2d,
                &self.gpu.device,
                &self.gpu.queue,
                generation,
                line,
                SegmentLayout::TopologicalDepth,
            )
            .map(|scene| {
                let method = scene.method();
                (ActiveScene::TwoD(scene), method)
            }),
        };
        match result {
            Ok((scene, method)) => {
                self.scene = scene;
                self.finish_successful_rebuild(config, started, method);
                Ok(())
            }
            Err(error) => {
                self.handle_rebuild_error(error);
                Err(error)
            }
        }
    }

    fn handle_rebuild_error(&mut self, error: SceneUploadError) {
        self.upload_started = None;
        self.scene = ActiveScene::NoUpload;
        log::error!("scene rebuild failed: {error}");
    }

    fn finish_successful_rebuild(
        &mut self,
        config: &Config,
        started: Instant,
        method: GenerationMethod,
    ) {
        let colors = config.colors;
        self.color_params = color_params_from_config(
            &colors.line,
            self.scene.total_segments(),
            self.scene.max_topological_depth(),
        );
        self.hue_offset_degrees = 0.0;
        self.background = rgb_to_wgpu_color(colors.background);
        let elapsed = started.elapsed();
        let template_iterations = match method {
            GenerationMethod::Stamped {
                template_iterations,
            } => template_iterations,
            GenerationMethod::Interpreted => 0,
        };
        log::info!(
            "generation_wall_ms={:.2} segments={} template_iterations={}",
            elapsed.as_secs_f64() * 1000.0,
            self.scene.total_segments(),
            template_iterations,
        );
        self.upload_started = Some(Instant::now());
    }

    pub fn set_colors_and_render(
        &mut self,
        canvas: &web_sys::HtmlCanvasElement,
        config: &Config,
    ) -> RenderStatus {
        let colors = config.colors;
        self.color_params = color_params_from_config(
            &colors.line,
            self.scene.total_segments(),
            self.scene.max_topological_depth(),
        );
        self.background = rgb_to_wgpu_color(colors.background);
        self.write_color_params();
        self.render(canvas)
    }

    pub fn animate_and_render(
        &mut self,
        canvas: &web_sys::HtmlCanvasElement,
        auto_rotate_degrees: Option<f32>,
        hue_offset_degrees: Option<f32>,
    ) -> RenderStatus {
        if let (ActiveScene::ThreeD(_), Some(degrees)) = (&self.scene, auto_rotate_degrees) {
            self.camera.auto_rotate_by(degrees);
        }
        if !matches!(self.scene, ActiveScene::NoUpload)
            && let Some(offset) = hue_offset_degrees
        {
            self.set_hue_offset_degrees(offset);
        }
        self.render(canvas)
    }

    pub fn drag_and_render(
        &mut self,
        canvas: &web_sys::HtmlCanvasElement,
        css_delta: Vec2,
    ) -> RenderStatus {
        match &self.scene {
            ActiveScene::NoUpload => {}
            ActiveScene::TwoD(scene) => {
                let (_, _, dpr) = sync_canvas_size(canvas);
                let width = canvas.width().max(1);
                let height = canvas.height().max(1);
                self.camera
                    .pan_by_pixels(css_delta * dpr, scene.bounds(), width, height);
            }
            ActiveScene::ThreeD(_) => {
                self.camera.orbit_by_pixels(css_delta);
            }
        }
        self.render(canvas)
    }

    pub fn zoom_and_render(
        &mut self,
        canvas: &web_sys::HtmlCanvasElement,
        delta_y: f32,
        delta_mode: u32,
        client: Vec2,
    ) -> RenderStatus {
        let rect = canvas.get_bounding_client_rect();
        let pixel_delta_y = normalize_wheel_delta_y(delta_y, delta_mode, rect.height() as f32);
        let factor = 1.1_f32.powf(-pixel_delta_y / 100.0);
        self.zoom_by_factor_and_render(canvas, factor, client)
    }

    pub fn zoom_by_factor_and_render(
        &mut self,
        canvas: &web_sys::HtmlCanvasElement,
        factor: f32,
        client: Vec2,
    ) -> RenderStatus {
        let (_, _, dpr) = sync_canvas_size(canvas);
        let rect = canvas.get_bounding_client_rect();
        match &self.scene {
            ActiveScene::NoUpload => {}
            ActiveScene::TwoD(scene) => {
                let cursor = Vec2::new(
                    (client.x as f64 - rect.left()) as f32 * dpr,
                    (client.y as f64 - rect.top()) as f32 * dpr,
                );
                self.camera.zoom_toward_cursor(
                    factor,
                    cursor,
                    scene.bounds(),
                    canvas.width().max(1),
                    canvas.height().max(1),
                );
            }
            ActiveScene::ThreeD(_) => {
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
        if matches!(self.scene, ActiveScene::ThreeD(_)) {
            self.camera.orbit_by(d_az, d_el);
        }
        self.render(canvas)
    }

    pub fn roll_and_render(
        &mut self,
        canvas: &web_sys::HtmlCanvasElement,
        degrees: f32,
    ) -> RenderStatus {
        if matches!(self.scene, ActiveScene::ThreeD(_)) {
            self.camera.roll_by(degrees);
        }
        self.render(canvas)
    }

    pub fn reset_and_render(&mut self, canvas: &web_sys::HtmlCanvasElement) -> RenderStatus {
        if !matches!(self.scene, ActiveScene::NoUpload) {
            self.camera.reset();
        }
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
        self.pipeline_3d =
            LinePipeline3D::new(&gpu.device, gpu.surface_format(), Some(LINE_DEPTH_FORMAT));
        self.gpu = gpu;
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

        let started = self.upload_started.take();

        match &self.scene {
            ActiveScene::NoUpload => {}
            ActiveScene::TwoD(scene) => {
                self.pipeline_2d.write_transform(
                    &self.gpu.queue,
                    self.camera
                        .compute_transform(scene.bounds(), width.max(1), height.max(1)),
                );
            }
            ActiveScene::ThreeD(scene) => {
                self.pipeline_3d.write_mvp(
                    &self.gpu.queue,
                    self.camera
                        .compute_mvp_3d(scene.bounds(), width.max(1), height.max(1)),
                );
            }
        }

        {
            // 3D pipelines are built with a matching DepthStencilState, 2D pipelines
            // are not — this attachment must stay derived from the same `self.scene`
            // as the draw dispatch below, so the two can never disagree.
            let is_3d = matches!(self.scene, ActiveScene::ThreeD(_));
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
                depth_stencil_attachment: is_3d.then_some(wgpu::RenderPassDepthStencilAttachment {
                    view: self.gpu.depth_view(),
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });
            match &self.scene {
                ActiveScene::NoUpload => {}
                ActiveScene::TwoD(_) => self.pipeline_2d.draw(&mut pass),
                ActiveScene::ThreeD(_) => self.pipeline_3d.draw(&mut pass),
            }
        }

        self.gpu
            .end_frame(*frame, encoder, reconfigure_after_present);

        if let Some(started) = started {
            let segment_count = self.scene.total_segments();
            let device = Arc::clone(&self.gpu.device);
            let done = Arc::new(AtomicBool::new(false));
            let done_for_callback = Arc::clone(&done);
            // Registered synchronously, right after the submit it tracks, so no later
            // submission can be folded into "previously submitted work" by the time a
            // deferred async task gets around to registering it.
            self.gpu.queue.on_submitted_work_done(move || {
                done_for_callback.store(true, Ordering::Release);
            });
            wasm_bindgen_futures::spawn_local(async move {
                if !await_submitted_work_done(&device, &done).await {
                    log::warn!(
                        "geometry_upload_frame_gpu_ms: gave up waiting for GPU completion \
                         after {SUBMITTED_WORK_DONE_TIMEOUT:?}; segments={segment_count}"
                    );
                    return;
                }
                let elapsed = started.elapsed();
                log::info!(
                    "geometry_upload_frame_gpu_ms={:.2} segments={segment_count} \
                     scope=\"wall time from successful rebuild completion through next submitted \
                     frame GPU completion; includes queued staging copy and draw; excludes CPU \
                     generation/staging fill and present\"",
                    elapsed.as_secs_f64() * 1000.0,
                );
            });
        }
    }

    fn set_hue_offset_degrees(&mut self, offset: f32) {
        let offset = offset.rem_euclid(360.0);
        if self.hue_offset_degrees != offset {
            self.hue_offset_degrees = offset;
            self.write_color_params();
        }
    }

    fn effective_color_params(&self) -> ColorParams {
        self.color_params
            .with_hue_offset_degrees(self.hue_offset_degrees)
    }

    fn write_color_params(&self) {
        let color_params = self.effective_color_params();
        match &self.scene {
            ActiveScene::NoUpload => {}
            ActiveScene::TwoD(_) => {
                self.pipeline_2d
                    .write_color_params(&self.gpu.queue, color_params);
            }
            ActiveScene::ThreeD(_) => {
                self.pipeline_3d
                    .write_color_params(&self.gpu.queue, color_params);
            }
        }
    }
}

/// Bounds how long a `geometry_upload_frame_gpu_ms` wait can run. `Device::poll` with
/// `PollType::Poll` cannot report device/context loss (its `Result` only ever carries
/// `PollError::Timeout`/`WrongSubmissionIndex`, which don't apply here), so a dead device
/// after, e.g., a lost WebGL context would otherwise spin this loop forever. This timeout
/// is the only backstop against that.
const SUBMITTED_WORK_DONE_TIMEOUT: Duration = Duration::from_secs(30);

/// Polls `device` until `done` is set by an already-registered `on_submitted_work_done`
/// callback, or `SUBMITTED_WORK_DONE_TIMEOUT` elapses. Returns `false` on timeout.
async fn await_submitted_work_done(device: &wgpu::Device, done: &AtomicBool) -> bool {
    let started = Instant::now();
    while !done.load(Ordering::Acquire) {
        if started.elapsed() > SUBMITTED_WORK_DONE_TIMEOUT {
            return false;
        }
        let _ = device.poll(wgpu::PollType::Poll);
        gloo_timers::future::TimeoutFuture::new(0).await;
    }
    true
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
