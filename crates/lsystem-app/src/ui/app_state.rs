use iced::keyboard;
use iced::widget::row;
use iced::{Element, Event, Length, Point, Size, Subscription, Task, event, window};
use include_dir::{Dir, include_dir};
use lsystem_core::{Config, ConfigWorkspace};
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

#[derive(Debug, Clone)]
pub(super) enum Message {
    PresetSelected(String),
    TomlEdited(iced::widget::text_editor::Action),
    ApplyConfig,
    RevertConfig,
    ResetConfig,
    IterationsChanged(u32),
    AngleChanged(f32),
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
    AnimationTick,
}

pub(super) struct FractalApp {
    pub(super) config_workspace: ConfigWorkspace,
    pub(super) toml: iced::widget::text_editor::Content,
    pub(super) base_config: Option<Config>,
    pub(super) iterations: u32,
    pub(super) max_iterations: u32,
    pub(super) angle: f32,
    pub(super) png_width: u32,
    pub(super) png_width_text: String,
    pub(super) error: Option<String>,
    pub(super) export_status: Option<String>,
    pub(super) scene_pending: bool,
    pub(super) scene: Scene,
    pub(super) auto_rotate: bool,
    pub(super) auto_rotate_speed: f32,
    scene_generation: Arc<AtomicU64>,
}

impl FractalApp {
    pub(super) fn new() -> (Self, Task<Message>) {
        let config_workspace = ConfigWorkspace::from_presets(load_presets())
            .expect("at least one bundled preset should parse");
        let toml_text = config_workspace.selected_draft_text().to_string();
        let base_config = config_workspace.selected_applied_config().clone();

        let mut app = Self {
            config_workspace,
            toml: iced::widget::text_editor::Content::with_text(&toml_text),
            base_config: Some(base_config),
            iterations: 1,
            max_iterations: 1,
            angle: 60.0,
            png_width: 2048,
            png_width_text: "2048".to_string(),
            error: None,
            export_status: None,
            scene_pending: false,
            scene: Scene::default(),
            auto_rotate: false,
            auto_rotate_speed: 45.0,
            scene_generation: Arc::new(AtomicU64::new(0)),
        };
        app.sync_controls_from_base_config();
        let task = app.schedule_scene_generation();
        (app, task)
    }

    pub(super) fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::PresetSelected(name) => {
                if self.config_workspace.select_by_name(&name) {
                    return self.refresh_from_workspace();
                }
                Task::none()
            }
            Message::TomlEdited(action) => {
                self.toml.perform(action);
                self.config_workspace
                    .set_selected_draft_text(self.toml.text());
                self.export_status = None;
                Task::none()
            }
            Message::ApplyConfig => self.apply_config(),
            Message::RevertConfig => {
                self.config_workspace.revert_selected();
                self.refresh_from_workspace()
            }
            Message::ResetConfig => {
                if self.config_workspace.reset_selected().is_some() {
                    self.refresh_from_workspace()
                } else {
                    Task::none()
                }
            }
            Message::IterationsChanged(iterations) => {
                if self.config_workspace.selected_is_dirty() {
                    return Task::none();
                }
                self.iterations = iterations.min(self.max_iterations);
                self.schedule_scene_generation()
            }
            Message::AngleChanged(angle) => {
                if self.config_workspace.selected_is_dirty() {
                    return Task::none();
                }
                self.angle = angle;
                self.schedule_scene_generation()
            }
            Message::PngWidthChanged(value) => {
                self.png_width_text = value;
                self.export_status = None;
                if let Ok(width) = self.png_width_text.parse::<u32>() {
                    self.png_width = width.clamp(PNG_MIN_WIDTH, PNG_MAX_WIDTH);
                }
                Task::none()
            }
            Message::ExportSvg => {
                if self.scene.is_3d() {
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
                if let SceneBuildResult::Ready { generation, scene } = result
                    && self.is_current_generation(generation)
                {
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
            Message::AnimationTick => {
                self.scene
                    .auto_rotate_by(self.auto_rotate_speed * AUTO_ROTATE_DT_SECS);
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

        if is_3d && auto_rotate {
            let frames = window::frames().map(|_| Message::AnimationTick);
            Subscription::batch([key_sub, frames])
        } else {
            key_sub
        }
    }

    fn apply_config(&mut self) -> Task<Message> {
        self.config_workspace
            .set_selected_draft_text(self.toml.text());
        match self.config_workspace.apply_selected() {
            Ok(config) => {
                let config = config.clone();
                self.base_config = Some(config);
                self.sync_controls_from_base_config();
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
        self.toml = iced::widget::text_editor::Content::with_text(
            self.config_workspace.selected_draft_text(),
        );
        self.base_config = Some(self.config_workspace.selected_applied_config().clone());
        self.sync_controls_from_base_config();
        self.error = None;
        self.export_status = None;
        self.schedule_scene_generation()
    }

    fn sync_controls_from_base_config(&mut self) {
        let Some(config) = &self.base_config else {
            return;
        };
        let max_seg = if config.dimensions == 3 {
            lsystem_renderer::line_renderer::MAX_SEGMENTS_3D
        } else {
            lsystem_renderer::line_renderer::MAX_SEGMENTS
        };
        self.max_iterations = lsystem_core::max_safe_iterations(
            &config.generation.axiom,
            &config.generation.rules,
            max_seg,
        ) as u32;
        self.iterations = config.generation.iterations.min(self.max_iterations);
        self.angle = config.generation.angle;
    }

    fn effective_config(&self) -> Option<Config> {
        self.base_config.clone().map(|mut config| {
            config.generation.iterations = self.iterations;
            config.generation.angle = self.angle;
            config
        })
    }

    fn schedule_scene_generation(&mut self) -> Task<Message> {
        let Some(config) = self.effective_config() else {
            return Task::none();
        };

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
        let Some(config) = self.effective_config() else {
            return Task::none();
        };
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
            let toml = file.contents_utf8()?;
            let name = match Config::parse(toml) {
                Ok(config) => config.name,
                Err(err) => {
                    log::error!("Bundled preset {:?} failed to parse: {err}", file.path());
                    return None;
                }
            };
            Some((name, toml.to_string()))
        })
        .collect()
}
