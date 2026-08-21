use glam::Vec2;
use iced::keyboard;
use iced::widget::row;
use iced::{Element, Event, Length, Point, Size, Subscription, Task, event, window};
use lsystem_app_model::{
    CAMERA_AUTO_ROTATION_DEFAULT_SPEED_DEGREES_PER_SECOND,
    CAMERA_AUTO_ROTATION_MAX_SPEED_DEGREES_PER_SECOND,
    CAMERA_AUTO_ROTATION_MIN_SPEED_DEGREES_PER_SECOND, CleanMut, ColorControlMemory,
    ConfigDefaults, ConfigEntryId, ConfigWorkspace, EditorConfig, EditorLineColorConfig,
    EntryViewMut, HueRotation, HueRotationDirection, LineColorMode, ParseConfigError,
    advance_hue_rotation_phase_degrees, line_color_for_controls, load_presets,
};
use lsystem_core::{ColorConfig, Config, Dimensions, LineColorConfig, Rgb};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

#[cfg(not(target_arch = "wasm32"))]
use crate::export::choose_export_path;
use crate::export::{ExportKind, ExportOutcome, ExportRequest, handle_export};

use super::fractal_canvas::{Scene, SceneBuildResult, build_scene};
use super::{PNG_MAX_DIMENSION, PNG_MIN_DIMENSION};

const ROTATION_STEP_DEG: f32 = 5.0;
const AUTO_ROTATE_DT_SECS: f32 = 1.0 / 60.0;

fn camera_auto_rotation_active(auto_rotate: bool, is_3d: bool, orbit_drag_active: bool) -> bool {
    auto_rotate && is_3d && !orbit_drag_active
}

pub(super) fn normalized_rename_name(name: &str) -> Option<&str> {
    let name = name.trim();
    (!name.is_empty()).then_some(name)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ColorDefaultField {
    SolidLine,
    GradientStart,
    GradientEnd,
    HueCycleInitial,
}

#[derive(Debug, Clone)]
pub(super) enum Message {
    PresetSelected(ConfigEntryId),
    CopyConfig,
    BeginRename,
    RenameDraftChanged(String),
    CommitRename,
    CancelRename,
    TomlEdited(iced::widget::text_editor::Action),
    ApplyConfig,
    RevertConfig,
    ResetConfig,
    IterationsChanged(u16),
    AngleChanged(f32),
    BackgroundDefaultToggled(bool),
    BackgroundColorChanged(Rgb),
    LineColorModeSelected(LineColorMode),
    LineColorChanged(Option<EditorLineColorConfig>),
    LineColorDefaultToggled {
        field: ColorDefaultField,
        use_default: bool,
    },
    PngWidthChanged(String),
    PngHeightChanged(String),
    ExportSvg,
    ExportPng,
    ExportFinished(ExportOutcome),
    SceneGenerated(SceneBuildResult),
    Fit,
    FractalPan {
        delta: Vec2,
        size: Size,
    },
    FractalOrbit {
        delta: Vec2,
    },
    FractalOrbitStarted,
    FractalOrbitEnded,
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
    ToggleHueRotation,
    SetHueRotationSpeed(f32),
    SetHueRotationDirection(HueRotationDirection),
    AnimationTick,
}

pub(super) struct FractalApp {
    pub(super) config_workspace: ConfigWorkspace,
    pub(super) toml: iced::widget::text_editor::Content,
    pub(super) rename_draft: Option<String>,
    pub(super) iterations: u16,
    pub(super) max_iterations: u16,
    pub(super) png_width: u32,
    pub(super) png_width_text: String,
    pub(super) png_height: u32,
    pub(super) png_height_text: String,
    pub(super) error: Option<String>,
    pub(super) export_status: Option<String>,
    pub(super) scene_pending: bool,
    pub(super) scene: Scene,
    pub(super) auto_rotate: bool,
    pub(super) auto_rotate_speed: f32,
    orbit_drag_active: bool,
    pub(super) hue_rotation: HueRotation,
    pub(super) hue_rotation_phase_degrees: f32,
    color_memory: ColorControlMemory,
    scene_generation: Arc<AtomicU64>,
}

impl FractalApp {
    pub(super) fn new() -> (Self, Task<Message>) {
        let config_workspace = ConfigWorkspace::from_presets(load_presets())
            .expect("at least one bundled preset should parse");
        let selected_entry = config_workspace.selected();
        let toml_text = selected_entry.draft_text().into_owned();
        let color_memory = ColorControlMemory::from_editor_config(
            &selected_entry.editor_config().colors,
            &ConfigDefaults::embedded().colors,
        );

        let mut app = Self {
            config_workspace,
            toml: iced::widget::text_editor::Content::with_text(&toml_text),
            rename_draft: None,
            iterations: 1,
            max_iterations: 1,
            png_width: 800,
            png_width_text: "800".to_string(),
            png_height: 800,
            png_height_text: "800".to_string(),
            error: None,
            export_status: None,
            scene_pending: false,
            scene: Scene::default(),
            auto_rotate: false,
            auto_rotate_speed: CAMERA_AUTO_ROTATION_DEFAULT_SPEED_DEGREES_PER_SECOND,
            orbit_drag_active: false,
            hue_rotation: HueRotation::default(),
            hue_rotation_phase_degrees: 0.0,
            color_memory,
            scene_generation: Arc::new(AtomicU64::new(0)),
        };
        app.sync_controls_from_workspace();
        let task = app.schedule_scene_generation();
        (app, task)
    }

    pub(super) fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::PresetSelected(id) => {
                if let Err(error) = self.config_workspace.select_by_id(id) {
                    self.error = Some(error.to_string());
                    return Task::none();
                }
                self.refresh_from_workspace()
            }
            Message::CopyConfig => match self.config_workspace.copy() {
                Ok(_) => self.refresh_from_workspace(),
                Err(error) => {
                    self.error = Some(error.to_string());
                    Task::none()
                }
            },
            Message::BeginRename => {
                self.rename_draft = Some(
                    self.config_workspace
                        .selected()
                        .name_for_rename()
                        .into_owned(),
                );
                Task::none()
            }
            Message::RenameDraftChanged(name) => {
                if let Some(rename_draft) = &mut self.rename_draft {
                    *rename_draft = name;
                }
                Task::none()
            }
            Message::CommitRename => {
                let Some(name) = self
                    .rename_draft
                    .as_deref()
                    .and_then(normalized_rename_name)
                else {
                    return Task::none();
                };
                match self.config_workspace.selected_mut().rename(name) {
                    Ok(()) => {
                        self.refresh_toml_from_workspace();
                    }
                    Err(error) => self.error = Some(error.to_string()),
                }
                Task::none()
            }
            Message::CancelRename => {
                self.rename_draft = None;
                Task::none()
            }
            Message::TomlEdited(action) => {
                if action.is_edit() {
                    self.rename_draft = None;
                }
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
                if self.config_workspace.selected_mut().reset_to_default() {
                    self.refresh_from_workspace()
                } else {
                    Task::none()
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
            Message::BackgroundDefaultToggled(is_default) => {
                if is_default {
                    self.color_memory
                        .remember_background(self.render_colors().background);
                }
                let background = if is_default {
                    None
                } else {
                    Some(self.color_memory.background())
                };
                self.update_clean_color_config("background", |clean| {
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
                let editor_line = self.config_workspace.selected().editor_config().colors.line;
                self.color_memory.remember_line(editor_line);
                let new_editor_line = self.color_memory.line_for(mode);
                self.update_clean_color_config("line color mode", |clean| {
                    clean.set_line_color(new_editor_line)
                })
            }
            Message::LineColorChanged(line_color) => self
                .update_clean_color_config("line color", |clean| clean.set_line_color(line_color)),
            Message::LineColorDefaultToggled { field, use_default } => {
                use ColorDefaultField::*;
                let editor_line = self.config_workspace.selected().editor_config().colors.line;
                if use_default {
                    self.color_memory.remember_line(editor_line);
                }
                let (editor_start, editor_end, editor_td) =
                    editor_line.map(|l| l.gradient_fields()).unwrap_or_default();
                let (mem_start, mem_end, _) = self.color_memory.gradient_fields();
                let new_line = match (field, use_default) {
                    (SolidLine, true) => None,
                    (SolidLine, false) => Some(EditorLineColorConfig::Solid(
                        self.color_memory.solid_color(),
                    )),
                    (GradientStart, true) => Some(EditorLineColorConfig::Gradient {
                        start: None,
                        end: editor_end,
                        topological_depth: editor_td,
                    }),
                    (GradientStart, false) => Some(EditorLineColorConfig::Gradient {
                        start: Some(mem_start),
                        end: editor_end,
                        topological_depth: editor_td,
                    }),
                    (GradientEnd, true) => Some(EditorLineColorConfig::Gradient {
                        start: editor_start,
                        end: None,
                        topological_depth: editor_td,
                    }),
                    (GradientEnd, false) => Some(EditorLineColorConfig::Gradient {
                        start: editor_start,
                        end: Some(mem_end),
                        topological_depth: editor_td,
                    }),
                    (HueCycleInitial, true) => {
                        Some(EditorLineColorConfig::HueCycle { initial: None })
                    }
                    (HueCycleInitial, false) => Some(EditorLineColorConfig::HueCycle {
                        initial: Some(self.color_memory.hue_cycle_initial()),
                    }),
                };
                self.update_clean_color_config("line color default", |clean| {
                    clean.set_line_color(new_line)
                })
            }
            Message::PngWidthChanged(value) => {
                self.png_width_text = value;
                self.export_status = None;
                if let Ok(width) = self.png_width_text.parse::<u32>() {
                    self.png_width = width.clamp(PNG_MIN_DIMENSION, PNG_MAX_DIMENSION);
                }
                Task::none()
            }
            Message::PngHeightChanged(value) => {
                self.png_height_text = value;
                self.export_status = None;
                if let Ok(height) = self.png_height_text.parse::<u32>() {
                    self.png_height = height.clamp(PNG_MIN_DIMENSION, PNG_MAX_DIMENSION);
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
                    scene.update_colors(&self.render_colors());
                    self.scene = scene;
                    self.scene_pending = false;
                }
                Task::none()
            }
            Message::Fit => {
                self.scene.reset_camera();
                Task::none()
            }
            Message::FractalPan { delta, size } => {
                self.scene.pan_by_pixels(delta, size);
                Task::none()
            }
            Message::FractalOrbit { delta } => {
                self.scene.orbit_by_pixels(delta);
                Task::none()
            }
            Message::FractalOrbitStarted => {
                self.orbit_drag_active = true;
                Task::none()
            }
            Message::FractalOrbitEnded => {
                self.orbit_drag_active = false;
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
                self.auto_rotate_speed = speed.clamp(
                    CAMERA_AUTO_ROTATION_MIN_SPEED_DEGREES_PER_SECOND,
                    CAMERA_AUTO_ROTATION_MAX_SPEED_DEGREES_PER_SECOND,
                );
                Task::none()
            }
            Message::ToggleHueRotation => {
                if self.hue_rotation.is_enabled() {
                    self.reset_hue_rotation();
                } else if matches!(self.control_line_color(), LineColorConfig::HueCycle { .. }) {
                    self.hue_rotation.start();
                }
                Task::none()
            }
            Message::SetHueRotationSpeed(speed) => {
                self.hue_rotation.set_speed(speed);
                Task::none()
            }
            Message::SetHueRotationDirection(direction) => {
                self.hue_rotation.set_direction(direction);
                Task::none()
            }
            Message::AnimationTick => {
                if camera_auto_rotation_active(
                    self.auto_rotate,
                    self.scene.is_3d(),
                    self.orbit_drag_active,
                ) {
                    self.scene
                        .auto_rotate_by(self.auto_rotate_speed * AUTO_ROTATE_DT_SECS);
                }
                if self.hue_rotation.is_active(&self.control_line_color()) {
                    self.hue_rotation_phase_degrees = advance_hue_rotation_phase_degrees(
                        self.hue_rotation_phase_degrees,
                        self.hue_rotation.speed_degrees_per_second(),
                        AUTO_ROTATE_DT_SECS,
                        self.hue_rotation.direction(),
                    );
                    self.scene
                        .set_hue_offset_degrees(self.hue_rotation_phase_degrees);
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
        let auto_rotate =
            camera_auto_rotation_active(self.auto_rotate, is_3d, self.orbit_drag_active);
        let hue_rotation = self.hue_rotation.is_active(&self.control_line_color());

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

        if auto_rotate || hue_rotation {
            let frames = window::frames().map(|_| Message::AnimationTick);
            Subscription::batch([key_sub, frames])
        } else {
            key_sub
        }
    }

    fn apply_config(&mut self) -> Task<Message> {
        self.rename_draft = None;
        self.config_workspace
            .selected_mut()
            .set_draft_text(self.toml.text());
        if !self.config_workspace.selected().is_dirty() {
            return Task::none();
        }
        match self.config_workspace.selected_mut().apply_draft() {
            Ok(()) => {
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
        self.refresh_toml_from_workspace();
        if reset_color_memory {
            self.reset_color_memory_from_workspace();
        }
        self.sync_controls_from_workspace();
        self.schedule_scene_generation()
    }

    fn refresh_toml_from_workspace(&mut self) {
        self.toml = iced::widget::text_editor::Content::with_text(
            self.config_workspace.selected().draft_text().as_ref(),
        );
        self.rename_draft = None;
        self.error = None;
        self.export_status = None;
    }

    /// Rebuilds the scene from scratch (config-affecting changes such as
    /// iterations or angle).
    fn update_clean_config(
        &mut self,
        event: &'static str,
        update: impl FnOnce(&mut CleanMut<'_>) -> Result<(), ParseConfigError>,
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
        update: impl FnOnce(&mut CleanMut<'_>) -> Result<(), ParseConfigError>,
    ) -> Task<Message> {
        self.update_clean_entry(event, update, Self::refresh_after_clean_color_update)
    }

    /// Applies `update` to the selected entry, then runs `on_success`. If a
    /// raw TOML draft is pending, it is discarded first — see
    /// docs/specs/application-workspace.md's "Direct configuration controls"
    /// section — so `update` always runs against a clean entry. If `update`
    /// fails, the discarded draft is restored rather than lost: the direct
    /// control's change didn't end up applied either, so the user should not
    /// lose both the draft and the attempted change.
    fn update_clean_entry(
        &mut self,
        event: &'static str,
        update: impl FnOnce(&mut CleanMut<'_>) -> Result<(), ParseConfigError>,
        on_success: impl FnOnce(&mut Self) -> Task<Message>,
    ) -> Task<Message> {
        let entry = self.config_workspace.selected_mut();
        let pending_draft = entry.is_dirty().then(|| entry.draft_text().into_owned());
        if let EntryViewMut::Dirty(dirty) = entry.view_mut() {
            log::info!("{event}: discarding pending TOML draft to apply a direct control change");
            dirty.revert();
        }
        let result = match entry.view_mut() {
            EntryViewMut::Clean(mut clean) => update(&mut clean).map_err(|e| e.to_string()),
            EntryViewMut::Dirty(_) => {
                log::error!("{event}: entry still dirty after discarding its draft");
                Err("Internal error: could not apply this change.".to_string())
            }
        };
        match result {
            Ok(()) => on_success(self),
            Err(message) => {
                if let Some(pending_draft) = pending_draft {
                    log::info!("{event}: restoring discarded TOML draft after a failed change");
                    self.config_workspace
                        .selected_mut()
                        .set_draft_text(pending_draft);
                }
                self.error = Some(message);
                Task::none()
            }
        }
    }

    fn refresh_after_clean_color_update(&mut self) -> Task<Message> {
        self.refresh_toml_from_workspace();
        let current_iterations = self.iterations;
        self.recompute_max_iterations();
        self.iterations = current_iterations.min(self.max_iterations);

        self.scene.update_colors(&self.render_colors());
        Task::none()
    }

    fn reset_color_memory_from_workspace(&mut self) {
        let entry = self.config_workspace.selected();
        self.color_memory = ColorControlMemory::from_editor_config(
            &entry.editor_config().colors,
            &ConfigDefaults::embedded().colors,
        );
    }

    fn reset_hue_rotation(&mut self) {
        self.hue_rotation.stop();
        self.hue_rotation_phase_degrees = 0.0;
        self.scene.set_hue_offset_degrees(0.0);
    }

    fn sync_controls_from_workspace(&mut self) {
        let iterations = self
            .config_workspace
            .selected()
            .editor_config()
            .generation
            .iterations;
        self.recompute_max_iterations();
        self.iterations = iterations.min(self.max_iterations);
    }

    fn recompute_max_iterations(&mut self) {
        let generation = &self.config_workspace.selected().editor_config().generation;
        let max_seg = lsystem_renderer::line_renderer::max_segments_for_line_color(
            generation.dimensions,
            generation.has_stack_directives(),
        );
        self.max_iterations =
            lsystem_core::max_safe_iterations(&generation.axiom, &generation.rules, max_seg);
    }

    pub(super) fn selected_editor_config(&self) -> &EditorConfig {
        self.config_workspace.selected().editor_config()
    }

    fn effective_render_config(&self) -> Config {
        self.selected_editor_config()
            .resolve(ConfigDefaults::embedded(), self.iterations)
    }

    fn render_colors(&self) -> ColorConfig {
        let editor_config = self.selected_editor_config();
        editor_config
            .colors
            .resolve(&ConfigDefaults::embedded().colors)
    }

    fn control_line_color(&self) -> LineColorConfig {
        line_color_for_controls(
            &self.selected_editor_config().colors,
            &ConfigDefaults::embedded().colors.line,
        )
    }

    pub(super) fn effective_is_3d(&self) -> bool {
        matches!(
            self.selected_editor_config().generation.dimensions,
            Dimensions::ThreeD
        )
    }

    fn schedule_scene_generation(&mut self) -> Task<Message> {
        let config = self.effective_render_config();

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
        let config = self.effective_render_config();
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
        let png_height = if matches!(kind, ExportKind::Png) {
            match self.normalized_png_height() {
                Ok(height) => height,
                Err(error) => {
                    return Task::done(Message::ExportFinished(ExportOutcome::Failed(error)));
                }
            }
        } else {
            self.png_height
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
                    height: png_height,
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
                height: png_height,
                camera: self.scene.camera.clone(),
            },
        };

        Task::perform(handle_export(request), Message::ExportFinished)
    }

    fn normalized_png_width(&mut self) -> Result<u32, String> {
        let Ok(width) = self.png_width_text.trim().parse::<u32>() else {
            return Err(format!(
                "PNG width must be a number from {PNG_MIN_DIMENSION} to {PNG_MAX_DIMENSION}"
            ));
        };

        let width = width.clamp(PNG_MIN_DIMENSION, PNG_MAX_DIMENSION);
        self.png_width = width;
        self.png_width_text = width.to_string();
        Ok(width)
    }

    fn normalized_png_height(&mut self) -> Result<u32, String> {
        let Ok(height) = self.png_height_text.trim().parse::<u32>() else {
            return Err(format!(
                "PNG height must be a number from {PNG_MIN_DIMENSION} to {PNG_MAX_DIMENSION}"
            ));
        };

        let height = height.clamp(PNG_MIN_DIMENSION, PNG_MAX_DIMENSION);
        self.png_height = height;
        self.png_height_text = height.to_string();
        Ok(height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_control_memory_uses_defaults_and_restores_edits() {
        let solid = Some(EditorLineColorConfig::Solid(Rgb::new(0x1a, 0x33, 0x4d)));
        let gradient = Some(EditorLineColorConfig::Gradient {
            start: Some(Rgb::new(0x66, 0x80, 0x99)),
            end: Some(Rgb::new(0xb3, 0xcc, 0xe5)),
            topological_depth: Some(false),
        });
        let topological_gradient = Some(EditorLineColorConfig::Gradient {
            start: Some(Rgb::new(0x33, 0x4d, 0x66)),
            end: Some(Rgb::new(0x80, 0x99, 0xb3)),
            topological_depth: Some(true),
        });

        let mut memory = ColorControlMemory::from_editor_config(
            &lsystem_app_model::EditorColorConfig {
                background: Some(Rgb::new(0xcc, 0xcc, 0xcc)),
                line: Some(lsystem_app_model::EditorLineColorConfig::Solid(Rgb::new(
                    0x1a, 0x33, 0x4d,
                ))),
            },
            &lsystem_app_model::ConfigDefaults::embedded().colors,
        );

        assert_eq!(memory.background(), Rgb::new(0xcc, 0xcc, 0xcc));
        assert_eq!(memory.line_for(LineColorMode::Solid), solid);
        assert_eq!(
            memory.line_for(LineColorMode::Gradient),
            Some(EditorLineColorConfig::Gradient {
                start: None,
                end: None,
                topological_depth: None,
            })
        );
        assert_eq!(
            memory.line_for(LineColorMode::HueCycle),
            Some(EditorLineColorConfig::HueCycle { initial: None })
        );

        memory.remember_background(Rgb::new(0xe5, 0x1a, 0x33));
        memory.remember_line(gradient);
        memory.remember_line(topological_gradient);

        assert_eq!(memory.background(), Rgb::new(0xe5, 0x1a, 0x33));
        assert_eq!(memory.line_for(LineColorMode::Solid), solid);
        assert_eq!(
            memory.line_for(LineColorMode::Gradient),
            topological_gradient
        );
    }

    #[test]
    fn clean_apply_preserves_inactive_line_color_memory() {
        let (mut app, _) = FractalApp::new();
        let gradient = Some(EditorLineColorConfig::Gradient {
            start: Some(Rgb::new(0x66, 0x80, 0x99)),
            end: Some(Rgb::new(0xb3, 0xcc, 0xe5)),
            topological_depth: Some(false),
        });
        app.color_memory.remember_line(gradient);

        let _ = app.update(Message::ApplyConfig);

        assert_eq!(app.color_memory.line_for(LineColorMode::Gradient), gradient);
    }

    #[test]
    fn png_dimensions_default_to_square_800() {
        let (app, _) = FractalApp::new();

        assert_eq!(app.png_width, 800);
        assert_eq!(app.png_width_text, "800");
        assert_eq!(app.png_height, 800);
        assert_eq!(app.png_height_text, "800");
    }

    #[test]
    fn camera_auto_rotation_defaults_to_twenty_degrees_per_second() {
        let (app, _) = FractalApp::new();

        assert_eq!(
            app.auto_rotate_speed,
            CAMERA_AUTO_ROTATION_DEFAULT_SPEED_DEGREES_PER_SECOND
        );
    }

    #[test]
    fn rename_updates_the_selected_config_and_toml_editor() {
        let (mut app, _) = FractalApp::new();

        let _ = app.update(Message::BeginRename);
        assert_eq!(
            app.rename_draft.as_deref(),
            Some(app.config_workspace.selected().name())
        );

        let _ = app.update(Message::RenameDraftChanged("  Renamed Plant  ".to_string()));
        let _ = app.update(Message::CommitRename);

        assert_eq!(app.rename_draft, None);
        assert_eq!(app.config_workspace.selected().name(), "Renamed Plant");
        assert!(app.toml.text().contains("name = \"Renamed Plant\""));
    }

    #[test]
    fn rename_prefills_and_preserves_an_unapplied_draft_name() {
        let (mut app, _) = FractalApp::new();
        let applied_name = app.config_workspace.selected().name().to_string();
        let draft = app.config_workspace.selected().draft_text().replace(
            &format!("name = \"{applied_name}\""),
            "name = \"Draft Plant\"",
        );
        app.toml = iced::widget::text_editor::Content::with_text(&draft);
        app.config_workspace.selected_mut().set_draft_text(draft);

        let _ = app.update(Message::BeginRename);
        assert_eq!(app.rename_draft.as_deref(), Some("Draft Plant"));

        let _ = app.update(Message::CommitRename);

        assert_eq!(app.config_workspace.selected().name(), "Draft Plant");
        assert!(app.toml.text().contains("name = \"Draft Plant\""));
    }

    #[test]
    fn applying_toml_closes_rename_without_reverting_the_applied_name() {
        let (mut app, _) = FractalApp::new();
        let applied_name = app.config_workspace.selected().name().to_string();
        let draft = app.config_workspace.selected().draft_text().replace(
            &format!("name = \"{applied_name}\""),
            "name = \"Applied Plant\"",
        );
        app.toml = iced::widget::text_editor::Content::with_text(&draft);
        app.config_workspace.selected_mut().set_draft_text(draft);
        let _ = app.update(Message::BeginRename);
        let _ = app.update(Message::RenameDraftChanged("Stale Rename".to_string()));

        let _ = app.update(Message::ApplyConfig);
        let _ = app.update(Message::CommitRename);

        assert_eq!(app.rename_draft, None);
        assert_eq!(app.config_workspace.selected().name(), "Applied Plant");
    }

    #[test]
    fn editing_toml_closes_rename_mode() {
        let (mut app, _) = FractalApp::new();
        let _ = app.update(Message::BeginRename);

        let _ = app.update(Message::TomlEdited(
            iced::widget::text_editor::Action::Edit(iced::widget::text_editor::Edit::Insert(' ')),
        ));

        assert_eq!(app.rename_draft, None);
    }

    #[test]
    fn camera_auto_rotation_speed_is_clamped() {
        let (mut app, _) = FractalApp::new();

        let _ = app.update(Message::SetAutoRotateSpeed(0.0));
        assert_eq!(
            app.auto_rotate_speed,
            CAMERA_AUTO_ROTATION_MIN_SPEED_DEGREES_PER_SECOND
        );

        let _ = app.update(Message::SetAutoRotateSpeed(500.0));
        assert_eq!(
            app.auto_rotate_speed,
            CAMERA_AUTO_ROTATION_MAX_SPEED_DEGREES_PER_SECOND
        );
    }

    #[test]
    fn angle_changed_discards_pending_toml_draft_and_applies() {
        let (mut app, _) = FractalApp::new();
        let applied_name = app.config_workspace.selected().name().to_string();
        let draft = app.config_workspace.selected().draft_text().replace(
            &format!("name = \"{applied_name}\""),
            "name = \"Draft Plant\"",
        );
        app.toml = iced::widget::text_editor::Content::with_text(&draft);
        app.config_workspace.selected_mut().set_draft_text(draft);
        assert!(app.config_workspace.selected().is_dirty());

        let _ = app.update(Message::AngleChanged(90.0));

        assert!(!app.config_workspace.selected().is_dirty());
        assert_eq!(app.config_workspace.selected().name(), applied_name);
        assert_eq!(
            app.config_workspace
                .selected()
                .editor_config()
                .generation
                .angle,
            90.0
        );
    }

    #[test]
    fn background_color_changed_discards_pending_toml_draft_and_applies() {
        let (mut app, _) = FractalApp::new();
        let applied_name = app.config_workspace.selected().name().to_string();
        let draft = app.config_workspace.selected().draft_text().replace(
            &format!("name = \"{applied_name}\""),
            "name = \"Draft Plant\"",
        );
        app.toml = iced::widget::text_editor::Content::with_text(&draft);
        app.config_workspace.selected_mut().set_draft_text(draft);
        assert!(app.config_workspace.selected().is_dirty());

        let new_background = Rgb::new(0x11, 0x22, 0x33);
        let _ = app.update(Message::BackgroundColorChanged(new_background));

        assert!(!app.config_workspace.selected().is_dirty());
        assert_eq!(app.config_workspace.selected().name(), applied_name);
        assert_eq!(
            app.config_workspace
                .selected()
                .editor_config()
                .colors
                .background,
            Some(new_background)
        );
    }

    #[test]
    fn angle_changed_with_invalid_value_preserves_pending_toml_draft() {
        let (mut app, _) = FractalApp::new();
        let applied_name = app.config_workspace.selected().name().to_string();
        let draft = app.config_workspace.selected().draft_text().replace(
            &format!("name = \"{applied_name}\""),
            "name = \"Draft Plant\"",
        );
        app.toml = iced::widget::text_editor::Content::with_text(&draft);
        app.config_workspace
            .selected_mut()
            .set_draft_text(draft.clone());
        assert!(app.config_workspace.selected().is_dirty());

        let _ = app.update(Message::AngleChanged(f32::NAN));

        // The angle change failed validation — the pending TOML draft must
        // survive, not be silently destroyed, and the failure must surface.
        assert!(app.config_workspace.selected().is_dirty());
        assert_eq!(app.config_workspace.selected().draft_text(), draft);
        assert!(app.error.is_some());
    }

    #[test]
    fn preset_selected_message_selects_by_id() {
        let (mut app, _) = FractalApp::new();
        let first_id = app.config_workspace.selected_id();
        let _ = app.update(Message::CopyConfig);
        let copied_id = app.config_workspace.selected_id();
        assert_ne!(first_id, copied_id);

        let _ = app.update(Message::PresetSelected(first_id));

        assert_eq!(app.config_workspace.selected_id(), first_id);
    }

    #[test]
    fn png_height_change_clamps_and_normalizes_text() {
        let (mut app, _) = FractalApp::new();

        let _ = app.update(Message::PngHeightChanged("9000".to_string()));
        assert_eq!(app.png_height, PNG_MAX_DIMENSION);
        assert_eq!(app.png_height_text, "9000");

        assert_eq!(app.normalized_png_height().unwrap(), PNG_MAX_DIMENSION);
        assert_eq!(app.png_height_text, PNG_MAX_DIMENSION.to_string());
    }

    #[test]
    fn render_colors_preserves_topological_depth_for_bracketless_grammar() {
        // After removing the normalize_line_color_for_render boundary leak, render_colors()
        // faithfully returns the authored color — topological_depth: true is preserved even
        // for bracketless grammars. Callers that allocate geometry decide independently via
        // `config.colors.line.needs_topological_depth() && config.generation.has_stack_directives()`.
        let (mut app, _) = FractalApp::new();
        let EntryViewMut::Clean(mut clean) = app.config_workspace.selected_mut().view_mut() else {
            panic!("initial config entry should be clean");
        };
        clean.set_grammar("F", &[]).unwrap();

        let start = Rgb::new(0x33, 0x4d, 0x66);
        let end = Rgb::new(0x80, 0x99, 0xb3);
        let _ = app.update(Message::LineColorChanged(Some(
            EditorLineColorConfig::Gradient {
                start: Some(start),
                end: Some(end),
                topological_depth: Some(true),
            },
        )));

        // render_colors() is now a faithful resolve — no normalization.
        assert_eq!(
            app.render_colors().line,
            LineColorConfig::Gradient {
                start,
                end,
                topological_depth: true,
            }
        );
    }

    #[test]
    fn color_mode_change_does_not_schedule_or_cancel_geometry() {
        let (mut app, _) = FractalApp::new();
        let generation_before = app.scene_generation.load(Ordering::Acquire);
        app.scene_pending = false;

        let _ = app.update(Message::LineColorModeSelected(LineColorMode::Gradient));
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
        let background = Rgb::new(0x33, 0x4d, 0x66);

        let _ = app.update(Message::BackgroundColorChanged(background));
        let _ = app.update(Message::BackgroundDefaultToggled(true)); // true = use default (remove)

        assert_eq!(
            app.config_workspace
                .selected()
                .editor_config()
                .colors
                .background,
            None
        );

        let _ = app.update(Message::BackgroundDefaultToggled(false)); // false = explicit (restore)

        assert_eq!(
            app.config_workspace
                .selected()
                .editor_config()
                .colors
                .background,
            Some(background)
        );
    }

    #[test]
    fn angle_change_updates_editor_config_from_workspace() {
        let (mut app, _) = FractalApp::new();
        let _ = app.update(Message::AngleChanged(45.0));

        assert_eq!(app.selected_editor_config().generation.angle, 45.0);
    }

    #[test]
    fn angle_changed_while_dirty_discards_draft_and_applies() {
        let (mut app, _) = FractalApp::new();
        let original_angle = app.selected_editor_config().generation.angle;
        let modified = format!("{} ", app.config_workspace.selected().draft_text());
        app.config_workspace.selected_mut().set_draft_text(modified);
        assert!(app.config_workspace.selected().is_dirty());

        let new_angle = original_angle + 10.0;
        let _ = app.update(Message::AngleChanged(new_angle));

        assert!(!app.config_workspace.selected().is_dirty());
        assert_eq!(app.selected_editor_config().generation.angle, new_angle);
    }

    #[test]
    fn hue_rotation_toggle_off_resets_phase() {
        let (mut app, _) = FractalApp::new();

        let _ = app.update(Message::LineColorModeSelected(LineColorMode::HueCycle));
        let _ = app.update(Message::ToggleHueRotation);
        let _ = app.update(Message::AnimationTick);

        assert!(app.hue_rotation.is_enabled());
        assert_ne!(app.hue_rotation_phase_degrees, 0.0);
        assert_ne!(app.scene.hue_offset_degrees(), 0.0);

        let _ = app.update(Message::ToggleHueRotation);

        assert!(!app.hue_rotation.is_enabled());
        assert_eq!(app.hue_rotation_phase_degrees, 0.0);
        assert_eq!(app.scene.hue_offset_degrees(), 0.0);
    }

    #[test]
    fn orbit_drag_pauses_only_camera_auto_rotation() {
        assert!(camera_auto_rotation_active(true, true, false));
        assert!(!camera_auto_rotation_active(true, true, true));
        assert!(!camera_auto_rotation_active(false, true, false));
        assert!(!camera_auto_rotation_active(true, false, false));

        let (mut app, _) = FractalApp::new();
        app.auto_rotate = true;
        let _ = app.update(Message::LineColorModeSelected(LineColorMode::HueCycle));
        let _ = app.update(Message::ToggleHueRotation);

        let _ = app.update(Message::FractalOrbitStarted);
        assert!(app.orbit_drag_active);
        assert!(app.auto_rotate, "dragging must not disable auto-rotation");

        let _ = app.update(Message::AnimationTick);
        assert_ne!(app.hue_rotation_phase_degrees, 0.0);

        let _ = app.update(Message::FractalOrbitEnded);
        assert!(!app.orbit_drag_active);
        assert!(app.auto_rotate);
    }

    #[test]
    fn hue_rotation_toggle_is_ignored_outside_hue_cycle() {
        let (mut app, _) = FractalApp::new();

        let _ = app.update(Message::LineColorModeSelected(LineColorMode::Solid));
        assert!(!matches!(
            app.render_colors().line,
            LineColorConfig::HueCycle { .. }
        ));

        let _ = app.update(Message::ToggleHueRotation);

        assert!(!app.hue_rotation.is_enabled());
        assert_eq!(app.hue_rotation_phase_degrees, 0.0);
    }

    #[test]
    fn hue_rotation_speed_is_clamped() {
        let (mut app, _) = FractalApp::new();

        let _ = app.update(Message::SetHueRotationSpeed(0.0));
        assert_eq!(app.hue_rotation.speed_degrees_per_second(), 1.0);

        let _ = app.update(Message::SetHueRotationSpeed(75.0));
        assert_eq!(app.hue_rotation.speed_degrees_per_second(), 60.0);
    }

    #[test]
    fn hue_rotation_direction_changes_phase_advance_sign() {
        let (mut app, _) = FractalApp::new();

        let _ = app.update(Message::LineColorModeSelected(LineColorMode::HueCycle));
        let _ = app.update(Message::ToggleHueRotation);
        let _ = app.update(Message::SetHueRotationSpeed(60.0));
        let _ = app.update(Message::SetHueRotationDirection(
            HueRotationDirection::Reverse,
        ));
        let _ = app.update(Message::AnimationTick);

        assert_eq!(app.hue_rotation_phase_degrees, 359.0);
    }

    #[test]
    fn leaving_hue_cycle_preserves_hue_rotation_state_and_ignores_ticks() {
        let (mut app, _) = FractalApp::new();

        let _ = app.update(Message::LineColorModeSelected(LineColorMode::HueCycle));
        let _ = app.update(Message::ToggleHueRotation);
        let _ = app.update(Message::AnimationTick);
        let phase_before_mode_change = app.hue_rotation_phase_degrees;

        let _ = app.update(Message::LineColorModeSelected(LineColorMode::Solid));
        let _ = app.update(Message::AnimationTick);

        assert!(app.hue_rotation.is_enabled());
        assert_eq!(app.hue_rotation_phase_degrees, phase_before_mode_change);
    }

    #[test]
    fn effective_is_3d_uses_config_not_stale_scene() {
        let (mut app, _) = FractalApp::new();
        let three_d_config = r##"[metadata]
name = "3D"

[l-system]
dimensions = "3D"
axiom = "F"
iterations = 1
angle = 60.0
step = 1.0
initial_heading = 0.0

[l-system.rules]
F = "F"

[colors.line]
solid = "#00e680"
"##;
        app.config_workspace =
            ConfigWorkspace::from_presets(vec![("3d", three_d_config.to_string())]).unwrap();

        assert!(!app.scene.is_3d());
        assert!(app.effective_is_3d());
    }
}
