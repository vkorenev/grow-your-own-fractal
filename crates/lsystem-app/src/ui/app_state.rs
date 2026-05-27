use iced::keyboard;
use iced::widget::row;
use iced::{Element, Event, Length, Point, Size, Subscription, Task, event, window};
use include_dir::{Dir, include_dir};
use lsystem_core::{
    CleanMut, ColorConfig, Config, ConfigError, ConfigWorkspace, Dimensions, EntryViewMut,
    LineColorConfig,
};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

#[cfg(not(target_arch = "wasm32"))]
use crate::export::choose_export_path;
use crate::export::{ExportKind, ExportOutcome, ExportRequest, handle_export};

use super::fractal_canvas::{Scene, SceneBuildResult, build_scene};
use super::{PNG_MAX_WIDTH, PNG_MIN_WIDTH};

static PRESETS_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../presets");

const ROTATION_STEP_DEG: f32 = 5.0;
const AUTO_ROTATE_DT_SECS: f32 = 1.0 / 60.0;
const HSV_MOVEMENT_DEFAULT_SPEED_DEGREES_PER_SECOND: f32 = 15.0;
pub(super) const HSV_MOVEMENT_MIN_SPEED_DEGREES_PER_SECOND: f32 = 1.0;
pub(super) const HSV_MOVEMENT_MAX_SPEED_DEGREES_PER_SECOND: f32 = 60.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LineColorMode {
    Solid,
    Gradient,
    HueCycle,
    DepthGradient,
}

impl LineColorMode {
    pub(super) const ALL: &'static [Self] = &[
        Self::Solid,
        Self::Gradient,
        Self::HueCycle,
        Self::DepthGradient,
    ];

    pub(super) fn from_line_color(line_color: &LineColorConfig) -> Self {
        match line_color {
            LineColorConfig::Solid { .. } => Self::Solid,
            LineColorConfig::Gradient { .. } => Self::Gradient,
            LineColorConfig::HueCycle { .. } => Self::HueCycle,
            LineColorConfig::DepthGradient { .. } => Self::DepthGradient,
        }
    }

    fn default_line_color(self) -> LineColorConfig {
        match self {
            Self::Solid => LineColorConfig::DEFAULT_SOLID,
            Self::Gradient => LineColorConfig::DEFAULT_GRADIENT,
            Self::HueCycle => LineColorConfig::DEFAULT_HUE_CYCLE,
            Self::DepthGradient => LineColorConfig::DEFAULT_DEPTH_GRADIENT,
        }
    }
}

impl std::fmt::Display for LineColorMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Solid => "Solid",
            Self::Gradient => "Gradient",
            Self::HueCycle => "Hue cycle",
            Self::DepthGradient => "Depth gradient",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HsvMovementDirection {
    Forward,
    Reverse,
}

impl HsvMovementDirection {
    pub(super) const ALL: &'static [Self] = &[Self::Forward, Self::Reverse];

    fn sign(self) -> f32 {
        match self {
            Self::Forward => 1.0,
            Self::Reverse => -1.0,
        }
    }
}

impl std::fmt::Display for HsvMovementDirection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Forward => "Forward",
            Self::Reverse => "Reverse",
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct HsvMovement {
    pub(super) enabled: bool,
    pub(super) speed_degrees_per_second: f32,
    pub(super) direction: HsvMovementDirection,
    pub(super) phase_degrees: f32,
}

impl HsvMovement {
    fn stop_and_reset(&mut self) {
        self.enabled = false;
        self.phase_degrees = 0.0;
    }

    fn set_speed(&mut self, speed: f32) {
        self.speed_degrees_per_second = speed.clamp(
            HSV_MOVEMENT_MIN_SPEED_DEGREES_PER_SECOND,
            HSV_MOVEMENT_MAX_SPEED_DEGREES_PER_SECOND,
        );
    }

    fn advance(&mut self, dt_seconds: f32) {
        self.phase_degrees = (self.phase_degrees
            + self.direction.sign() * self.speed_degrees_per_second * dt_seconds)
            .rem_euclid(360.0);
    }
}

impl Default for HsvMovement {
    fn default() -> Self {
        Self {
            enabled: false,
            speed_degrees_per_second: HSV_MOVEMENT_DEFAULT_SPEED_DEGREES_PER_SECOND,
            direction: HsvMovementDirection::Forward,
            phase_degrees: 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct ColorControlMemory {
    background: [f32; 3],
    solid: Option<[f32; 3]>,
    gradient: Option<([f32; 3], [f32; 3])>,
    hue_cycle: Option<[f32; 3]>,
    depth_gradient: Option<([f32; 3], [f32; 3])>,
}

impl ColorControlMemory {
    fn from_colors(colors: &ColorConfig) -> Self {
        let mut memory = Self {
            background: colors.background.unwrap_or(ColorConfig::DEFAULT_BACKGROUND),
            solid: None,
            gradient: None,
            hue_cycle: None,
            depth_gradient: None,
        };
        memory.remember_line(colors.line);
        memory
    }

    fn background(&self) -> [f32; 3] {
        self.background
    }

    fn remember_background(&mut self, background: [f32; 3]) {
        self.background = background;
    }

    fn remember_line(&mut self, line_color: LineColorConfig) {
        match line_color {
            LineColorConfig::Solid { color } => self.solid = Some(color),
            LineColorConfig::Gradient { start, end } => self.gradient = Some((start, end)),
            LineColorConfig::HueCycle { initial } => self.hue_cycle = Some(initial),
            LineColorConfig::DepthGradient { start, end } => {
                self.depth_gradient = Some((start, end));
            }
        }
    }

    fn line_for(&self, mode: LineColorMode) -> LineColorConfig {
        match (mode, self) {
            (
                LineColorMode::Solid,
                Self {
                    solid: Some(color), ..
                },
            ) => LineColorConfig::Solid { color: *color },
            (
                LineColorMode::Gradient,
                Self {
                    gradient: Some((start, end)),
                    ..
                },
            ) => LineColorConfig::Gradient {
                start: *start,
                end: *end,
            },
            (
                LineColorMode::HueCycle,
                Self {
                    hue_cycle: Some(initial),
                    ..
                },
            ) => LineColorConfig::HueCycle { initial: *initial },
            (
                LineColorMode::DepthGradient,
                Self {
                    depth_gradient: Some((start, end)),
                    ..
                },
            ) => LineColorConfig::DepthGradient {
                start: *start,
                end: *end,
            },
            _ => mode.default_line_color(),
        }
    }
}

#[derive(Debug, Clone)]
pub(super) enum Message {
    PresetSelected(String),
    CopyConfig,
    TomlEdited(iced::widget::text_editor::Action),
    ApplyConfig,
    RevertConfig,
    ResetConfig,
    IterationsChanged(u32),
    AngleChanged(f32),
    BackgroundOverrideToggled(bool),
    BackgroundColorChanged([f32; 3]),
    LineColorModeSelected(LineColorMode),
    LineColorChanged(LineColorConfig),
    PngWidthChanged(String),
    ExportSvg,
    ExportPng,
    ExportFinished(ExportOutcome),
    SceneGenerated(SceneBuildResult),
    Fit,
    FractalPan {
        dx: f32,
        dy: f32,
        size: Size,
    },
    FractalOrbit {
        dx: f32,
        dy: f32,
    },
    FractalZoom {
        delta_y: f32,
        cursor: Point,
        size: Size,
    },
    RotateBy {
        d_az: f32,
        d_el: f32,
    },
    RollBy(f32),
    ToggleAutoRotate,
    SetAutoRotateSpeed(f32),
    ToggleHsvMovement,
    SetHsvMovementSpeed(f32),
    SetHsvMovementDirection(HsvMovementDirection),
    AnimationTick,
}

pub(super) struct FractalApp {
    pub(super) config_workspace: ConfigWorkspace,
    pub(super) toml: iced::widget::text_editor::Content,
    pub(super) iterations: u32,
    pub(super) max_iterations: u32,
    pub(super) png_width: u32,
    pub(super) png_width_text: String,
    pub(super) error: Option<String>,
    pub(super) export_status: Option<String>,
    pub(super) scene_pending: bool,
    pub(super) scene: Scene,
    pub(super) auto_rotate: bool,
    pub(super) auto_rotate_speed: f32,
    pub(super) hsv_movement: HsvMovement,
    color_memory: ColorControlMemory,
    scene_generation: Arc<AtomicU64>,
}

impl FractalApp {
    pub(super) fn new() -> (Self, Task<Message>) {
        let config_workspace = ConfigWorkspace::from_presets(load_presets())
            .expect("at least one bundled preset should parse");
        let selected_entry = config_workspace.selected();
        let toml_text = selected_entry.draft_text().into_owned();
        let color_memory = ColorControlMemory::from_colors(&selected_entry.applied_config().colors);

        let mut app = Self {
            config_workspace,
            toml: iced::widget::text_editor::Content::with_text(&toml_text),
            iterations: 1,
            max_iterations: 1,
            png_width: 2048,
            png_width_text: "2048".to_string(),
            error: None,
            export_status: None,
            scene_pending: false,
            scene: Scene::default(),
            auto_rotate: false,
            auto_rotate_speed: 45.0,
            hsv_movement: HsvMovement::default(),
            color_memory,
            scene_generation: Arc::new(AtomicU64::new(0)),
        };
        app.sync_controls_from_workspace();
        let task = app.schedule_scene_generation();
        (app, task)
    }

    pub(super) fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::PresetSelected(name) => {
                if let Some(index) = self.config_workspace.index_by_name(&name) {
                    if let Err(error) = self.config_workspace.select(index) {
                        self.error = Some(error.to_string());
                        return Task::none();
                    }
                    return self.refresh_from_workspace();
                }
                Task::none()
            }
            Message::CopyConfig => match self.config_workspace.copy() {
                Ok(_) => self.refresh_from_workspace(),
                Err(error) => {
                    self.error = Some(error.to_string());
                    Task::none()
                }
            },
            Message::TomlEdited(action) => {
                self.toml.perform(action);
                self.config_workspace
                    .selected_mut()
                    .set_draft_text(self.toml.text());
                self.export_status = None;
                Task::none()
            }
            Message::ApplyConfig => self.apply_config(),
            Message::RevertConfig => match self.config_workspace.selected_mut().view_mut() {
                EntryViewMut::Dirty(dirty) => {
                    dirty.revert();
                    self.refresh_from_workspace()
                }
                EntryViewMut::Clean(_) => {
                    log::error!("revert fired while entry is clean; UI guards bypassed");
                    Task::none()
                }
            },
            Message::ResetConfig => {
                if self.config_workspace.selected().is_dirty() {
                    return Task::none();
                }
                match self.config_workspace.reset() {
                    Ok(Some(_)) => self.refresh_from_workspace(),
                    Ok(None) => Task::none(),
                    Err(error) => {
                        self.error = Some(error.to_string());
                        Task::none()
                    }
                }
            }
            Message::IterationsChanged(iterations) => {
                let iterations = iterations.min(self.max_iterations);
                self.update_clean_config("iterations slider", |clean| {
                    clean.set_iterations(iterations)
                })
            }
            Message::AngleChanged(angle) => {
                self.update_clean_config("angle slider", |clean| clean.set_angle(angle))
            }
            Message::BackgroundOverrideToggled(enabled) => {
                if let Some(background) = self.selected_config().colors.background {
                    self.color_memory.remember_background(background);
                }
                let background = if enabled {
                    Some(self.color_memory.background())
                } else {
                    None
                };
                self.update_clean_color_config("background override toggle", |clean| {
                    clean.set_background(background)
                })
            }
            Message::BackgroundColorChanged(color) => {
                self.color_memory.remember_background(color);
                self.update_clean_color_config("background color slider", |clean| {
                    clean.set_background(Some(color))
                })
            }
            Message::LineColorModeSelected(mode) => {
                let current = self.selected_config().colors.line;
                self.color_memory.remember_line(current);
                let line_color = self.color_memory.line_for(mode);
                self.update_clean_color_config("line color mode select", |clean| {
                    clean.set_line_color(line_color)
                })
            }
            Message::LineColorChanged(line_color) => self
                .update_clean_color_config("line color control", |clean| {
                    clean.set_line_color(line_color)
                }),
            Message::PngWidthChanged(value) => {
                self.png_width_text = value;
                self.export_status = None;
                if let Ok(width) = self.png_width_text.parse::<u32>() {
                    self.png_width = width.clamp(PNG_MIN_WIDTH, PNG_MAX_WIDTH);
                }
                Task::none()
            }
            Message::ExportSvg => {
                if self.effective_is_3d() {
                    return Task::none();
                }
                self.export(ExportKind::Svg)
            }
            Message::ExportPng => self.export(ExportKind::Png),
            Message::ExportFinished(outcome) => {
                self.export_status = Some(match outcome {
                    ExportOutcome::Saved(kind) => format!("{kind} export complete"),
                    ExportOutcome::Cancelled => "Export cancelled".to_string(),
                    ExportOutcome::Failed(error) => format!("Export failed: {error}"),
                });
                Task::none()
            }
            Message::SceneGenerated(result) => {
                if let SceneBuildResult::Ready {
                    generation,
                    mut scene,
                } = result
                    && self.is_current_generation(generation)
                {
                    scene.update_colors(&self.selected_config().effective_colors());
                    self.scene = scene;
                    self.scene_pending = false;
                }
                Task::none()
            }
            Message::Fit => {
                self.scene.reset_camera();
                Task::none()
            }
            Message::FractalPan { dx, dy, size } => {
                self.scene.pan_by_pixels(dx, dy, size);
                Task::none()
            }
            Message::FractalOrbit { dx, dy } => {
                self.scene.orbit_by_pixels(dx, dy);
                Task::none()
            }
            Message::FractalZoom {
                delta_y,
                cursor,
                size,
            } => {
                self.scene.zoom_toward_cursor(delta_y, cursor, size);
                Task::none()
            }
            Message::RotateBy { d_az, d_el } => {
                if self.scene.is_3d() {
                    self.scene.orbit_by(d_az, d_el);
                }
                Task::none()
            }
            Message::RollBy(degrees) => {
                if self.scene.is_3d() {
                    self.scene.roll_by(degrees);
                }
                Task::none()
            }
            Message::ToggleAutoRotate => {
                self.auto_rotate = !self.auto_rotate;
                Task::none()
            }
            Message::SetAutoRotateSpeed(speed) => {
                self.auto_rotate_speed = speed;
                Task::none()
            }
            Message::ToggleHsvMovement => {
                if self.hsv_movement.enabled {
                    self.reset_hsv_movement();
                } else if self.line_color_is_hue_cycle() {
                    self.hsv_movement.enabled = true;
                }
                Task::none()
            }
            Message::SetHsvMovementSpeed(speed) => {
                self.hsv_movement.set_speed(speed);
                Task::none()
            }
            Message::SetHsvMovementDirection(direction) => {
                self.hsv_movement.direction = direction;
                Task::none()
            }
            Message::AnimationTick => {
                if self.auto_rotate && self.scene.is_3d() {
                    self.scene
                        .auto_rotate_by(self.auto_rotate_speed * AUTO_ROTATE_DT_SECS);
                }
                if self.hsv_movement.enabled && self.line_color_is_hue_cycle() {
                    self.hsv_movement.advance(AUTO_ROTATE_DT_SECS);
                    self.scene
                        .set_hue_offset_degrees(self.hsv_movement.phase_degrees);
                }
                Task::none()
            }
        }
    }

    pub(super) fn view(&self) -> Element<'_, Message> {
        row![self.controls(), self.fractal_view()]
            .height(Length::Fill)
            .into()
    }

    pub(super) fn subscription(&self) -> Subscription<Message> {
        let is_3d = self.scene.is_3d();
        let auto_rotate = self.auto_rotate;
        let hsv_movement = self.hsv_movement.enabled && self.line_color_is_hue_cycle();

        let key_sub = event::listen_with(|event, status, _window| {
            if status == event::Status::Captured {
                return None;
            }
            match event {
                Event::Keyboard(keyboard::Event::KeyPressed { key, .. }) => match &key {
                    keyboard::Key::Character(ch) if ch.eq_ignore_ascii_case("f") => {
                        Some(Message::Fit)
                    }
                    keyboard::Key::Named(keyboard::key::Named::ArrowLeft) => {
                        Some(Message::RotateBy {
                            d_az: -ROTATION_STEP_DEG,
                            d_el: 0.0,
                        })
                    }
                    keyboard::Key::Named(keyboard::key::Named::ArrowRight) => {
                        Some(Message::RotateBy {
                            d_az: ROTATION_STEP_DEG,
                            d_el: 0.0,
                        })
                    }
                    keyboard::Key::Named(keyboard::key::Named::ArrowUp) => {
                        Some(Message::RotateBy {
                            d_az: 0.0,
                            d_el: ROTATION_STEP_DEG,
                        })
                    }
                    keyboard::Key::Named(keyboard::key::Named::ArrowDown) => {
                        Some(Message::RotateBy {
                            d_az: 0.0,
                            d_el: -ROTATION_STEP_DEG,
                        })
                    }
                    keyboard::Key::Character(ch) if ch.eq_ignore_ascii_case("q") => {
                        Some(Message::RollBy(-ROTATION_STEP_DEG))
                    }
                    keyboard::Key::Character(ch) if ch.eq_ignore_ascii_case("e") => {
                        Some(Message::RollBy(ROTATION_STEP_DEG))
                    }
                    _ => None,
                },
                _ => None,
            }
        });

        if (is_3d && auto_rotate) || hsv_movement {
            let frames = window::frames().map(|_| Message::AnimationTick);
            Subscription::batch([key_sub, frames])
        } else {
            key_sub
        }
    }

    fn apply_config(&mut self) -> Task<Message> {
        self.config_workspace
            .selected_mut()
            .set_draft_text(self.toml.text());
        if !self.config_workspace.selected().is_dirty() {
            return Task::none();
        }
        match self.config_workspace.apply() {
            Ok(_) => {
                self.reset_color_memory_from_workspace();
                self.sync_controls_from_workspace();
                self.error = None;
                self.export_status = None;
                self.schedule_scene_generation()
            }
            Err(error) => {
                self.error = Some(error.to_string());
                Task::none()
            }
        }
    }

    fn refresh_from_workspace(&mut self) -> Task<Message> {
        self.refresh_from_workspace_impl(true)
    }

    fn refresh_from_workspace_preserving_color_memory(&mut self) -> Task<Message> {
        self.refresh_from_workspace_impl(false)
    }

    fn refresh_from_workspace_impl(&mut self, reset_color_memory: bool) -> Task<Message> {
        let entry = self.config_workspace.selected();
        let toml_text = entry.draft_text();
        self.toml = iced::widget::text_editor::Content::with_text(&toml_text);
        if reset_color_memory {
            self.reset_color_memory_from_workspace();
        }
        self.sync_controls_from_workspace();
        self.error = None;
        self.export_status = None;
        self.schedule_scene_generation()
    }

    /// Rebuilds the scene from scratch (config-affecting changes such as
    /// iterations or angle).
    fn update_clean_config(
        &mut self,
        event: &'static str,
        update: impl FnOnce(&mut CleanMut<'_>) -> Result<(), ConfigError>,
    ) -> Task<Message> {
        self.update_clean_entry(
            event,
            update,
            Self::refresh_from_workspace_preserving_color_memory,
        )
    }

    /// Recolors the existing scene in place (color-only changes that do not
    /// alter geometry).
    fn update_clean_color_config(
        &mut self,
        event: &'static str,
        update: impl FnOnce(&mut CleanMut<'_>) -> Result<(), ConfigError>,
    ) -> Task<Message> {
        self.update_clean_entry(event, update, Self::refresh_after_clean_color_update)
    }

    /// Applies `update` to the selected entry while it is clean, then runs
    /// `on_success`. Logs and no-ops if UI guards let a dirty entry through.
    fn update_clean_entry(
        &mut self,
        event: &'static str,
        update: impl FnOnce(&mut CleanMut<'_>) -> Result<(), ConfigError>,
        on_success: impl FnOnce(&mut Self) -> Task<Message>,
    ) -> Task<Message> {
        match self.config_workspace.selected_mut().view_mut() {
            EntryViewMut::Clean(mut clean) => match update(&mut clean) {
                Ok(()) => on_success(self),
                Err(error) => {
                    self.error = Some(error.to_string());
                    Task::none()
                }
            },
            EntryViewMut::Dirty(_) => {
                log::error!("{event} fired while entry is dirty; UI guards bypassed");
                Task::none()
            }
        }
    }

    fn refresh_after_clean_color_update(&mut self) -> Task<Message> {
        self.toml = iced::widget::text_editor::Content::with_text(
            self.config_workspace.selected().draft_text().as_ref(),
        );
        let current_iterations = self.iterations;
        self.recompute_max_iterations();
        self.iterations = current_iterations.min(self.max_iterations);

        let colors = self
            .config_workspace
            .selected()
            .applied_config()
            .effective_colors();
        self.scene.update_colors(&colors);
        self.error = None;
        self.export_status = None;
        Task::none()
    }

    fn reset_color_memory_from_workspace(&mut self) {
        self.color_memory = ColorControlMemory::from_colors(&self.selected_config().colors);
    }

    fn reset_hsv_movement(&mut self) {
        self.hsv_movement.stop_and_reset();
        self.scene.set_hue_offset_degrees(0.0);
    }

    fn line_color_is_hue_cycle(&self) -> bool {
        matches!(
            self.selected_config().colors.line,
            LineColorConfig::HueCycle { .. }
        )
    }

    fn sync_controls_from_workspace(&mut self) {
        let iterations = self
            .config_workspace
            .selected()
            .applied_config()
            .generation
            .iterations;
        self.recompute_max_iterations();
        self.iterations = iterations.min(self.max_iterations);
    }

    fn recompute_max_iterations(&mut self) {
        let config = self.config_workspace.selected().applied_config();
        let generation = &config.generation;
        let max_seg = lsystem_renderer::line_renderer::max_segments_for_line_color(
            generation.dimensions,
            generation.has_stack_directives(),
        );
        self.max_iterations =
            lsystem_core::max_safe_iterations(&generation.axiom, &generation.rules, max_seg) as u32;
    }

    pub(super) fn selected_config(&self) -> &Config {
        self.config_workspace.selected().applied_config()
    }

    fn effective_config(&self) -> Config {
        let mut config = self.selected_config().clone();
        config.generation.iterations = self.iterations;
        config
    }

    pub(super) fn effective_is_3d(&self) -> bool {
        matches!(
            self.selected_config().generation.dimensions,
            Dimensions::ThreeD
        )
    }

    fn schedule_scene_generation(&mut self) -> Task<Message> {
        let config = self.effective_config();

        let generation = self
            .scene_generation
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1);
        let token = Arc::clone(&self.scene_generation);
        self.scene_pending = true;
        self.export_status = None;

        let prev_camera = self.scene.camera.clone();
        Task::perform(
            build_scene(config, generation, token, prev_camera),
            Message::SceneGenerated,
        )
    }

    fn is_current_generation(&self, generation: u64) -> bool {
        self.scene_generation.load(Ordering::Acquire) == generation
    }

    fn export(&mut self, kind: ExportKind) -> Task<Message> {
        let config = self.effective_config();
        let png_width = if matches!(kind, ExportKind::Png) {
            match self.normalized_png_width() {
                Ok(width) => width,
                Err(error) => {
                    return Task::done(Message::ExportFinished(ExportOutcome::Failed(error)));
                }
            }
        } else {
            self.png_width
        };

        #[cfg(not(target_arch = "wasm32"))]
        let request = {
            let Some(path) = choose_export_path(&config.name, kind) else {
                return Task::done(Message::ExportFinished(ExportOutcome::Cancelled));
            };

            match kind {
                ExportKind::Svg => ExportRequest::Svg { config, path },
                ExportKind::Png => ExportRequest::Png {
                    config,
                    width: png_width,
                    path,
                    camera: self.scene.camera.clone(),
                },
            }
        };

        #[cfg(target_arch = "wasm32")]
        let request = match kind {
            ExportKind::Svg => ExportRequest::Svg(config),
            ExportKind::Png => ExportRequest::Png {
                config,
                width: png_width,
                camera: self.scene.camera.clone(),
            },
        };

        Task::perform(handle_export(request), Message::ExportFinished)
    }

    fn normalized_png_width(&mut self) -> Result<u32, String> {
        let Ok(width) = self.png_width_text.trim().parse::<u32>() else {
            return Err(format!(
                "PNG width must be a number from {PNG_MIN_WIDTH} to {PNG_MAX_WIDTH}"
            ));
        };

        let width = width.clamp(PNG_MIN_WIDTH, PNG_MAX_WIDTH);
        self.png_width = width;
        self.png_width_text = width.to_string();
        Ok(width)
    }
}

fn load_presets() -> Vec<(String, String)> {
    let mut files: Vec<_> = PRESETS_DIR
        .files()
        .filter(|file| file.path().extension().and_then(|ext| ext.to_str()) == Some("toml"))
        .collect();
    files.sort_by_key(|file| file.path());
    files
        .into_iter()
        .filter_map(|file| {
            let label = file.path().display().to_string();
            Some((label, file.contents_utf8()?.to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_control_memory_uses_defaults_and_restores_edits() {
        let solid = LineColorConfig::Solid {
            color: [0.1, 0.2, 0.3],
        };
        let gradient = LineColorConfig::Gradient {
            start: [0.4, 0.5, 0.6],
            end: [0.7, 0.8, 0.9],
        };
        let depth_gradient = LineColorConfig::DepthGradient {
            start: [0.2, 0.3, 0.4],
            end: [0.5, 0.6, 0.7],
        };

        let mut memory = ColorControlMemory::from_colors(&lsystem_core::ColorConfig {
            background: Some([0.8, 0.8, 0.8]),
            line: solid,
        });

        assert_eq!(memory.background(), [0.8, 0.8, 0.8]);
        assert_eq!(memory.line_for(LineColorMode::Solid), solid);
        assert_eq!(
            memory.line_for(LineColorMode::Gradient),
            LineColorConfig::DEFAULT_GRADIENT
        );
        assert_eq!(
            memory.line_for(LineColorMode::HueCycle),
            LineColorConfig::DEFAULT_HUE_CYCLE
        );
        assert_eq!(
            memory.line_for(LineColorMode::DepthGradient),
            LineColorConfig::DEFAULT_DEPTH_GRADIENT
        );

        memory.remember_background([0.9, 0.1, 0.2]);
        memory.remember_line(gradient);
        memory.remember_line(depth_gradient);

        assert_eq!(memory.background(), [0.9, 0.1, 0.2]);
        assert_eq!(memory.line_for(LineColorMode::Solid), solid);
        assert_eq!(memory.line_for(LineColorMode::Gradient), gradient);
        assert_eq!(
            memory.line_for(LineColorMode::DepthGradient),
            depth_gradient
        );
    }

    #[test]
    fn clean_apply_preserves_inactive_line_color_memory() {
        let (mut app, _) = FractalApp::new();
        let gradient = LineColorConfig::Gradient {
            start: [0.4, 0.5, 0.6],
            end: [0.7, 0.8, 0.9],
        };
        app.color_memory.remember_line(gradient);

        let _ = app.update(Message::ApplyConfig);

        assert_eq!(app.color_memory.line_for(LineColorMode::Gradient), gradient);
    }

    #[test]
    fn color_mode_change_does_not_schedule_or_cancel_geometry() {
        let (mut app, _) = FractalApp::new();
        let generation_before = app.scene_generation.load(Ordering::Acquire);
        app.scene_pending = false;

        let _ = app.update(Message::LineColorModeSelected(LineColorMode::DepthGradient));
        assert!(
            !app.scene_pending,
            "color change must not schedule geometry"
        );
        assert_eq!(
            app.scene_generation.load(Ordering::Acquire),
            generation_before
        );

        // Simulate a geometry build in-flight for an unrelated reason
        app.scene_pending = true;
        let generation_mid = app.scene_generation.load(Ordering::Acquire);

        let _ = app.update(Message::LineColorModeSelected(LineColorMode::Solid));
        assert!(
            app.scene_pending,
            "color change must not cancel in-flight geometry build"
        );
        assert_eq!(app.scene_generation.load(Ordering::Acquire), generation_mid);
    }

    #[test]
    fn background_toggle_restores_last_selected_color() {
        let (mut app, _) = FractalApp::new();
        let background = [0.2, 0.3, 0.4];

        let _ = app.update(Message::BackgroundColorChanged(background));
        let _ = app.update(Message::BackgroundOverrideToggled(false));

        assert_eq!(
            app.config_workspace
                .selected()
                .applied_config()
                .colors
                .background,
            None
        );

        let _ = app.update(Message::BackgroundOverrideToggled(true));

        assert_eq!(
            app.config_workspace
                .selected()
                .applied_config()
                .colors
                .background,
            Some(background)
        );
    }

    #[test]
    fn angle_change_updates_effective_config_from_workspace() {
        let (mut app, _) = FractalApp::new();
        let _ = app.update(Message::AngleChanged(45.0));

        assert_eq!(app.effective_config().generation.angle, 45.0);
        assert_eq!(app.selected_config().generation.angle, 45.0);
    }

    #[test]
    fn angle_changed_while_dirty_is_ignored() {
        let (mut app, _) = FractalApp::new();
        let original_angle = app.selected_config().generation.angle;
        let modified = format!("{} ", app.config_workspace.selected().draft_text());
        app.config_workspace.selected_mut().set_draft_text(modified);

        let _ = app.update(Message::AngleChanged(original_angle + 10.0));

        assert_eq!(app.selected_config().generation.angle, original_angle);
    }

    #[test]
    fn hsv_movement_toggle_off_resets_phase() {
        let (mut app, _) = FractalApp::new();

        let _ = app.update(Message::LineColorModeSelected(LineColorMode::HueCycle));
        let _ = app.update(Message::ToggleHsvMovement);
        let _ = app.update(Message::AnimationTick);

        assert!(app.hsv_movement.enabled);
        assert_ne!(app.hsv_movement.phase_degrees, 0.0);

        let _ = app.update(Message::ToggleHsvMovement);

        assert!(!app.hsv_movement.enabled);
        assert_eq!(app.hsv_movement.phase_degrees, 0.0);
    }

    #[test]
    fn hsv_movement_speed_is_clamped() {
        let (mut app, _) = FractalApp::new();

        let _ = app.update(Message::SetHsvMovementSpeed(0.0));
        assert_eq!(app.hsv_movement.speed_degrees_per_second, 1.0);

        let _ = app.update(Message::SetHsvMovementSpeed(75.0));
        assert_eq!(app.hsv_movement.speed_degrees_per_second, 60.0);
    }

    #[test]
    fn hsv_movement_direction_changes_phase_advance_sign() {
        let (mut app, _) = FractalApp::new();

        let _ = app.update(Message::LineColorModeSelected(LineColorMode::HueCycle));
        let _ = app.update(Message::ToggleHsvMovement);
        let _ = app.update(Message::SetHsvMovementSpeed(60.0));
        let _ = app.update(Message::SetHsvMovementDirection(
            HsvMovementDirection::Reverse,
        ));
        let _ = app.update(Message::AnimationTick);

        assert_eq!(app.hsv_movement.phase_degrees, 359.0);
    }

    #[test]
    fn leaving_hue_cycle_preserves_hsv_movement_state_and_ignores_ticks() {
        let (mut app, _) = FractalApp::new();

        let _ = app.update(Message::LineColorModeSelected(LineColorMode::HueCycle));
        let _ = app.update(Message::ToggleHsvMovement);
        let _ = app.update(Message::AnimationTick);
        let phase_before_mode_change = app.hsv_movement.phase_degrees;

        let _ = app.update(Message::LineColorModeSelected(LineColorMode::Solid));
        let _ = app.update(Message::AnimationTick);

        assert!(app.hsv_movement.enabled);
        assert_eq!(app.hsv_movement.phase_degrees, phase_before_mode_change);
    }

    #[test]
    fn effective_is_3d_uses_config_not_stale_scene() {
        let (mut app, _) = FractalApp::new();
        let three_d_config = r#"[metadata]
name = "3D"

[l-system]
dimensions = 3
axiom = "F"
iterations = 1

[l-system.rules]
F = "F"

[turtle]
angle = 60.0
step = 1.0
initial_heading = 0.0

[colors.line]
mode = "solid"
color = [0.0, 0.9, 0.5]
"#;
        app.config_workspace =
            ConfigWorkspace::from_presets(vec![("3d", three_d_config.to_string())]).unwrap();

        assert!(!app.scene.is_3d());
        assert!(app.effective_is_3d());
    }
}
