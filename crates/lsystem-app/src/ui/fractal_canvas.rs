use iced::mouse;
use iced::widget::{container, shader};
use iced::{Background, Color, Element, Event, Length, Point, Rectangle, Size, Theme};
use lsystem_app_model::ConfigDefaults;
use lsystem_core::{ColorConfig, Config, Dimensions};
use lsystem_renderer::camera::Camera;
use lsystem_renderer::line_renderer::{
    ColorParams, LinePipeline2D, LinePipeline3D, Segment2D, Segment3D, TopologicalDepthSegment2D,
    TopologicalDepthSegment3D,
};
use lsystem_renderer::lsystem_bridge::{
    SegmentData, SegmentData3D, SegmentDataBuilder, SegmentDataBuilder3D,
    TopologicalDepthSegmentData, TopologicalDepthSegmentData3D, TopologicalDepthSegmentDataBuilder,
    TopologicalDepthSegmentDataBuilder3D, color_params_from_config,
};
use std::fmt;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use web_time::Instant;

use super::app_state::{FractalApp, Message};

const CANCELLATION_CHECK_INTERVAL: usize = 4096;

#[derive(Clone)]
enum SceneGeometry {
    TwoD {
        segments: Arc<Vec<Segment2D>>,
        bounds_min: [f32; 2],
        bounds_max: [f32; 2],
    },
    TwoDWithTopologicalDepth {
        segments: Arc<Vec<TopologicalDepthSegment2D>>,
        bounds_min: [f32; 2],
        bounds_max: [f32; 2],
        max_topological_depth: u32,
    },
    ThreeD {
        segments: Arc<Vec<Segment3D>>,
        bounds_min: [f32; 3],
        bounds_max: [f32; 3],
    },
    ThreeDWithTopologicalDepth {
        segments: Arc<Vec<TopologicalDepthSegment3D>>,
        bounds_min: [f32; 3],
        bounds_max: [f32; 3],
        max_topological_depth: u32,
    },
}

impl SceneGeometry {
    fn total_segments(&self) -> u32 {
        let segment_count = match self {
            SceneGeometry::TwoD { segments, .. } => segments.len(),
            SceneGeometry::TwoDWithTopologicalDepth { segments, .. } => segments.len(),
            SceneGeometry::ThreeD { segments, .. } => segments.len(),
            SceneGeometry::ThreeDWithTopologicalDepth { segments, .. } => segments.len(),
        };
        segment_count as u32
    }

    fn max_topological_depth(&self) -> Option<u32> {
        match self {
            Self::TwoD { .. } | Self::ThreeD { .. } => None,
            Self::TwoDWithTopologicalDepth {
                max_topological_depth,
                ..
            }
            | Self::ThreeDWithTopologicalDepth {
                max_topological_depth,
                ..
            } => Some(*max_topological_depth),
        }
    }
}

#[derive(Clone)]
pub(super) struct Scene {
    geometry: SceneGeometry,
    color_params: ColorParams,
    hue_offset_degrees: f32,
    background: [f32; 3],
    pub(super) camera: Camera,
    geometry_revision: u64,
    color_revision: u64,
}

impl Scene {
    fn from_segment_data_2d(
        colors: &ColorConfig,
        data: SegmentData,
        camera: Camera,
        revision: u64,
    ) -> Self {
        let geometry = SceneGeometry::TwoD {
            segments: Arc::new(data.segments),
            bounds_min: data.bounds_min,
            bounds_max: data.bounds_max,
        };
        Self {
            color_params: color_params_from_config(&colors.line, geometry.total_segments(), None),
            geometry,
            hue_offset_degrees: 0.0,
            background: colors.background.to_array(),
            camera,
            geometry_revision: revision,
            color_revision: 0,
        }
    }

    fn from_segment_data_3d(
        colors: &ColorConfig,
        data: SegmentData3D,
        camera: Camera,
        revision: u64,
    ) -> Self {
        let geometry = SceneGeometry::ThreeD {
            segments: Arc::new(data.segments),
            bounds_min: data.bounds_min,
            bounds_max: data.bounds_max,
        };
        Self {
            color_params: color_params_from_config(&colors.line, geometry.total_segments(), None),
            geometry,
            hue_offset_degrees: 0.0,
            background: colors.background.to_array(),
            camera,
            geometry_revision: revision,
            color_revision: 0,
        }
    }

    fn from_depth_segment_data_2d(
        colors: &ColorConfig,
        data: TopologicalDepthSegmentData,
        camera: Camera,
        revision: u64,
    ) -> Self {
        let max_topological_depth = data.max_topological_depth();
        let geometry = SceneGeometry::TwoDWithTopologicalDepth {
            segments: Arc::new(data.segments),
            bounds_min: data.bounds_min,
            bounds_max: data.bounds_max,
            max_topological_depth,
        };
        Self {
            color_params: color_params_from_config(
                &colors.line,
                geometry.total_segments(),
                geometry.max_topological_depth(),
            ),
            geometry,
            hue_offset_degrees: 0.0,
            background: colors.background.to_array(),
            camera,
            geometry_revision: revision,
            color_revision: 0,
        }
    }

    fn from_depth_segment_data_3d(
        colors: &ColorConfig,
        data: TopologicalDepthSegmentData3D,
        camera: Camera,
        revision: u64,
    ) -> Self {
        let max_topological_depth = data.max_topological_depth();
        let geometry = SceneGeometry::ThreeDWithTopologicalDepth {
            segments: Arc::new(data.segments),
            bounds_min: data.bounds_min,
            bounds_max: data.bounds_max,
            max_topological_depth,
        };
        Self {
            color_params: color_params_from_config(
                &colors.line,
                geometry.total_segments(),
                geometry.max_topological_depth(),
            ),
            geometry,
            hue_offset_degrees: 0.0,
            background: colors.background.to_array(),
            camera,
            geometry_revision: revision,
            color_revision: 0,
        }
    }

    pub(super) fn is_3d(&self) -> bool {
        matches!(
            self.geometry,
            SceneGeometry::ThreeD { .. } | SceneGeometry::ThreeDWithTopologicalDepth { .. }
        )
    }

    pub(super) fn reset_camera(&mut self) {
        self.camera.reset();
    }

    pub(super) fn pan_by_pixels(&mut self, dx: f32, dy: f32, size: Size) {
        if let SceneGeometry::TwoD {
            bounds_min,
            bounds_max,
            ..
        }
        | SceneGeometry::TwoDWithTopologicalDepth {
            bounds_min,
            bounds_max,
            ..
        } = &self.geometry
        {
            self.camera.pan_by_pixels(
                dx,
                dy,
                *bounds_min,
                *bounds_max,
                size.width.max(1.0) as u32,
                size.height.max(1.0) as u32,
            );
        }
    }

    pub(super) fn orbit_by_pixels(&mut self, dx: f32, dy: f32) {
        self.camera.orbit_by_pixels(dx, dy);
    }

    pub(super) fn orbit_by(&mut self, d_az: f32, d_el: f32) {
        self.camera.orbit_by(d_az, d_el);
    }

    pub(super) fn roll_by(&mut self, degrees: f32) {
        self.camera.roll_by(degrees);
    }

    pub(super) fn auto_rotate_by(&mut self, degrees: f32) {
        self.camera.auto_rotate_by(degrees);
    }

    pub(super) fn zoom_toward_cursor(&mut self, delta_y: f32, cursor: Point, size: Size) {
        let factor = 1.1_f32.powf(-delta_y / 100.0);
        match &self.geometry {
            SceneGeometry::TwoD {
                bounds_min,
                bounds_max,
                ..
            }
            | SceneGeometry::TwoDWithTopologicalDepth {
                bounds_min,
                bounds_max,
                ..
            } => {
                self.camera.zoom_toward_cursor(
                    factor,
                    [cursor.x, cursor.y],
                    *bounds_min,
                    *bounds_max,
                    size.width.max(1.0) as u32,
                    size.height.max(1.0) as u32,
                );
            }
            SceneGeometry::ThreeD { .. } | SceneGeometry::ThreeDWithTopologicalDepth { .. } => {
                self.camera.zoom_3d(factor);
            }
        }
    }

    pub(super) fn update_colors(&mut self, colors: &ColorConfig) {
        let total_segments = self.geometry.total_segments();
        self.color_params = color_params_from_config(
            &colors.line,
            total_segments,
            self.geometry.max_topological_depth(),
        );
        self.hue_offset_degrees = 0.0;
        self.background = colors.background.to_array();
        self.color_revision = self.color_revision.wrapping_add(1);
    }

    pub(super) fn set_hue_offset_degrees(&mut self, offset: f32) {
        let offset = offset.rem_euclid(360.0);
        if self.hue_offset_degrees != offset {
            self.hue_offset_degrees = offset;
            self.color_revision = self.color_revision.wrapping_add(1);
        }
    }

    #[cfg(test)]
    pub(super) fn hue_offset_degrees(&self) -> f32 {
        self.hue_offset_degrees
    }

    fn snapshot(&self) -> SceneSnapshot {
        SceneSnapshot {
            geometry: self.geometry.clone(),
            color_params: self
                .color_params
                .with_hue_offset_degrees(self.hue_offset_degrees),
            camera: self.camera.clone(),
            geometry_revision: self.geometry_revision,
            color_revision: self.color_revision,
        }
    }
}

#[derive(Clone)]
pub(super) enum SceneBuildResult {
    Ready { generation: u64, scene: Scene },
    Cancelled,
}

impl fmt::Debug for SceneBuildResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ready { generation, scene } => {
                let segment_count = match &scene.geometry {
                    SceneGeometry::TwoD { segments, .. } => segments.len(),
                    SceneGeometry::TwoDWithTopologicalDepth { segments, .. } => segments.len(),
                    SceneGeometry::ThreeD { segments, .. } => segments.len(),
                    SceneGeometry::ThreeDWithTopologicalDepth { segments, .. } => segments.len(),
                };
                f.debug_struct("Ready")
                    .field("generation", generation)
                    .field("segments", &segment_count)
                    .finish()
            }
            Self::Cancelled => f.write_str("Cancelled"),
        }
    }
}

pub(super) async fn build_scene(
    config: Config,
    generation: u64,
    current_generation: Arc<AtomicU64>,
    prev_camera: Camera,
) -> SceneBuildResult {
    let mut camera = prev_camera;
    camera.reset_position();
    let colors = config.colors;
    let started = Instant::now();

    match config.generation.dimensions {
        Dimensions::ThreeD => {
            // Depth geometry is decided by fractal structure, not color mode, so color
            // changes after this build never require a geometry rebuild.
            let use_topological_depth = config.generation.has_stack_directives();
            let mut segments_seen = 0usize;

            if use_topological_depth {
                let mut builder = TopologicalDepthSegmentDataBuilder3D::new();
                for segment in lsystem_core::generate_3d_with_topological_depth(&config.generation)
                {
                    if segments_seen.is_multiple_of(CANCELLATION_CHECK_INTERVAL) {
                        if is_cancelled(generation, &current_generation) {
                            return SceneBuildResult::Cancelled;
                        }
                        yield_generation().await;
                        if is_cancelled(generation, &current_generation) {
                            return SceneBuildResult::Cancelled;
                        }
                    }
                    builder.push_segment(segment);
                    segments_seen = segments_seen.wrapping_add(1);
                }

                if is_cancelled(generation, &current_generation) {
                    return SceneBuildResult::Cancelled;
                }

                log_generation_duration(started, segments_seen);

                SceneBuildResult::Ready {
                    generation,
                    scene: Scene::from_depth_segment_data_3d(
                        &colors,
                        builder.finish(),
                        camera,
                        generation,
                    ),
                }
            } else {
                let mut builder = SegmentDataBuilder3D::new();
                for segment in lsystem_core::generate_3d(&config.generation) {
                    if segments_seen.is_multiple_of(CANCELLATION_CHECK_INTERVAL) {
                        if is_cancelled(generation, &current_generation) {
                            return SceneBuildResult::Cancelled;
                        }
                        yield_generation().await;
                        if is_cancelled(generation, &current_generation) {
                            return SceneBuildResult::Cancelled;
                        }
                    }
                    builder.push_segment(segment);
                    segments_seen = segments_seen.wrapping_add(1);
                }

                if is_cancelled(generation, &current_generation) {
                    return SceneBuildResult::Cancelled;
                }

                log_generation_duration(started, segments_seen);

                SceneBuildResult::Ready {
                    generation,
                    scene: Scene::from_segment_data_3d(
                        &colors,
                        builder.finish(),
                        camera,
                        generation,
                    ),
                }
            }
        }
        Dimensions::TwoD => {
            // Depth geometry is decided by fractal structure, not color mode, so color
            // changes after this build never require a geometry rebuild.
            let use_topological_depth = config.generation.has_stack_directives();
            let mut segments_seen = 0usize;

            if use_topological_depth {
                let mut builder = TopologicalDepthSegmentDataBuilder::new();
                for segment in lsystem_core::generate_with_topological_depth(&config.generation) {
                    if segments_seen.is_multiple_of(CANCELLATION_CHECK_INTERVAL) {
                        if is_cancelled(generation, &current_generation) {
                            return SceneBuildResult::Cancelled;
                        }
                        yield_generation().await;
                        if is_cancelled(generation, &current_generation) {
                            return SceneBuildResult::Cancelled;
                        }
                    }
                    builder.push_segment(segment);
                    segments_seen = segments_seen.wrapping_add(1);
                }

                if is_cancelled(generation, &current_generation) {
                    return SceneBuildResult::Cancelled;
                }

                log_generation_duration(started, segments_seen);

                SceneBuildResult::Ready {
                    generation,
                    scene: Scene::from_depth_segment_data_2d(
                        &colors,
                        builder.finish(),
                        camera,
                        generation,
                    ),
                }
            } else {
                let mut builder = SegmentDataBuilder::new();
                for segment in lsystem_core::generate(&config.generation) {
                    if segments_seen.is_multiple_of(CANCELLATION_CHECK_INTERVAL) {
                        if is_cancelled(generation, &current_generation) {
                            return SceneBuildResult::Cancelled;
                        }
                        yield_generation().await;
                        if is_cancelled(generation, &current_generation) {
                            return SceneBuildResult::Cancelled;
                        }
                    }
                    builder.push_segment(segment);
                    segments_seen = segments_seen.wrapping_add(1);
                }

                if is_cancelled(generation, &current_generation) {
                    return SceneBuildResult::Cancelled;
                }

                log_generation_duration(started, segments_seen);

                SceneBuildResult::Ready {
                    generation,
                    scene: Scene::from_segment_data_2d(
                        &colors,
                        builder.finish(),
                        camera,
                        generation,
                    ),
                }
            }
        }
    }
}

fn log_generation_duration(started: Instant, segment_count: usize) {
    let elapsed = started.elapsed();
    log::info!(
        "generation_wall_ms={:.2} segments={segment_count}",
        elapsed.as_secs_f64() * 1000.0,
    );
}

fn is_cancelled(generation: u64, current_generation: &AtomicU64) -> bool {
    current_generation.load(Ordering::Acquire) != generation
}

#[cfg(not(target_arch = "wasm32"))]
async fn yield_generation() {
    std::thread::yield_now();
}

#[cfg(target_arch = "wasm32")]
async fn yield_generation() {
    gloo_timers::future::TimeoutFuture::new(0).await;
}

impl Default for Scene {
    fn default() -> Self {
        Self {
            geometry: SceneGeometry::TwoD {
                segments: Arc::new(Vec::new()),
                bounds_min: [-1.0, -1.0],
                bounds_max: [1.0, 1.0],
            },
            color_params: ColorParams::default(),
            hue_offset_degrees: 0.0,
            background: ConfigDefaults::embedded().colors.background.to_array(),
            camera: Camera::new(),
            geometry_revision: 0,
            color_revision: 0,
        }
    }
}

impl FractalApp {
    pub(super) fn fractal_view(&self) -> Element<'_, Message> {
        let [r, g, b] = self.scene.background;
        let background = Color::from_rgb(r, g, b);

        container(
            shader::Shader::new(FractalProgram {
                scene: self.scene.snapshot(),
            })
            .width(Length::Fill)
            .height(Length::Fill),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme: &Theme| container::Style {
            background: Some(Background::Color(background)),
            ..Default::default()
        })
        .into()
    }
}

#[derive(Clone)]
struct SceneSnapshot {
    geometry: SceneGeometry,
    color_params: ColorParams,
    camera: Camera,
    geometry_revision: u64,
    color_revision: u64,
}

impl SceneSnapshot {
    fn is_3d(&self) -> bool {
        matches!(
            self.geometry,
            SceneGeometry::ThreeD { .. } | SceneGeometry::ThreeDWithTopologicalDepth { .. }
        )
    }
}

struct FractalProgram {
    scene: SceneSnapshot,
}

#[derive(Default)]
struct FractalState {
    dragging: bool,
    last_cursor: Option<Point>,
}

impl shader::Program<Message> for FractalProgram {
    type State = FractalState;
    type Primitive = FractalPrimitive;

    fn update(
        &self,
        state: &mut Self::State,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<shader::Action<Message>> {
        let is_3d = self.scene.is_3d();
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let position = cursor_over(cursor, bounds)?;
                state.dragging = true;
                state.last_cursor = Some(position);
                Some(shader::Action::capture())
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) if state.dragging => {
                state.dragging = false;
                state.last_cursor = None;
                Some(shader::Action::capture())
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) if state.dragging => {
                let position = cursor_position(cursor)?;
                let previous = state.last_cursor.replace(position).unwrap_or(position);
                let dx = position.x - previous.x;
                let dy = position.y - previous.y;
                let msg = if is_3d {
                    Message::FractalOrbit { dx, dy }
                } else {
                    Message::FractalPan {
                        dx,
                        dy,
                        size: bounds.size(),
                    }
                };
                Some(shader::Action::publish(msg).and_capture())
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let position = cursor_over(cursor, bounds)?;
                Some(
                    shader::Action::publish(Message::FractalZoom {
                        delta_y: scroll_delta_y(*delta),
                        cursor: Point::new(position.x - bounds.x, position.y - bounds.y),
                        size: bounds.size(),
                    })
                    .and_capture(),
                )
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        _state: &Self::State,
        _cursor: mouse::Cursor,
        _bounds: Rectangle,
    ) -> Self::Primitive {
        FractalPrimitive {
            scene: self.scene.clone(),
        }
    }

    fn mouse_interaction(
        &self,
        state: &Self::State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if state.dragging || cursor_over(cursor, bounds).is_some() {
            mouse::Interaction::Grabbing
        } else {
            mouse::Interaction::default()
        }
    }
}

fn cursor_over(cursor: mouse::Cursor, bounds: Rectangle) -> Option<Point> {
    let position = cursor_position(cursor)?;
    bounds.contains(position).then_some(position)
}

fn cursor_position(cursor: mouse::Cursor) -> Option<Point> {
    match cursor {
        mouse::Cursor::Available(position) | mouse::Cursor::Levitating(position) => Some(position),
        mouse::Cursor::Unavailable => None,
    }
}

fn scroll_delta_y(delta: mouse::ScrollDelta) -> f32 {
    match delta {
        mouse::ScrollDelta::Lines { y, .. } => y * 40.0,
        mouse::ScrollDelta::Pixels { y, .. } => y,
    }
}

struct FractalPrimitive {
    scene: SceneSnapshot,
}

impl fmt::Debug for FractalPrimitive {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FractalPrimitive")
            .field("geometry_revision", &self.scene.geometry_revision)
            .field("color_revision", &self.scene.color_revision)
            .field("is_3d", &self.scene.is_3d())
            .finish()
    }
}

impl shader::Primitive for FractalPrimitive {
    type Pipeline = FractalPipeline;

    fn prepare(
        &self,
        pipeline: &mut Self::Pipeline,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bounds: &Rectangle,
        viewport: &shader::Viewport,
    ) {
        let scale_factor = viewport.scale_factor();
        let width = (bounds.width * scale_factor).round().max(1.0) as u32;
        let height = (bounds.height * scale_factor).round().max(1.0) as u32;

        let color_params = self.scene.color_params;
        let geometry_rev = self.scene.geometry_revision;
        let color_rev = self.scene.color_revision;
        match &self.scene.geometry {
            SceneGeometry::TwoD {
                segments,
                bounds_min,
                bounds_max,
            } => {
                let transform =
                    self.scene
                        .camera
                        .compute_transform(*bounds_min, *bounds_max, width, height);
                pipeline.sync_uploads(
                    geometry_rev,
                    color_rev,
                    |p| p.pipeline_2d.upload(device, queue, segments, color_params),
                    |p| p.pipeline_2d.write_color_params(queue, color_params),
                );
                pipeline.pipeline_2d.write_transform(queue, transform);
            }
            SceneGeometry::TwoDWithTopologicalDepth {
                segments,
                bounds_min,
                bounds_max,
                ..
            } => {
                let transform =
                    self.scene
                        .camera
                        .compute_transform(*bounds_min, *bounds_max, width, height);
                pipeline.sync_uploads(
                    geometry_rev,
                    color_rev,
                    |p| {
                        p.pipeline_2d.upload_with_topological_depth(
                            device,
                            queue,
                            segments,
                            color_params,
                        )
                    },
                    |p| p.pipeline_2d.write_color_params(queue, color_params),
                );
                pipeline.pipeline_2d.write_transform(queue, transform);
            }
            SceneGeometry::ThreeD {
                segments,
                bounds_min,
                bounds_max,
            } => {
                let mvp = self
                    .scene
                    .camera
                    .compute_mvp_3d(*bounds_min, *bounds_max, width, height);
                pipeline.sync_uploads(
                    geometry_rev,
                    color_rev,
                    |p| p.pipeline_3d.upload(device, queue, segments, color_params),
                    |p| p.pipeline_3d.write_color_params(queue, color_params),
                );
                pipeline.pipeline_3d.write_mvp(queue, mvp);
            }
            SceneGeometry::ThreeDWithTopologicalDepth {
                segments,
                bounds_min,
                bounds_max,
                ..
            } => {
                let mvp = self
                    .scene
                    .camera
                    .compute_mvp_3d(*bounds_min, *bounds_max, width, height);
                pipeline.sync_uploads(
                    geometry_rev,
                    color_rev,
                    |p| {
                        p.pipeline_3d.upload_with_topological_depth(
                            device,
                            queue,
                            segments,
                            color_params,
                        )
                    },
                    |p| p.pipeline_3d.write_color_params(queue, color_params),
                );
                pipeline.pipeline_3d.write_mvp(queue, mvp);
            }
        }
    }

    fn draw(&self, pipeline: &Self::Pipeline, render_pass: &mut wgpu::RenderPass<'_>) -> bool {
        match &self.scene.geometry {
            SceneGeometry::TwoD { .. } | SceneGeometry::TwoDWithTopologicalDepth { .. } => {
                pipeline.pipeline_2d.draw(render_pass)
            }
            SceneGeometry::ThreeD { .. } | SceneGeometry::ThreeDWithTopologicalDepth { .. } => {
                pipeline.pipeline_3d.draw(render_pass)
            }
        }
        true
    }
}

struct UploadedRevisions {
    geometry: u64,
    color: u64,
}

struct FractalPipeline {
    pipeline_2d: LinePipeline2D,
    pipeline_3d: LinePipeline3D,
    uploaded: Option<UploadedRevisions>,
}

impl shader::Pipeline for FractalPipeline {
    fn new(device: &wgpu::Device, _queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        Self {
            pipeline_2d: LinePipeline2D::new(device, format),
            pipeline_3d: LinePipeline3D::new(device, format),
            uploaded: None,
        }
    }
}

impl FractalPipeline {
    fn sync_uploads(
        &mut self,
        geometry_rev: u64,
        color_rev: u64,
        upload: impl FnOnce(&mut Self),
        write_color: impl FnOnce(&mut Self),
    ) {
        if self
            .uploaded
            .as_ref()
            .is_none_or(|r| r.geometry != geometry_rev)
        {
            upload(self);
            self.uploaded = Some(UploadedRevisions {
                geometry: geometry_rev,
                color: color_rev,
            });
        } else if self.uploaded.as_ref().is_some_and(|r| r.color != color_rev) {
            write_color(self);
            if let Some(u) = &mut self.uploaded {
                u.color = color_rev;
            }
        }
    }
}
