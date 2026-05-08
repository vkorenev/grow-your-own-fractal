use iced::keyboard;
use iced::widget::row;
use iced::{Element, Event, Length, Point, Size, Subscription, Task, event};
use include_dir::{Dir, include_dir};
use lsystem_core::Config;
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

#[derive(Debug, Clone)]
pub(super) enum Message {
    PresetSelected(String),
    TomlEdited(iced::widget::text_editor::Action),
    ApplyConfig,
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
    FractalZoom {
        delta_y: f32,
        cursor: Point,
        size: Size,
    },
}

pub(super) struct FractalApp {
    pub(super) presets: Vec<Preset>,
    pub(super) selected_preset: Option<String>,
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
    scene_generation: Arc<AtomicU64>,
}

impl FractalApp {
    pub(super) fn new() -> (Self, Task<Message>) {
        let presets = load_presets();
        let selected_preset = presets.first().map(|preset| preset.name.clone());
        let toml_text = presets
            .first()
            .map(|preset| preset.toml)
            .unwrap_or_default()
            .to_string();

        let mut app = Self {
            presets,
            selected_preset,
            toml: iced::widget::text_editor::Content::with_text(&toml_text),
            base_config: None,
            iterations: 1,
            max_iterations: 1,
            angle: 60.0,
            png_width: 2048,
            png_width_text: "2048".to_string(),
            error: None,
            export_status: None,
            scene_pending: false,
            scene: Scene::default(),
            scene_generation: Arc::new(AtomicU64::new(0)),
        };
        let task = app.apply_config();
        (app, task)
    }

    pub(super) fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::PresetSelected(name) => {
                if let Some(preset) = self.presets.iter().find(|preset| preset.name == name) {
                    self.selected_preset = Some(preset.name.clone());
                    self.toml = iced::widget::text_editor::Content::with_text(preset.toml);
                    return self.apply_config();
                }
                Task::none()
            }
            Message::TomlEdited(action) => {
                self.toml.perform(action);
                self.export_status = None;
                Task::none()
            }
            Message::ApplyConfig => self.apply_config(),
            Message::IterationsChanged(iterations) => {
                self.iterations = iterations.min(self.max_iterations);
                self.schedule_scene_generation()
            }
            Message::AngleChanged(angle) => {
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
            Message::ExportSvg => self.export(ExportKind::Svg),
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
            Message::FractalZoom {
                delta_y,
                cursor,
                size,
            } => {
                self.scene.zoom_toward_cursor(delta_y, cursor, size);
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
        event::listen_with(|event, status, _window| {
            if status == event::Status::Captured {
                return None;
            }
            match event {
                Event::Keyboard(keyboard::Event::KeyPressed {
                    key: keyboard::Key::Character(ch),
                    repeat: false,
                    ..
                }) if ch.eq_ignore_ascii_case("f") => Some(Message::Fit),
                _ => None,
            }
        })
    }

    fn apply_config(&mut self) -> Task<Message> {
        match Config::parse(&self.toml.text()) {
            Ok(config) => {
                self.max_iterations = lsystem_core::max_safe_iterations(
                    &config.axiom,
                    &config.rules,
                    lsystem_renderer::line_renderer::MAX_SEGMENTS,
                ) as u32;
                self.iterations = config.iterations.min(self.max_iterations);
                self.angle = config.angle;
                self.base_config = Some(config);
                self.error = None;
                self.export_status = None;
                self.schedule_scene_generation()
            }
            Err(error) => {
                self.error = Some(error.to_string());
                self.base_config = None;
                self.cancel_scene_generation();
                Task::none()
            }
        }
    }

    fn effective_config(&self) -> Option<Config> {
        self.base_config.clone().map(|mut config| {
            config.iterations = self.iterations;
            config.angle = self.angle;
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

        Task::perform(
            build_scene(config, generation, token),
            Message::SceneGenerated,
        )
    }

    fn cancel_scene_generation(&mut self) {
        self.scene_generation.fetch_add(1, Ordering::AcqRel);
        self.scene_pending = false;
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
            let Some(path) = choose_export_path(&config, kind) else {
                return Task::done(Message::ExportFinished(ExportOutcome::Cancelled));
            };

            match kind {
                ExportKind::Svg => ExportRequest::Svg { config, path },
                ExportKind::Png => ExportRequest::Png {
                    config,
                    width: png_width,
                    path,
                },
            }
        };

        #[cfg(target_arch = "wasm32")]
        let request = match kind {
            ExportKind::Svg => ExportRequest::Svg(config),
            ExportKind::Png => ExportRequest::Png {
                config,
                width: png_width,
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

#[derive(Debug, Clone)]
pub(super) struct Preset {
    pub(super) name: String,
    pub(super) toml: &'static str,
}

fn load_presets() -> Vec<Preset> {
    let mut files: Vec<_> = PRESETS_DIR
        .files()
        .filter(|file| file.path().extension().and_then(|ext| ext.to_str()) == Some("toml"))
        .collect();
    files.sort_by_key(|file| file.path());
    files
        .into_iter()
        .filter_map(|file| {
            let toml = file.contents_utf8()?;
            let name = Config::parse(toml).ok()?.name;
            Some(Preset { name, toml })
        })
        .collect()
}
