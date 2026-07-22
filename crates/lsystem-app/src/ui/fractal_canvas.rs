use glam::Vec2;
use iced::mouse;
use iced::widget::{container, shader};
use iced::{Background, Color, Element, Event, Length, Point, Rectangle, Size, Theme, window};
use lsystem_app_model::ConfigDefaults;
use lsystem_core::{
    AnyCompiledGeneration, BoundingCylinder3D, Bounds2D, ColorConfig, CompiledGeneration, Config,
    D2, D3, DEFAULT_TEMPLATE_SEGMENT_BUDGET, PreparedGeneration, TemplateDimension,
};
use lsystem_renderer::camera::Camera;
use lsystem_renderer::line_renderer::{
    ColorParams, LinePipeline2D, LinePipeline3D, RenderDimension, Segment2D, Segment3D,
    TopologicalDepthSegment2D, TopologicalDepthSegment3D,
};
use lsystem_renderer::lsystem_bridge::{
    CollectedDepthSegmentData, CollectedSegmentData, DepthSegmentDataBuilder,
    PlainSegmentDataBuilder, SegmentData2D, SegmentData3D, TopologicalDepthSegmentData2D,
    TopologicalDepthSegmentData3D, color_params_from_config,
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
        bounds: Bounds2D,
    },
    TwoDWithTopologicalDepth {
        segments: Arc<Vec<TopologicalDepthSegment2D>>,
        bounds: Bounds2D,
        max_topological_depth: u32,
    },
    ThreeD {
        segments: Arc<Vec<Segment3D>>,
        bounds: BoundingCylinder3D,
    },
    ThreeDWithTopologicalDepth {
        segments: Arc<Vec<TopologicalDepthSegment3D>>,
        bounds: BoundingCylinder3D,
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

/// Maps a dimension marker to its [`SceneGeometry`] variants. App-local so
/// the renderer stays ignorant of Iced scene types.
trait SceneDimension: RenderDimension + TemplateDimension {
    fn plain_geometry(data: CollectedSegmentData<Self::PlainRecord, Self::Bounds>)
    -> SceneGeometry;
    fn depth_geometry(
        data: CollectedDepthSegmentData<Self::DepthRecord, Self::Bounds>,
    ) -> SceneGeometry;
}

impl SceneDimension for D2 {
    fn plain_geometry(data: SegmentData2D) -> SceneGeometry {
        SceneGeometry::TwoD {
            segments: Arc::new(data.segments),
            bounds: data.bounds,
        }
    }

    fn depth_geometry(data: TopologicalDepthSegmentData2D) -> SceneGeometry {
        let max_topological_depth = data.max_topological_depth();
        SceneGeometry::TwoDWithTopologicalDepth {
            segments: Arc::new(data.segments),
            bounds: data.bounds,
            max_topological_depth,
        }
    }
}

impl SceneDimension for D3 {
    fn plain_geometry(data: SegmentData3D) -> SceneGeometry {
        SceneGeometry::ThreeD {
            segments: Arc::new(data.segments),
            bounds: data.bounds,
        }
    }

    fn depth_geometry(data: TopologicalDepthSegmentData3D) -> SceneGeometry {
        let max_topological_depth = data.max_topological_depth();
        SceneGeometry::ThreeDWithTopologicalDepth {
            segments: Arc::new(data.segments),
            bounds: data.bounds,
            max_topological_depth,
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
    fn from_geometry(
        colors: &ColorConfig,
        geometry: SceneGeometry,
        camera: Camera,
        revision: u64,
    ) -> Self {
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

    pub(super) fn pan_by_pixels(&mut self, delta: Vec2, size: Size) {
        if let SceneGeometry::TwoD { bounds, .. }
        | SceneGeometry::TwoDWithTopologicalDepth { bounds, .. } = &self.geometry
        {
            self.camera.pan_by_pixels(
                delta,
                *bounds,
                size.width.max(1.0) as u32,
                size.height.max(1.0) as u32,
            );
        }
    }

    pub(super) fn orbit_by_pixels(&mut self, delta: Vec2) {
        self.camera.orbit_by_pixels(delta);
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
            SceneGeometry::TwoD { bounds, .. }
            | SceneGeometry::TwoDWithTopologicalDepth { bounds, .. } => {
                self.camera.zoom_toward_cursor(
                    factor,
                    Vec2::new(cursor.x, cursor.y),
                    *bounds,
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
    generation_revision: u64,
    current_generation: Arc<AtomicU64>,
    prev_camera: Camera,
) -> SceneBuildResult {
    let mut camera = prev_camera;
    camera.reset_position();
    let colors = config.colors;
    let started = Instant::now();
    match config.generation.compile() {
        AnyCompiledGeneration::ThreeD(generation) => {
            build_typed_scene(
                generation,
                &colors,
                camera,
                generation_revision,
                &current_generation,
                started,
            )
            .await
        }
        AnyCompiledGeneration::TwoD(generation) => {
            build_typed_scene(
                generation,
                &colors,
                camera,
                generation_revision,
                &current_generation,
                started,
            )
            .await
        }
    }
}

async fn build_typed_scene<D: SceneDimension>(
    compiled_generation: CompiledGeneration<D>,
    colors: &ColorConfig,
    camera: Camera,
    generation_revision: u64,
    current_generation: &AtomicU64,
    started: Instant,
) -> SceneBuildResult {
    // Depth geometry is decided by fractal structure, not color mode, so color
    // changes after this build never require a geometry rebuild.
    let plan = compiled_generation.plan_templates(DEFAULT_TEMPLATE_SEGMENT_BUDGET);
    let use_topological_depth = plan.has_stack_directives();
    let template_iterations = plan.selected_template_iterations().unwrap_or(0);

    let prepared = plan.prepare();

    let (segment_count, geometry) = if use_topological_depth {
        let mut builder = DepthSegmentDataBuilder::<D>::new();
        let cancelled = match &prepared {
            PreparedGeneration::Stamped(set) => {
                drain_cancellable(
                    set.depth_segments(),
                    |segment| builder.push_segment(segment),
                    generation_revision,
                    current_generation,
                )
                .await
            }
            PreparedGeneration::Interpreted(generation) => {
                drain_cancellable(
                    generation.depth_segments(),
                    |segment| builder.push_segment(segment),
                    generation_revision,
                    current_generation,
                )
                .await
            }
        };
        if cancelled {
            return SceneBuildResult::Cancelled;
        }
        let data = builder.finish();
        (data.segments.len(), D::depth_geometry(data))
    } else {
        let mut builder = PlainSegmentDataBuilder::<D>::new();
        let cancelled = match &prepared {
            PreparedGeneration::Stamped(set) => {
                drain_cancellable(
                    set.segments(),
                    |segment| builder.push_segment(segment),
                    generation_revision,
                    current_generation,
                )
                .await
            }
            PreparedGeneration::Interpreted(generation) => {
                drain_cancellable(
                    generation.segments(),
                    |segment| builder.push_segment(segment),
                    generation_revision,
                    current_generation,
                )
                .await
            }
        };
        if cancelled {
            return SceneBuildResult::Cancelled;
        }
        let data = builder.finish();
        (data.segments.len(), D::plain_geometry(data))
    };

    if is_cancelled(generation_revision, current_generation) {
        return SceneBuildResult::Cancelled;
    }

    log_generation_duration(started, segment_count, template_iterations);

    SceneBuildResult::Ready {
        generation: generation_revision,
        scene: Scene::from_geometry(colors, geometry, camera, generation_revision),
    }
}

fn log_generation_duration(started: Instant, segment_count: usize, template_iterations: u16) {
    let elapsed = started.elapsed();
    log::info!(
        "generation_wall_ms={:.2} segments={segment_count} template_iterations={template_iterations}",
        elapsed.as_secs_f64() * 1000.0,
    );
}

async fn drain_cancellable<T>(
    iter: impl Iterator<Item = T>,
    mut push: impl FnMut(T),
    generation_revision: u64,
    current_generation: &AtomicU64,
) -> bool {
    let mut segments_seen = 0usize;
    for item in iter {
        if segments_seen.is_multiple_of(CANCELLATION_CHECK_INTERVAL) {
            if is_cancelled(generation_revision, current_generation) {
                return true;
            }
            yield_generation().await;
            if is_cancelled(generation_revision, current_generation) {
                return true;
            }
        }
        push(item);
        segments_seen = segments_seen.wrapping_add(1);
    }
    false
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
                bounds: D2::empty_scene_bounds(),
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
    orbit_dragging: bool,
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
                state.orbit_dragging = is_3d;
                state.last_cursor = Some(position);
                if state.orbit_dragging {
                    Some(shader::Action::publish(Message::FractalOrbitStarted).and_capture())
                } else {
                    Some(shader::Action::capture())
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) if state.dragging => {
                let orbit_dragging = state.orbit_dragging;
                state.dragging = false;
                state.orbit_dragging = false;
                state.last_cursor = None;
                if orbit_dragging {
                    Some(shader::Action::publish(Message::FractalOrbitEnded).and_capture())
                } else {
                    Some(shader::Action::capture())
                }
            }
            Event::Window(window::Event::Unfocused) if state.dragging => {
                let orbit_dragging = state.orbit_dragging;
                state.dragging = false;
                state.orbit_dragging = false;
                state.last_cursor = None;
                orbit_dragging.then(|| shader::Action::publish(Message::FractalOrbitEnded))
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) if state.dragging => {
                let position = cursor_position(cursor)?;
                let previous = state.last_cursor.replace(position).unwrap_or(position);
                let delta = Vec2::new(position.x - previous.x, position.y - previous.y);
                let msg = if state.orbit_dragging {
                    Message::FractalOrbit { delta }
                } else {
                    Message::FractalPan {
                        delta,
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
            SceneGeometry::TwoD { segments, bounds } => {
                let transform = self.scene.camera.compute_transform(*bounds, width, height);
                pipeline.sync_uploads(
                    geometry_rev,
                    color_rev,
                    |p| p.pipeline_2d.upload(device, queue, segments, color_params),
                    |p| p.pipeline_2d.write_color_params(queue, color_params),
                );
                pipeline.pipeline_2d.write_transform(queue, transform);
            }
            SceneGeometry::TwoDWithTopologicalDepth {
                segments, bounds, ..
            } => {
                let transform = self.scene.camera.compute_transform(*bounds, width, height);
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
            SceneGeometry::ThreeD { segments, bounds } => {
                let mvp = self.scene.camera.compute_mvp_3d(*bounds, width, height);
                pipeline.sync_uploads(
                    geometry_rev,
                    color_rev,
                    |p| p.pipeline_3d.upload(device, queue, segments, color_params),
                    |p| p.pipeline_3d.write_color_params(queue, color_params),
                );
                pipeline.pipeline_3d.write_mvp(queue, mvp);
            }
            SceneGeometry::ThreeDWithTopologicalDepth {
                segments, bounds, ..
            } => {
                let mvp = self.scene.camera.compute_mvp_3d(*bounds, width, height);
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use iced::futures::executor::block_on;
    use lsystem_core::{Dimensions, GenerationConfig, LineColorConfig, Rgb};

    use super::*;

    fn generation_config(dimensions: Dimensions, axiom: &str) -> GenerationConfig {
        GenerationConfig::new(
            dimensions,
            axiom.to_string(),
            0,
            90.0,
            1.0,
            0.0,
            BTreeMap::new(),
        )
        .expect("balanced config")
    }

    fn color_config() -> ColorConfig {
        ColorConfig {
            background: Rgb::new(0, 0, 0),
            line: LineColorConfig::Solid(Rgb::new(255, 255, 255)),
        }
    }

    fn compile_2d(config: &GenerationConfig) -> CompiledGeneration<D2> {
        match config.compile() {
            AnyCompiledGeneration::TwoD(generation) => generation,
            AnyCompiledGeneration::ThreeD(_) => panic!("expected 2D generation"),
        }
    }

    fn compile_3d(config: &GenerationConfig) -> CompiledGeneration<D3> {
        match config.compile() {
            AnyCompiledGeneration::ThreeD(generation) => generation,
            AnyCompiledGeneration::TwoD(_) => panic!("expected 3D generation"),
        }
    }

    #[test]
    fn drain_cancellable_pushes_active_generation_in_order() {
        let current_generation = AtomicU64::new(7);
        let mut pushed = Vec::new();

        let cancelled = block_on(drain_cancellable(
            0..4,
            |item| pushed.push(item),
            7,
            &current_generation,
        ));

        assert!(!cancelled);
        assert_eq!(pushed, [0, 1, 2, 3]);
    }

    #[test]
    fn drain_cancellable_stops_before_first_item_when_stale() {
        let current_generation = AtomicU64::new(8);
        let mut pushed = Vec::new();

        let cancelled = block_on(drain_cancellable(
            0..4,
            |item| pushed.push(item),
            7,
            &current_generation,
        ));

        assert!(cancelled);
        assert!(pushed.is_empty());
    }

    #[test]
    fn drain_cancellable_observes_revision_at_next_interval() {
        let current_generation = AtomicU64::new(7);
        let mut pushed = Vec::new();

        let cancelled = block_on(drain_cancellable(
            0..=CANCELLATION_CHECK_INTERVAL,
            |item| {
                pushed.push(item);
                if pushed.len() == 1 {
                    current_generation.store(8, Ordering::Release);
                }
            },
            7,
            &current_generation,
        ));

        assert!(cancelled);
        assert_eq!(pushed.len(), CANCELLATION_CHECK_INTERVAL);
        assert_eq!(pushed.last(), Some(&(CANCELLATION_CHECK_INTERVAL - 1)));
    }

    fn cancellable_generation() -> GenerationConfig {
        GenerationConfig::new(
            Dimensions::TwoD,
            "F".to_string(),
            14,
            90.0,
            1.0,
            0.0,
            BTreeMap::from([('F', "FF".to_string())]),
        )
        .expect("balanced config")
    }

    fn assert_generation_drain_is_cancellable(
        iter: impl Iterator<Item = [<D2 as lsystem_core::Dimension>::Point; 2]>,
    ) {
        let current_generation = AtomicU64::new(7);
        let mut pushed = 0;
        let cancelled = block_on(drain_cancellable(
            iter,
            |_| {
                pushed += 1;
                if pushed == 1 {
                    current_generation.store(8, Ordering::Release);
                }
            },
            7,
            &current_generation,
        ));

        assert!(cancelled);
        assert_eq!(pushed, CANCELLATION_CHECK_INTERVAL);
    }

    #[test]
    fn interpreted_generation_drain_is_cancellable() {
        let generation = compile_2d(&cancellable_generation());
        assert_generation_drain_is_cancellable(generation.segments());
    }

    #[test]
    fn stamped_generation_drain_is_cancellable() {
        let prepared = compile_2d(&cancellable_generation())
            .plan_templates(DEFAULT_TEMPLATE_SEGMENT_BUDGET)
            .prepare();
        let PreparedGeneration::Stamped(set) = prepared else {
            panic!("the fixture must select stamped generation")
        };
        assert_generation_drain_is_cancellable(set.segments());
    }

    #[test]
    fn build_typed_scene_maps_plain_and_depth_variants_2d() {
        let colors = color_config();
        let current_generation = AtomicU64::new(1);

        let plain = compile_2d(&generation_config(Dimensions::TwoD, "F+F"));
        let result = block_on(build_typed_scene::<D2>(
            plain,
            &colors,
            Camera::new(),
            1,
            &current_generation,
            Instant::now(),
        ));
        match result {
            SceneBuildResult::Ready { generation, scene } => {
                assert_eq!(generation, 1);
                match &scene.geometry {
                    SceneGeometry::TwoD { segments, .. } => assert_eq!(segments.len(), 2),
                    _ => panic!("expected TwoD geometry for bracketless 2D axiom"),
                }
            }
            SceneBuildResult::Cancelled => panic!("expected Ready for matching revision"),
        }

        let depth = compile_2d(&generation_config(Dimensions::TwoD, "F[+F]F"));
        let result = block_on(build_typed_scene::<D2>(
            depth,
            &colors,
            Camera::new(),
            1,
            &current_generation,
            Instant::now(),
        ));
        match result {
            SceneBuildResult::Ready { generation, scene } => {
                assert_eq!(generation, 1);
                match &scene.geometry {
                    SceneGeometry::TwoDWithTopologicalDepth {
                        segments,
                        max_topological_depth,
                        ..
                    } => {
                        assert_eq!(segments.len(), 3);
                        assert_eq!(*max_topological_depth, 1);
                    }
                    _ => {
                        panic!("expected TwoDWithTopologicalDepth geometry for bracketed 2D axiom")
                    }
                }
            }
            SceneBuildResult::Cancelled => panic!("expected Ready for matching revision"),
        }
    }

    #[test]
    fn build_typed_scene_maps_plain_and_depth_variants_3d() {
        let colors = color_config();
        let current_generation = AtomicU64::new(1);

        let plain = compile_3d(&generation_config(Dimensions::ThreeD, "F+F"));
        let result = block_on(build_typed_scene::<D3>(
            plain,
            &colors,
            Camera::new(),
            1,
            &current_generation,
            Instant::now(),
        ));
        match result {
            SceneBuildResult::Ready { generation, scene } => {
                assert_eq!(generation, 1);
                match &scene.geometry {
                    SceneGeometry::ThreeD { segments, .. } => assert_eq!(segments.len(), 2),
                    _ => panic!("expected ThreeD geometry for bracketless 3D axiom"),
                }
            }
            SceneBuildResult::Cancelled => panic!("expected Ready for matching revision"),
        }

        let depth = compile_3d(&generation_config(Dimensions::ThreeD, "F[+F]F"));
        let result = block_on(build_typed_scene::<D3>(
            depth,
            &colors,
            Camera::new(),
            1,
            &current_generation,
            Instant::now(),
        ));
        match result {
            SceneBuildResult::Ready { generation, scene } => {
                assert_eq!(generation, 1);
                match &scene.geometry {
                    SceneGeometry::ThreeDWithTopologicalDepth {
                        segments,
                        max_topological_depth,
                        ..
                    } => {
                        assert_eq!(segments.len(), 3);
                        assert_eq!(*max_topological_depth, 1);
                    }
                    _ => panic!(
                        "expected ThreeDWithTopologicalDepth geometry for bracketed 3D axiom"
                    ),
                }
            }
            SceneBuildResult::Cancelled => panic!("expected Ready for matching revision"),
        }
    }

    #[test]
    fn build_typed_scene_cancels_for_stale_revision() {
        // Exercises the post-collection cancellation check in
        // `build_typed_scene` (the `is_cancelled` call after segment
        // collection completes, distinct from the in-progress check inside
        // `drain_cancellable` covered above): `current_generation` is bumped
        // past `generation_revision` before the call, so even though
        // collection itself completes normally, the stale revision must
        // still yield `Cancelled`.
        let colors = color_config();
        let current_generation = AtomicU64::new(2);

        let plain = compile_2d(&generation_config(Dimensions::TwoD, "F+F"));
        let result = block_on(build_typed_scene::<D2>(
            plain,
            &colors,
            Camera::new(),
            1,
            &current_generation,
            Instant::now(),
        ));

        assert!(matches!(result, SceneBuildResult::Cancelled));
    }
}
