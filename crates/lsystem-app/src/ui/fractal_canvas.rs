use iced::mouse;
use iced::widget::{container, shader};
use iced::{Background, Color, Element, Event, Length, Point, Rectangle, Size, Theme};
use lsystem_core::{ColorConfig, Config, Dimensions};
use lsystem_renderer::camera::Camera;
use lsystem_renderer::line_renderer::{
    ColorParams, LinePipeline2D, LinePipeline3D, Vertex2D, Vertex3D,
};
use lsystem_renderer::lsystem_bridge::{
    VertexData, VertexData3D, VertexDataBuilder, VertexDataBuilder3D, color_params_from_config,
};
use std::fmt;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use super::app_state::{FractalApp, Message};

const CANCELLATION_CHECK_INTERVAL: usize = 4096;

#[derive(Clone)]
enum SceneGeometry {
    TwoD {
        vertices: Arc<Vec<Vertex2D>>,
        bounds_min: [f32; 2],
        bounds_max: [f32; 2],
    },
    ThreeD {
        vertices: Arc<Vec<Vertex3D>>,
        bounds_min: [f32; 3],
        bounds_max: [f32; 3],
    },
}

#[derive(Clone)]
pub(super) struct Scene {
    geometry: SceneGeometry,
    color_params: ColorParams,
    background: [f32; 3],
    pub(super) camera: Camera,
    revision: u64,
}

impl Scene {
    fn from_vertex_data_2d(
        colors: &ColorConfig,
        data: VertexData,
        camera: Camera,
        revision: u64,
    ) -> Self {
        let total_segments = (data.vertices.len() / 2) as u32;
        Self {
            geometry: SceneGeometry::TwoD {
                vertices: Arc::new(data.vertices),
                bounds_min: data.bounds_min,
                bounds_max: data.bounds_max,
            },
            color_params: color_params_from_config(&colors.line, total_segments),
            background: colors.background,
            camera,
            revision,
        }
    }

    fn from_vertex_data_3d(
        colors: &ColorConfig,
        data: VertexData3D,
        camera: Camera,
        revision: u64,
    ) -> Self {
        let total_segments = (data.vertices.len() / 2) as u32;
        Self {
            geometry: SceneGeometry::ThreeD {
                vertices: Arc::new(data.vertices),
                bounds_min: data.bounds_min,
                bounds_max: data.bounds_max,
            },
            color_params: color_params_from_config(&colors.line, total_segments),
            background: colors.background,
            camera,
            revision,
        }
    }

    pub(super) fn is_3d(&self) -> bool {
        matches!(self.geometry, SceneGeometry::ThreeD { .. })
    }

    pub(super) fn reset_camera(&mut self) {
        self.camera.reset();
    }

    pub(super) fn pan_by_pixels(&mut self, dx: f32, dy: f32, size: Size) {
        if let SceneGeometry::TwoD {
            bounds_min,
            bounds_max,
            ..
        } = self.geometry
        {
            self.camera.pan_by_pixels(
                dx,
                dy,
                bounds_min,
                bounds_max,
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
        match self.geometry {
            SceneGeometry::TwoD {
                bounds_min,
                bounds_max,
                ..
            } => {
                self.camera.zoom_toward_cursor(
                    factor,
                    [cursor.x, cursor.y],
                    bounds_min,
                    bounds_max,
                    size.width.max(1.0) as u32,
                    size.height.max(1.0) as u32,
                );
            }
            SceneGeometry::ThreeD { .. } => {
                self.camera.zoom_3d(factor);
            }
        }
    }

    fn snapshot(&self) -> SceneSnapshot {
        SceneSnapshot {
            geometry: self.geometry.clone(),
            color_params: self.color_params,
            camera: self.camera.clone(),
            revision: self.revision,
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
                let vertex_count = match &scene.geometry {
                    SceneGeometry::TwoD { vertices, .. } => vertices.len(),
                    SceneGeometry::ThreeD { vertices, .. } => vertices.len(),
                };
                f.debug_struct("Ready")
                    .field("generation", generation)
                    .field("vertices", &vertex_count)
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

    match config.generation.dimensions {
        Dimensions::ThreeD => {
            let mut builder = VertexDataBuilder3D::new();
            let mut segments_seen = 0usize;

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

            SceneBuildResult::Ready {
                generation,
                scene: Scene::from_vertex_data_3d(
                    &config.colors,
                    builder.finish(),
                    camera,
                    generation,
                ),
            }
        }
        Dimensions::TwoD => {
            let mut builder = VertexDataBuilder::new();
            let mut segments_seen = 0usize;

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

            SceneBuildResult::Ready {
                generation,
                scene: Scene::from_vertex_data_2d(
                    &config.colors,
                    builder.finish(),
                    camera,
                    generation,
                ),
            }
        }
    }
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
    use wasm_bindgen::{JsCast, JsValue, closure::Closure};

    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        let Some(window) = web_sys::window() else {
            let _ = resolve.call0(&JsValue::UNDEFINED);
            return;
        };

        let fallback_resolve = resolve.clone();
        let callback = Closure::once_into_js(move || {
            let _ = resolve.call0(&JsValue::UNDEFINED);
        });

        if window
            .set_timeout_with_callback_and_timeout_and_arguments_0(callback.unchecked_ref(), 0)
            .is_err()
        {
            let _ = fallback_resolve.call0(&JsValue::UNDEFINED);
        }
    });

    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}

impl Default for Scene {
    fn default() -> Self {
        Self {
            geometry: SceneGeometry::TwoD {
                vertices: Arc::new(Vec::new()),
                bounds_min: [-1.0, -1.0],
                bounds_max: [1.0, 1.0],
            },
            color_params: ColorParams::default(),
            background: [0.0, 0.0, 0.0],
            camera: Camera::new(),
            revision: 0,
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
    revision: u64,
}

impl SceneSnapshot {
    fn is_3d(&self) -> bool {
        matches!(self.geometry, SceneGeometry::ThreeD { .. })
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
            .field("revision", &self.scene.revision)
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

        match &self.scene.geometry {
            SceneGeometry::TwoD {
                vertices,
                bounds_min,
                bounds_max,
            } => {
                let transform =
                    self.scene
                        .camera
                        .compute_transform(*bounds_min, *bounds_max, width, height);
                if pipeline.uploaded_revision != Some(self.scene.revision) {
                    pipeline
                        .pipeline_2d
                        .upload(device, queue, vertices, self.scene.color_params);
                    pipeline.uploaded_revision = Some(self.scene.revision);
                }
                pipeline.pipeline_2d.write_transform(queue, transform);
            }
            SceneGeometry::ThreeD {
                vertices,
                bounds_min,
                bounds_max,
            } => {
                let mvp = self
                    .scene
                    .camera
                    .compute_mvp_3d(*bounds_min, *bounds_max, width, height);
                if pipeline.uploaded_revision != Some(self.scene.revision) {
                    pipeline
                        .pipeline_3d
                        .upload(device, queue, vertices, self.scene.color_params);
                    pipeline.uploaded_revision = Some(self.scene.revision);
                }
                pipeline.pipeline_3d.write_mvp(queue, mvp);
            }
        }
    }

    fn draw(&self, pipeline: &Self::Pipeline, render_pass: &mut wgpu::RenderPass<'_>) -> bool {
        match &self.scene.geometry {
            SceneGeometry::TwoD { .. } => pipeline.pipeline_2d.draw(render_pass),
            SceneGeometry::ThreeD { .. } => pipeline.pipeline_3d.draw(render_pass),
        }
        true
    }
}

struct FractalPipeline {
    pipeline_2d: LinePipeline2D,
    pipeline_3d: LinePipeline3D,
    uploaded_revision: Option<u64>,
}

impl shader::Pipeline for FractalPipeline {
    fn new(device: &wgpu::Device, _queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        Self {
            pipeline_2d: LinePipeline2D::new(device, format),
            pipeline_3d: LinePipeline3D::new(device, format),
            uploaded_revision: None,
        }
    }
}
