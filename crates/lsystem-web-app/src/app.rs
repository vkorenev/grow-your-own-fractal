use leptos::html::Canvas;
use leptos::prelude::*;
use lsystem_core::{
    CleanMut, ColorConfig, Config, ConfigError, ConfigWorkspace, Dimensions, EntryViewMut,
    LineColorConfig,
};
use lsystem_renderer::line_renderer::FrameSkipReason;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use crate::color_input::{hex_to_rgb, rgb_to_hex};
use crate::export::{export_png, export_svg};
use crate::presets::{load_presets, max_iterations_for_config};
use crate::renderer::{CanvasRenderer, RenderStatus};

type RendererState = StoredValue<Option<CanvasRenderer>, LocalStorage>;

const ROTATION_STEP_DEG: f32 = 5.0;
const HSV_MOVEMENT_DEFAULT_SPEED_DEGREES_PER_SECOND: f32 = 15.0;
const HSV_MOVEMENT_MIN_SPEED_DEGREES_PER_SECOND: f32 = 1.0;
const HSV_MOVEMENT_MAX_SPEED_DEGREES_PER_SECOND: f32 = 60.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LineColorMode {
    Solid,
    Gradient,
    HueCycle,
    DepthGradient,
}

impl LineColorMode {
    fn from_line_color(line_color: &LineColorConfig) -> Self {
        match line_color {
            LineColorConfig::Solid { .. } => Self::Solid,
            LineColorConfig::Gradient { .. } => Self::Gradient,
            LineColorConfig::HueCycle { .. } => Self::HueCycle,
            LineColorConfig::DepthGradient { .. } => Self::DepthGradient,
        }
    }

    fn from_key(key: &str) -> Option<Self> {
        match key {
            "solid" => Some(Self::Solid),
            "gradient" => Some(Self::Gradient),
            "hue_cycle" => Some(Self::HueCycle),
            "depth_gradient" => Some(Self::DepthGradient),
            _ => None,
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Solid => "solid",
            Self::Gradient => "gradient",
            Self::HueCycle => "hue_cycle",
            Self::DepthGradient => "depth_gradient",
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HsvMovementDirection {
    Forward,
    Reverse,
}

impl HsvMovementDirection {
    fn from_key(key: &str) -> Option<Self> {
        match key {
            "forward" => Some(Self::Forward),
            "reverse" => Some(Self::Reverse),
            _ => None,
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Forward => "forward",
            Self::Reverse => "reverse",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Forward => "Forward",
            Self::Reverse => "Reverse",
        }
    }

    fn sign(self) -> f32 {
        match self {
            Self::Forward => 1.0,
            Self::Reverse => -1.0,
        }
    }
}

fn advance_hsv_phase_degrees(
    phase_degrees: f32,
    speed_degrees_per_second: f32,
    dt_seconds: f32,
    direction: HsvMovementDirection,
) -> f32 {
    (phase_degrees + direction.sign() * speed_degrees_per_second * dt_seconds).rem_euclid(360.0)
}

fn hsv_movement_is_active(enabled: bool, line_color: &LineColorConfig) -> bool {
    enabled && matches!(line_color, LineColorConfig::HueCycle { .. })
}

#[derive(Clone, Copy)]
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

#[component]
pub(crate) fn App() -> impl IntoView {
    let initial_workspace =
        ConfigWorkspace::from_presets(load_presets()).expect("bundled presets should parse");
    let initial_config = initial_workspace.selected().applied_config().clone();

    let config_workspace = RwSignal::new(initial_workspace);
    let color_memory = RwSignal::new(ColorControlMemory::from_colors(&initial_config.colors));
    let error = RwSignal::new(None::<String>);
    let png_width = RwSignal::new(2048u32);
    let gpu_error = RwSignal::new(None::<String>);
    let auto_rotate = RwSignal::new(false);
    let auto_rotate_speed = RwSignal::new(45.0f32);
    let hsv_movement = RwSignal::new(false);
    let hsv_movement_speed = RwSignal::new(HSV_MOVEMENT_DEFAULT_SPEED_DEGREES_PER_SECOND);
    let hsv_movement_direction = RwSignal::new(HsvMovementDirection::Forward);
    let hsv_movement_phase = RwSignal::new(0.0f32);
    // Generation counter: bumped on each animation start so older rAF loops detect
    // they have been superseded and exit.
    let animation_token = RwSignal::new(0u32);

    let base_config =
        Memo::new(move |_| config_workspace.with(|ws| ws.selected().applied_config().clone()));
    let toml_text =
        Memo::new(move |_| config_workspace.with(|ws| ws.selected().draft_text().into_owned()));
    let max_iterations = Memo::new(move |_| {
        config_workspace.with(|ws| max_iterations_for_config(ws.selected().applied_config()))
    });
    let iterations = Memo::new(move |_| {
        let max = max_iterations.get();
        config_workspace.with(|ws| {
            ws.selected()
                .applied_config()
                .generation
                .iterations
                .min(max)
        })
    });
    let angle = Memo::new(move |_| {
        config_workspace.with(|ws| ws.selected().applied_config().generation.angle)
    });

    let renderer: RendererState = StoredValue::new_local(None::<CanvasRenderer>);
    let last_pointer = StoredValue::new(None::<(i32, f64, f64)>);

    let is_3d = move || matches!(base_config.get().generation.dimensions, Dimensions::ThreeD);
    let is_3d_untracked = move || {
        matches!(
            base_config.get_untracked().generation.dimensions,
            Dimensions::ThreeD
        )
    };
    let is_dirty = move || config_workspace.with(|workspace| workspace.selected().is_dirty());

    let canvas_ref = NodeRef::<Canvas>::new();

    let recover_after_render =
        move |status: RenderStatus, canvas: web_sys::HtmlCanvasElement| match status {
            RenderStatus::Rendered
            | RenderStatus::Skipped(FrameSkipReason::Timeout | FrameSkipReason::Occluded) => {}
            RenderStatus::Skipped(reason) => {
                log::error!("Skipped GPU frame: {reason}");
            }
            RenderStatus::SurfaceLost => {
                log::error!("GPU surface was lost; attempting to recreate it");
                wasm_bindgen_futures::spawn_local(async move {
                    let Some(Some(mut renderer_state)) =
                        renderer.try_update_value(|opt| opt.take())
                    else {
                        return;
                    };
                    match renderer_state.recover_surface(canvas.clone()).await {
                        Ok(_) => {
                            let mut config = base_config.get_untracked();
                            config.generation.iterations = iterations.get_untracked();
                            config.generation.angle = angle.get_untracked();
                            match renderer_state
                                .set_config_preserving_camera_and_render(&canvas, &config)
                            {
                                RenderStatus::Rendered
                                | RenderStatus::Skipped(
                                    FrameSkipReason::Timeout | FrameSkipReason::Occluded,
                                ) => {
                                    gpu_error.set(None);
                                    renderer.update_value(|opt| *opt = Some(renderer_state));
                                }
                                RenderStatus::Skipped(reason) => {
                                    log::error!(
                                        "Skipped GPU frame after surface recovery: {reason}"
                                    );
                                    gpu_error.set(None);
                                    renderer.update_value(|opt| *opt = Some(renderer_state));
                                }
                                RenderStatus::SurfaceLost => {
                                    log::error!("GPU surface was lost again after recovery");
                                    gpu_error.set(Some(
                                        "GPU surface was lost again after recovery".to_string(),
                                    ));
                                }
                            }
                        }
                        Err(err) => {
                            log::error!("Failed to recover GPU surface: {err}");
                            gpu_error.set(Some(err.to_string()));
                        }
                    }
                });
            }
        };

    let render_current = move || {
        let Some(canvas) = canvas_ref.get_untracked() else {
            return;
        };
        let mut config = base_config.get_untracked();
        config.generation.iterations = iterations.get_untracked();
        config.generation.angle = angle.get_untracked();
        with_renderer(canvas, renderer, recover_after_render, |r, c| {
            r.set_config_and_render(c, &config)
        });
    };

    let render_current_colors = move || {
        let Some(canvas) = canvas_ref.get_untracked() else {
            return;
        };
        let config = base_config.get_untracked();
        with_renderer(canvas, renderer, recover_after_render, |r, c| {
            r.set_colors_and_render(c, &config)
        });
    };

    canvas_ref.on_load(move |canvas| {
        wasm_bindgen_futures::spawn_local(async move {
            match CanvasRenderer::new(canvas.clone()).await {
                Ok(new_renderer) => {
                    gpu_error.set(None);
                    renderer.update_value(|opt| *opt = Some(new_renderer));
                    render_current();
                }
                Err(err) => {
                    log::error!("Failed to initialize GPU renderer: {err}");
                    gpu_error.set(Some(err.to_string()));
                }
            }
        });
    });

    install_resize_listener(canvas_ref, renderer, recover_after_render);

    let refresh_color_memory = move || {
        color_memory.set(ColorControlMemory::from_colors(
            &base_config.get_untracked().colors,
        ));
    };

    let reset_hsv_movement = move || {
        let was_active = hsv_movement.get_untracked() || hsv_movement_phase.get_untracked() != 0.0;
        hsv_movement.set(false);
        hsv_movement_phase.set(0.0);
        if was_active && let Some(canvas) = canvas_ref.get_untracked() {
            with_renderer(canvas, renderer, recover_after_render, |r, c| {
                r.animate_and_render(c, None, Some(0.0))
            });
        }
    };

    let stop_auto_rotate_if_2d = move || {
        if !is_3d_untracked() && auto_rotate.get_untracked() {
            auto_rotate.set(false);
        }
    };

    let start_animation_loop = move || {
        animation_token.update(|t| *t = t.wrapping_add(1));
        let token = animation_token.get_untracked();
        wasm_bindgen_futures::spawn_local(async move {
            let mut prev_ts: Option<f64> = None;
            loop {
                let ts = match next_animation_frame().await {
                    Ok(ts) => ts,
                    Err(reason) => {
                        log::error!("requestAnimationFrame failed ({reason}); stopping animation");
                        auto_rotate.set(false);
                        hsv_movement.set(false);
                        hsv_movement_phase.set(0.0);
                        error.set(Some(
                            "Animation stopped unexpectedly. Try toggling it again.".to_string(),
                        ));
                        break;
                    }
                };
                if animation_token.get_untracked() != token {
                    break;
                }

                let auto_active = auto_rotate.get_untracked() && is_3d_untracked();
                let line_color = base_config.with_untracked(line_color_of);
                if !(auto_active
                    || hsv_movement_is_active(hsv_movement.get_untracked(), &line_color))
                {
                    break;
                }
                let hsv_active = hsv_movement_is_active(hsv_movement.get_untracked(), &line_color);

                // Clamp dt to 100 ms to prevent a large jump after the tab was backgrounded.
                let dt = prev_ts
                    .map_or(1.0_f32 / 60.0, |p| ((ts - p) / 1000.0) as f32)
                    .min(1.0 / 10.0);
                prev_ts = Some(ts);

                let auto_degrees = auto_active.then(|| auto_rotate_speed.get_untracked() * dt);
                let hue_phase = hsv_active.then(|| {
                    let next = advance_hsv_phase_degrees(
                        hsv_movement_phase.get_untracked(),
                        hsv_movement_speed.get_untracked(),
                        dt,
                        hsv_movement_direction.get_untracked(),
                    );
                    hsv_movement_phase.set(next);
                    next
                });

                if let Some(canvas) = canvas_ref.get_untracked() {
                    with_renderer(canvas, renderer, recover_after_render, |r, c| {
                        r.animate_and_render(c, auto_degrees, hue_phase)
                    });
                }
            }
        });
    };

    let resume_hsv_movement_if_active = move || {
        if hsv_movement_is_active(
            hsv_movement.get_untracked(),
            &base_config.with_untracked(line_color_of),
        ) {
            start_animation_loop();
        }
    };

    let apply_current = move || {
        if !config_workspace.with_untracked(|ws| ws.selected().is_dirty()) {
            return;
        }
        let result = config_workspace
            .try_update(|workspace| workspace.apply().map(|_| ()).map_err(|e| e.to_string()));
        match result {
            Some(Ok(())) => {
                error.set(None);
                refresh_color_memory();
                if !is_3d_untracked() && auto_rotate.get_untracked() {
                    auto_rotate.set(false);
                }
                render_current();
                resume_hsv_movement_if_active();
            }
            Some(Err(msg)) => error.set(Some(msg)),
            None => {
                log::error!("apply: config_workspace signal was unavailable");
                error.set(Some("Internal error: could not apply config.".to_string()));
            }
        }
    };

    let select_current_config = move || {
        error.set(None);
        stop_auto_rotate_if_2d();
        refresh_color_memory();
        render_current();
        resume_hsv_movement_if_active();
    };

    let toggle_auto_rotate = move |_: web_sys::MouseEvent| {
        if auto_rotate.get_untracked() {
            auto_rotate.set(false);
        } else {
            auto_rotate.set(true);
            start_animation_loop();
        }
    };

    let toggle_hsv_movement = move |_: web_sys::MouseEvent| {
        if hsv_movement.get_untracked() {
            reset_hsv_movement();
        } else if matches!(
            base_config.with_untracked(line_color_of),
            LineColorConfig::HueCycle { .. }
        ) {
            hsv_movement.set(true);
            start_animation_loop();
        }
    };

    view! {
        <main class="app-shell">
            <aside class="controls">
                <h1>"Grow Your Own Fractal"</h1>

                <label for="preset">"Config"</label>
                <select
                    id="preset"
                    prop:value=move || {
                        config_workspace.with(|workspace| workspace.selected_index().to_string())
                    }
                    on:change:target=move |ev| {
                        let idx = ev.target().value().parse::<usize>().unwrap_or(0);
                        let selected = config_workspace.try_update(|workspace| {
                            workspace.select(idx).map(|_| ()).map_err(|e| e.to_string())
                        });
                        match selected {
                            Some(Ok(())) => {
                                select_current_config();
                            }
                            Some(Err(err)) => {
                                log::error!("select preset: rejected index {idx}: {err}");
                                error.set(Some(err));
                            }
                            None => {
                                log::error!(
                                    "select preset: config_workspace signal was unavailable"
                                );
                                error.set(Some(
                                    "Internal error: could not select config.".to_string(),
                                ));
                            }
                        }
                    }
                >
                    {move || {
                        config_workspace.with(|workspace| {
                            workspace
                                .entries()
                                .iter()
                                .enumerate()
                                .map(|(idx, entry)| {
                                    view! {
                                        <option value=idx.to_string()>{entry.name().to_string()}</option>
                                    }
                                })
                                .collect_view()
                        })
                    }}
                </select>

                <button
                    type="button"
                    on:click=move |_| {
                        let result = config_workspace.try_update(|workspace| {
                            workspace.copy().map(|_| ()).map_err(|e| e.to_string())
                        });
                        match result {
                            Some(Ok(())) => {
                                error.set(None);
                                stop_auto_rotate_if_2d();
                                refresh_color_memory();
                                render_current();
                                resume_hsv_movement_if_active();
                            }
                            Some(Err(msg)) => error.set(Some(msg)),
                            None => {
                                log::error!("copy: config_workspace signal was unavailable");
                                error.set(Some(
                                    "Internal error: could not copy config.".to_string(),
                                ));
                            }
                        }
                    }
                >
                    "Copy"
                </button>

                <label for="config">"Config (TOML)"</label>
                <textarea
                    id="config"
                    spellcheck="false"
                    prop:value=move || toml_text.get()
                    on:input:target=move |ev| {
                        let text = ev.target().value();
                        let updated = config_workspace.try_update(|workspace| {
                            workspace.selected_mut().set_draft_text(text);
                        });
                        if updated.is_none() {
                            log::error!("textarea input: config_workspace signal was unavailable");
                            error.set(Some(
                                "Internal error: could not update config.".to_string(),
                            ));
                        }
                    }
                />

                <div class="row">
                    <button
                        type="button"
                        disabled=move || !is_dirty()
                        on:click=move |_| apply_current()
                    >
                        "Apply"
                    </button>
                    <button
                        type="button"
                        disabled=move || !is_dirty()
                        on:click=move |_| {
                            let ok = config_workspace.try_update(|workspace| {
                                match workspace.selected_mut().view_mut() {
                                    EntryViewMut::Dirty(dirty) => dirty.revert(),
                                    EntryViewMut::Clean(_) => {
                                        log::error!(
                                            "revert fired while entry is clean; UI guards bypassed"
                                        );
                                    }
                                }
                            });
                            if ok.is_some() {
                                error.set(None);
                                stop_auto_rotate_if_2d();
                                refresh_color_memory();
                                render_current();
                                resume_hsv_movement_if_active();
                            } else {
                                log::error!("revert: config_workspace signal was unavailable");
                                error.set(Some(
                                    "Internal error: could not revert config.".to_string(),
                                ));
                            }
                        }
                    >
                        "Revert"
                    </button>
                    <button
                        type="button"
                        disabled=move || {
                            config_workspace.with(|workspace| {
                                workspace.selected().is_dirty() || !workspace.can_reset()
                            })
                        }
                        on:click=move |_| {
                            if is_dirty() {
                                return;
                            }
                            let result = config_workspace.try_update(|workspace| {
                                workspace
                                    .reset()
                                    .map(|opt| opt.is_some())
                                    .map_err(|e| e.to_string())
                            });
                            match result {
                                Some(Ok(true)) => {
                                    error.set(None);
                                    stop_auto_rotate_if_2d();
                                    refresh_color_memory();
                                    render_current();
                                    resume_hsv_movement_if_active();
                                }
                                Some(Ok(false)) => {
                                    log::warn!(
                                        "reset: no-op for entry without a bundled default; button guard may have been bypassed"
                                    );
                                }
                                Some(Err(msg)) => error.set(Some(msg)),
                                None => {
                                    log::error!(
                                        "reset: config_workspace signal was unavailable"
                                    );
                                    error.set(Some(
                                        "Internal error: could not reset config.".to_string(),
                                    ));
                                }
                            }
                        }
                    >
                        "Reset"
                    </button>
                </div>

                <p class=move || if error.get().is_some() { "status error" } else { "status ok" }>
                    {move || error.get().unwrap_or_else(|| "OK".to_string())}
                </p>

                <p class:hidden=move || !is_dirty() class="status">
                    "Apply or Revert the edited config before using controls."
                </p>

                <div class="group" class:hidden=move || is_dirty()>
                    <label for="iterations">"Iterations"</label>
                    <div class="row">
                        <input
                            id="iterations"
                            type="range"
                            min="0"
                            max=move || max_iterations.get().to_string()
                            prop:value=move || iterations.get().to_string()
                            on:input:target=move |ev| {
                                let next = ev
                                    .target()
                                    .value()
                                    .parse::<u32>()
                                    .unwrap_or(0)
                                    .clamp(0, max_iterations.get_untracked());
                                update_clean_config(
                                    config_workspace,
                                    error,
                                    render_current,
                                    "iterations input",
                                    move |clean| clean.set_iterations(next),
                                );
                            }
                        />
                        <output>{move || iterations.get()}</output>
                    </div>

                    <label for="angle">"Angle"</label>
                    <div class="row">
                        <input
                            id="angle"
                            type="range"
                            min="1"
                            max="180"
                            step="0.5"
                            prop:value=move || angle.get().to_string()
                            on:input:target=move |ev| {
                                let next = ev
                                    .target()
                                    .value()
                                    .parse::<f32>()
                                    .unwrap_or(60.0)
                                    .clamp(1.0, 180.0);
                                update_clean_config(
                                    config_workspace,
                                    error,
                                    render_current,
                                    "angle input",
                                    move |clean| clean.set_angle(next),
                                );
                            }
                        />
                        <output>{move || format!("{:.1}", angle.get())}</output>
                    </div>

                    <label class="check-row" for="background-override">
                        <input
                            id="background-override"
                            type="checkbox"
                            prop:checked=move || base_config.with(|c| c.colors.background.is_some())
                            on:change:target=move |ev| {
                                let current_bg =
                                    base_config.with_untracked(|c| c.colors.background);
                                if let Some(background) = current_bg {
                                    color_memory.update(|memory| {
                                        memory.remember_background(background);
                                    });
                                }
                                let enabled = ev.target().checked();
                                let background = if enabled {
                                    Some(color_memory.get_untracked().background())
                                } else {
                                    None
                                };
                                update_clean_config(
                                    config_workspace,
                                    error,
                                    render_current_colors,
                                    "background checkbox",
                                    move |clean| clean.set_background(background),
                                );
                            }
                        />
                        <span>"Background"</span>
                    </label>

                    <div
                        class="color-row"
                        class:hidden=move || base_config.with(|c| c.colors.background.is_none())
                    >
                        <label for="background-color">"Background color"</label>
                        <input
                            id="background-color"
                            type="color"
                            prop:value=move || {
                                rgb_to_hex(base_config.with(|c| c.colors.effective_background()))
                            }
                            on:input:target=move |ev| {
                                let Some(color) = hex_to_rgb(&ev.target().value()) else {
                                    error.set(Some("Invalid color value.".to_string()));
                                    return;
                                };
                                if update_clean_config(
                                    config_workspace,
                                    error,
                                    render_current_colors,
                                    "background color input",
                                    move |clean| clean.set_background(Some(color)),
                                ) {
                                    color_memory.update(|memory| {
                                        memory.remember_background(color);
                                    });
                                }
                            }
                        />
                    </div>

                    <label for="line-color-mode">"Line color"</label>
                    <select
                        id="line-color-mode"
                        prop:value=move || {
                            LineColorMode::from_line_color(&current_line_color(base_config))
                                .key()
                                .to_string()
                        }
                        on:change:target=move |ev| {
                            let mode_key = ev.target().value();
                            let Some(mode) = LineColorMode::from_key(&mode_key) else {
                                log::error!("unknown line color mode selected: {mode_key}");
                                return;
                            };
                            let current = base_config.with_untracked(line_color_of);
                            let Some(line_color) = color_memory.try_update(|memory| {
                                memory.remember_line(current);
                                memory.line_for(mode)
                            }) else {
                                log::error!(
                                    "line color mode select: line color memory signal was unavailable"
                                );
                                error.set(Some(
                                    "Internal error: could not update line color.".to_string(),
                                ));
                                return;
                            };
                            if update_clean_config(
                                config_workspace,
                                error,
                                render_current_colors,
                                "line color mode select",
                                move |clean| clean.set_line_color(line_color),
                            ) {
                                color_memory.update(|memory| {
                                    memory.remember_line(line_color);
                                });
                                resume_hsv_movement_if_active();
                            }
                        }
                    >
                        <option value="solid">"Solid"</option>
                        <option value="gradient">"Gradient"</option>
                        <option value="hue_cycle">"Hue cycle"</option>
                        <option value="depth_gradient">"Depth gradient"</option>
                    </select>

                    <div
                        class="color-row"
                        class:hidden=move || {
                            !matches!(current_line_color(base_config), LineColorConfig::Solid { .. })
                        }
                    >
                        <label for="line-solid-color">"Line color"</label>
                        <input
                            id="line-solid-color"
                            type="color"
                            prop:value=move || {
                                match line_color_for_mode(
                                    base_config,
                                    color_memory,
                                    LineColorMode::Solid,
                                ) {
                                    LineColorConfig::Solid { color } => rgb_to_hex(color),
                                    _ => unreachable!("solid mode must provide solid color"),
                                }
                            }
                            on:input:target=move |ev| {
                                let Some(color) = hex_to_rgb(&ev.target().value()) else {
                                    error.set(Some("Invalid color value.".to_string()));
                                    return;
                                };
                                let line_color = LineColorConfig::Solid { color };
                                if update_clean_config(
                                    config_workspace,
                                    error,
                                    render_current_colors,
                                    "solid line color input",
                                    move |clean| clean.set_line_color(line_color),
                                ) {
                                    color_memory.update(|memory| {
                                        memory.remember_line(line_color);
                                    });
                                }
                            }
                        />
                    </div>

                    <div
                        class="color-row"
                        class:hidden=move || {
                            !matches!(
                                current_line_color(base_config),
                                LineColorConfig::DepthGradient { .. }
                            )
                        }
                    >
                        <label for="line-depth-gradient-start">"Depth start"</label>
                        <input
                            id="line-depth-gradient-start"
                            type="color"
                            prop:value=move || {
                                match line_color_for_mode(
                                    base_config,
                                    color_memory,
                                    LineColorMode::DepthGradient,
                                ) {
                                    LineColorConfig::DepthGradient { start, .. } => {
                                        rgb_to_hex(start)
                                    }
                                    _ => unreachable!(
                                        "depth-gradient mode must provide depth-gradient color"
                                    ),
                                }
                            }
                            on:input:target=move |ev| {
                                let Some(start) = hex_to_rgb(&ev.target().value()) else {
                                    error.set(Some("Invalid color value.".to_string()));
                                    return;
                                };
                                let current = line_color_for_mode_untracked(
                                    base_config,
                                    color_memory,
                                    LineColorMode::DepthGradient,
                                );
                                let end = match current {
                                    LineColorConfig::DepthGradient { end, .. } => end,
                                    _ => unreachable!(
                                        "depth-gradient mode must provide depth-gradient color"
                                    ),
                                };
                                let line_color = LineColorConfig::DepthGradient { start, end };
                                if update_clean_config(
                                    config_workspace,
                                    error,
                                    render_current_colors,
                                    "depth-gradient start color input",
                                    move |clean| clean.set_line_color(line_color),
                                ) {
                                    color_memory.update(|memory| {
                                        memory.remember_line(line_color);
                                    });
                                }
                            }
                        />

                        <label for="line-depth-gradient-end">"Depth end"</label>
                        <input
                            id="line-depth-gradient-end"
                            type="color"
                            prop:value=move || {
                                match line_color_for_mode(
                                    base_config,
                                    color_memory,
                                    LineColorMode::DepthGradient,
                                ) {
                                    LineColorConfig::DepthGradient { end, .. } => rgb_to_hex(end),
                                    _ => unreachable!(
                                        "depth-gradient mode must provide depth-gradient color"
                                    ),
                                }
                            }
                            on:input:target=move |ev| {
                                let Some(end) = hex_to_rgb(&ev.target().value()) else {
                                    error.set(Some("Invalid color value.".to_string()));
                                    return;
                                };
                                let current = line_color_for_mode_untracked(
                                    base_config,
                                    color_memory,
                                    LineColorMode::DepthGradient,
                                );
                                let start = match current {
                                    LineColorConfig::DepthGradient { start, .. } => start,
                                    _ => unreachable!(
                                        "depth-gradient mode must provide depth-gradient color"
                                    ),
                                };
                                let line_color = LineColorConfig::DepthGradient { start, end };
                                if update_clean_config(
                                    config_workspace,
                                    error,
                                    render_current_colors,
                                    "depth-gradient end color input",
                                    move |clean| clean.set_line_color(line_color),
                                ) {
                                    color_memory.update(|memory| {
                                        memory.remember_line(line_color);
                                    });
                                }
                            }
                        />
                    </div>

                    <div
                        class="color-row"
                        class:hidden=move || {
                            !matches!(
                                current_line_color(base_config),
                                LineColorConfig::Gradient { .. }
                            )
                        }
                    >
                        <label for="line-gradient-start">"Gradient start"</label>
                        <input
                            id="line-gradient-start"
                            type="color"
                            prop:value=move || {
                                match line_color_for_mode(
                                    base_config,
                                    color_memory,
                                    LineColorMode::Gradient,
                                ) {
                                    LineColorConfig::Gradient { start, .. } => rgb_to_hex(start),
                                    _ => unreachable!("gradient mode must provide gradient color"),
                                }
                            }
                            on:input:target=move |ev| {
                                let Some(start) = hex_to_rgb(&ev.target().value()) else {
                                    error.set(Some("Invalid color value.".to_string()));
                                    return;
                                };
                                let current = line_color_for_mode_untracked(
                                    base_config,
                                    color_memory,
                                    LineColorMode::Gradient,
                                );
                                let end = match current {
                                    LineColorConfig::Gradient { end, .. } => end,
                                    _ => unreachable!("gradient mode must provide gradient color"),
                                };
                                let line_color = LineColorConfig::Gradient { start, end };
                                if update_clean_config(
                                    config_workspace,
                                    error,
                                    render_current_colors,
                                    "gradient start color input",
                                    move |clean| clean.set_line_color(line_color),
                                ) {
                                    color_memory.update(|memory| {
                                        memory.remember_line(line_color);
                                    });
                                }
                            }
                        />

                        <label for="line-gradient-end">"Gradient end"</label>
                        <input
                            id="line-gradient-end"
                            type="color"
                            prop:value=move || {
                                match line_color_for_mode(
                                    base_config,
                                    color_memory,
                                    LineColorMode::Gradient,
                                ) {
                                    LineColorConfig::Gradient { end, .. } => rgb_to_hex(end),
                                    _ => unreachable!("gradient mode must provide gradient color"),
                                }
                            }
                            on:input:target=move |ev| {
                                let Some(end) = hex_to_rgb(&ev.target().value()) else {
                                    error.set(Some("Invalid color value.".to_string()));
                                    return;
                                };
                                let current = line_color_for_mode_untracked(
                                    base_config,
                                    color_memory,
                                    LineColorMode::Gradient,
                                );
                                let start = match current {
                                    LineColorConfig::Gradient { start, .. } => start,
                                    _ => unreachable!("gradient mode must provide gradient color"),
                                };
                                let line_color = LineColorConfig::Gradient { start, end };
                                if update_clean_config(
                                    config_workspace,
                                    error,
                                    render_current_colors,
                                    "gradient end color input",
                                    move |clean| clean.set_line_color(line_color),
                                ) {
                                    color_memory.update(|memory| {
                                        memory.remember_line(line_color);
                                    });
                                }
                            }
                        />
                    </div>

                    <div
                        class="color-row"
                        class:hidden=move || {
                            !matches!(
                                current_line_color(base_config),
                                LineColorConfig::HueCycle { .. }
                            )
                        }
                    >
                        <label for="line-hue-cycle-initial">"Initial color"</label>
                        <input
                            id="line-hue-cycle-initial"
                            type="color"
                            prop:value=move || {
                                match line_color_for_mode(
                                    base_config,
                                    color_memory,
                                    LineColorMode::HueCycle,
                                ) {
                                    LineColorConfig::HueCycle { initial } => rgb_to_hex(initial),
                                    _ => unreachable!("hue-cycle mode must provide hue-cycle color"),
                                }
                            }
                            on:input:target=move |ev| {
                                let Some(initial) = hex_to_rgb(&ev.target().value()) else {
                                    error.set(Some("Invalid color value.".to_string()));
                                    return;
                                };
                                let line_color = LineColorConfig::HueCycle { initial };
                                if update_clean_config(
                                    config_workspace,
                                    error,
                                    render_current_colors,
                                    "hue-cycle initial color input",
                                    move |clean| clean.set_line_color(line_color),
                                ) {
                                    color_memory.update(|memory| {
                                        memory.remember_line(line_color);
                                    });
                                }
                            }
                        />
                    </div>

                    <div
                        class:hidden=move || {
                            !matches!(
                                current_line_color(base_config),
                                LineColorConfig::HueCycle { .. }
                            )
                        }
                    >
                        <button type="button" on:click=toggle_hsv_movement>
                            {move || {
                                if hsv_movement.get() {
                                    "HSV movement: On"
                                } else {
                                    "HSV movement: Off"
                                }
                            }}
                        </button>
                        <label for="hsv-movement-direction">"Direction"</label>
                        <select
                            id="hsv-movement-direction"
                            prop:value=move || hsv_movement_direction.get().key().to_string()
                            on:change:target=move |ev| {
                                let key = ev.target().value();
                                let Some(direction) = HsvMovementDirection::from_key(&key) else {
                                    log::error!("unknown HSV movement direction selected: {key}");
                                    return;
                                };
                                hsv_movement_direction.set(direction);
                            }
                        >
                            <option value="forward">{HsvMovementDirection::Forward.label()}</option>
                            <option value="reverse">{HsvMovementDirection::Reverse.label()}</option>
                        </select>
                        <label for="hsv-movement-speed">"Movement speed (°/s)"</label>
                        <div class="row">
                            <input
                                id="hsv-movement-speed"
                                type="range"
                                min=HSV_MOVEMENT_MIN_SPEED_DEGREES_PER_SECOND.to_string()
                                max=HSV_MOVEMENT_MAX_SPEED_DEGREES_PER_SECOND.to_string()
                                step="1"
                                prop:value=move || hsv_movement_speed.get().to_string()
                                on:input:target=move |ev| {
                                    let next = ev
                                        .target()
                                        .value()
                                        .parse::<f32>()
                                        .unwrap_or(HSV_MOVEMENT_DEFAULT_SPEED_DEGREES_PER_SECOND);
                                    hsv_movement_speed.set(next.clamp(
                                        HSV_MOVEMENT_MIN_SPEED_DEGREES_PER_SECOND,
                                        HSV_MOVEMENT_MAX_SPEED_DEGREES_PER_SECOND,
                                    ));
                                }
                            />
                            <output>{move || format!("{:.0}", hsv_movement_speed.get())}</output>
                        </div>
                    </div>

                    <label for="png-width">"PNG width"</label>
                    <input
                        id="png-width"
                        type="number"
                        min="256"
                        max="4096"
                        step="16"
                        prop:value=move || png_width.get().to_string()
                        on:input:target=move |ev| {
                            let next = ev.target().value().parse::<u32>().unwrap_or(2048);
                            png_width.set(next.clamp(256, 4096));
                        }
                    />

                    <div class="export-row">
                        <button
                            type="button"
                            class:hidden=move || is_3d()
                            on:click=move |_| {
                                let mut config = base_config.get_untracked();
                                config.generation.iterations = iterations.get_untracked();
                                config.generation.angle = angle.get_untracked();
                                export_svg(config);
                            }
                        >
                            "Export SVG"
                        </button>
                        <button
                            type="button"
                            on:click=move |_| {
                                let Some(Some((device, queue, camera))) =
                                    renderer.try_with_value(|opt| {
                                        opt.as_ref().map(|r| {
                                            let (d, q) = r.device_queue();
                                            (d, q, r.camera())
                                        })
                                    })
                                else {
                                    log::error!("PNG export: renderer not initialized");
                                    error.set(Some(
                                        "Cannot export PNG: GPU renderer is not ready yet."
                                            .to_string(),
                                    ));
                                    return;
                                };
                                let mut config = base_config.get_untracked();
                                config.generation.iterations = iterations.get_untracked();
                                config.generation.angle = angle.get_untracked();
                                export_png(
                                    device,
                                    queue,
                                    camera,
                                    config,
                                    png_width.get_untracked(),
                                    move |e| error.set(Some(e)),
                                );
                            }
                        >
                            "Export PNG"
                        </button>
                    </div>

                    <div class:hidden=move || !is_3d()>
                        <button type="button" on:click=toggle_auto_rotate>
                            {move || {
                                if auto_rotate.get() {
                                    "Auto-rotate: On"
                                } else {
                                    "Auto-rotate: Off"
                                }
                            }}
                        </button>
                        <label for="auto-rotate-speed">"Speed (°/s)"</label>
                        <div class="row">
                            <input
                                id="auto-rotate-speed"
                                type="range"
                                min="10"
                                max="360"
                                step="10"
                                prop:value=move || auto_rotate_speed.get().to_string()
                                on:input:target=move |ev| {
                                    let next =
                                        ev.target().value().parse::<f32>().unwrap_or(45.0);
                                    auto_rotate_speed.set(next.clamp(10.0, 360.0));
                                }
                            />
                            <output>{move || format!("{:.0}", auto_rotate_speed.get())}</output>
                        </div>
                    </div>
                </div>
            </aside>

            <section class="viewport">
                <canvas
                    node_ref=canvas_ref
                    class="fractal-canvas"
                    tabindex="0"
                    on:pointerdown=move |ev: web_sys::PointerEvent| {
                        if let Some(canvas) = canvas_ref.get_untracked() {
                            let _ = canvas.focus();
                            let _ = canvas.set_pointer_capture(ev.pointer_id());
                        }
                        last_pointer.update_value(|p| {
                            *p = Some((
                                ev.pointer_id(),
                                ev.client_x() as f64,
                                ev.client_y() as f64,
                            ));
                        });
                    }
                    on:pointermove=move |ev: web_sys::PointerEvent| {
                        let Some((id, last_x, last_y)) = last_pointer.with_value(|p| *p) else {
                            return;
                        };
                        if id != ev.pointer_id() {
                            return;
                        }
                        let x = ev.client_x() as f64;
                        let y = ev.client_y() as f64;
                        let dx = x - last_x;
                        let dy = y - last_y;
                        last_pointer.update_value(|p| *p = Some((id, x, y)));
                        if let Some(canvas) = canvas_ref.get_untracked() {
                            with_renderer(
                                canvas,
                                renderer,
                                recover_after_render,
                                |r, c| r.drag_and_render(c, dx as f32, dy as f32),
                            );
                        }
                    }
                    on:pointerup=move |ev: web_sys::PointerEvent| {
                        if let Some(canvas) = canvas_ref.get_untracked() {
                            let _ = canvas.release_pointer_capture(ev.pointer_id());
                        }
                        last_pointer.update_value(|p| *p = None);
                    }
                    on:pointercancel=move |_| last_pointer.update_value(|p| *p = None)
                    on:wheel=move |ev: web_sys::WheelEvent| {
                        ev.prevent_default();
                        if let Some(canvas) = canvas_ref.get_untracked() {
                            with_renderer(
                                canvas,
                                renderer,
                                recover_after_render,
                                |r, c| {
                                    r.zoom_and_render(
                                        c,
                                        ev.delta_y() as f32,
                                        ev.delta_mode(),
                                        ev.client_x() as f32,
                                        ev.client_y() as f32,
                                    )
                                },
                            );
                        }
                    }
                    on:keydown=move |ev: web_sys::KeyboardEvent| {
                        let Some(canvas) = canvas_ref.get_untracked() else {
                            return;
                        };
                        let key = ev.key();
                        if key.eq_ignore_ascii_case("f") {
                            with_renderer(
                                canvas,
                                renderer,
                                recover_after_render,
                                |r, c| r.reset_and_render(c),
                            );
                        } else if is_3d_untracked() {
                            let handled = match key.as_str() {
                                "ArrowLeft" => {
                                    with_renderer(canvas, renderer, recover_after_render, |r, c| {
                                        r.orbit_and_render(c, -ROTATION_STEP_DEG, 0.0)
                                    });
                                    true
                                }
                                "ArrowRight" => {
                                    with_renderer(canvas, renderer, recover_after_render, |r, c| {
                                        r.orbit_and_render(c, ROTATION_STEP_DEG, 0.0)
                                    });
                                    true
                                }
                                "ArrowUp" => {
                                    with_renderer(canvas, renderer, recover_after_render, |r, c| {
                                        r.orbit_and_render(c, 0.0, ROTATION_STEP_DEG)
                                    });
                                    true
                                }
                                "ArrowDown" => {
                                    with_renderer(canvas, renderer, recover_after_render, |r, c| {
                                        r.orbit_and_render(c, 0.0, -ROTATION_STEP_DEG)
                                    });
                                    true
                                }
                                "q" | "Q" => {
                                    with_renderer(canvas, renderer, recover_after_render, |r, c| {
                                        r.roll_and_render(c, -ROTATION_STEP_DEG)
                                    });
                                    true
                                }
                                "e" | "E" => {
                                    with_renderer(canvas, renderer, recover_after_render, |r, c| {
                                        r.roll_and_render(c, ROTATION_STEP_DEG)
                                    });
                                    true
                                }
                                _ => false,
                            };
                            if handled {
                                ev.prevent_default();
                            }
                        }
                    }
                />
                <div class:hidden=move || gpu_error.get().is_none() class="unsupported">
                    <div>
                        <h2>"GPU rendering is not available in this browser."</h2>
                        <p>
                            {move || {
                                gpu_error
                                    .get()
                                    .unwrap_or_else(|| {
                                        "Try a browser with WebGPU or WebGL2 enabled.".to_string()
                                    })
                            }}
                        </p>
                    </div>
                </div>
            </section>
        </main>
    }
}

fn update_clean_config(
    config_workspace: RwSignal<ConfigWorkspace>,
    error: RwSignal<Option<String>>,
    render_after_update: impl Fn(),
    event: &'static str,
    update: impl FnOnce(&mut CleanMut<'_>) -> Result<(), ConfigError>,
) -> bool {
    let result = config_workspace.try_update(|workspace| {
        let entry = workspace.selected_mut();
        match entry.view_mut() {
            EntryViewMut::Clean(mut clean) => {
                update(&mut clean).map_err(|e| e.to_string())?;
                Ok(true)
            }
            EntryViewMut::Dirty(_) => {
                log::error!("{event} fired while entry is dirty; UI guards bypassed");
                Ok(false)
            }
        }
    });
    match result {
        Some(Ok(true)) => {
            error.set(None);
            render_after_update();
            true
        }
        Some(Ok(false)) => false,
        Some(Err(msg)) => {
            error.set(Some(msg));
            false
        }
        None => {
            log::error!("{event}: config_workspace signal was unavailable");
            error.set(Some("Internal error: could not update config.".to_string()));
            false
        }
    }
}

fn line_color_of(config: &Config) -> LineColorConfig {
    config.colors.line
}

fn current_line_color(base_config: Memo<Config>) -> LineColorConfig {
    base_config.with(|c| c.colors.line)
}

fn line_color_for_mode(
    base_config: Memo<Config>,
    memory: RwSignal<ColorControlMemory>,
    mode: LineColorMode,
) -> LineColorConfig {
    let current = current_line_color(base_config);
    if LineColorMode::from_line_color(&current) == mode {
        current
    } else {
        memory.with(|m| m.line_for(mode))
    }
}

fn line_color_for_mode_untracked(
    base_config: Memo<Config>,
    memory: RwSignal<ColorControlMemory>,
    mode: LineColorMode,
) -> LineColorConfig {
    let current = base_config.with_untracked(|c| c.colors.line);
    if LineColorMode::from_line_color(&current) == mode {
        current
    } else {
        memory.with_untracked(|m| m.line_for(mode))
    }
}

fn with_renderer<F, H>(
    canvas: web_sys::HtmlCanvasElement,
    renderer: RendererState,
    recover_after_render: H,
    render: F,
) where
    F: FnOnce(&mut CanvasRenderer, &web_sys::HtmlCanvasElement) -> RenderStatus,
    H: Fn(RenderStatus, web_sys::HtmlCanvasElement),
{
    let status = renderer.try_update_value(|opt| opt.as_mut().map(|r| render(r, &canvas)));
    if let Some(Some(status)) = status {
        recover_after_render(status, canvas);
    }
}

fn install_resize_listener<H>(
    canvas_ref: NodeRef<Canvas>,
    renderer: RendererState,
    recover_after_render: H,
) where
    H: Fn(RenderStatus, web_sys::HtmlCanvasElement) + Copy + 'static,
{
    let Some(window) = web_sys::window() else {
        log::error!("Failed to install resize listener: window unavailable");
        return;
    };
    let closure = Closure::<dyn FnMut()>::new(move || {
        if let Some(canvas) = canvas_ref.get_untracked() {
            with_renderer(canvas, renderer, recover_after_render, |r, c| r.render(c));
        }
    });
    match window.add_event_listener_with_callback("resize", closure.as_ref().unchecked_ref()) {
        Ok(()) => closure.forget(),
        Err(err) => log::error!("Failed to install resize listener: {err:?}"),
    }
}

async fn next_animation_frame() -> Result<f64, &'static str> {
    let mut resolve_fn: Option<js_sys::Function> = None;
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        resolve_fn = Some(resolve);
    });
    let resolve_fn = resolve_fn.ok_or("Promise constructor did not run synchronously")?;
    // once_into_js transfers ownership to the JS GC — no forget() needed.
    // The callback receives the DOMHighResTimeStamp from rAF and resolves the
    // Promise with it so the caller gets the actual frame timestamp.
    // call1 failure would leave the Promise unresolved; in practice a JS
    // resolve function never throws, so the result is intentionally ignored.
    let cb = Closure::once_into_js(move |ts: f64| {
        let _ = resolve_fn.call1(
            &wasm_bindgen::JsValue::UNDEFINED,
            &wasm_bindgen::JsValue::from_f64(ts),
        );
    });
    web_sys::window()
        .ok_or("window unavailable")?
        .request_animation_frame(cb.unchecked_ref())
        .map_err(|_| "request_animation_frame rejected")?;
    wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .ok()
        .and_then(|v| v.as_f64())
        .ok_or("animation frame timestamp was not a number")
}

#[cfg(test)]
mod tests {
    use lsystem_core::{ColorConfig, LineColorConfig};

    use super::{
        ColorControlMemory, HsvMovementDirection, LineColorMode, advance_hsv_phase_degrees,
        hsv_movement_is_active,
    };

    #[test]
    fn line_color_mode_key_round_trip() {
        for mode in [
            LineColorMode::Solid,
            LineColorMode::Gradient,
            LineColorMode::HueCycle,
            LineColorMode::DepthGradient,
        ] {
            assert_eq!(LineColorMode::from_key(mode.key()), Some(mode));
        }
        assert_eq!(LineColorMode::from_key("unknown"), None);
    }

    #[test]
    fn hsv_movement_direction_key_and_label_round_trip() {
        for direction in [HsvMovementDirection::Forward, HsvMovementDirection::Reverse] {
            assert_eq!(
                HsvMovementDirection::from_key(direction.key()),
                Some(direction)
            );
            assert!(!direction.label().is_empty());
        }
        assert_eq!(HsvMovementDirection::from_key("sideways"), None);
    }

    #[test]
    fn hsv_movement_phase_advances_by_speed_direction_and_wraps() {
        assert_eq!(
            advance_hsv_phase_degrees(350.0, 20.0, 1.0, HsvMovementDirection::Forward),
            10.0
        );
        assert_eq!(
            advance_hsv_phase_degrees(10.0, 20.0, 1.0, HsvMovementDirection::Reverse),
            350.0
        );
    }

    #[test]
    fn hsv_movement_is_only_active_for_hue_cycle_line_color() {
        assert!(hsv_movement_is_active(
            true,
            &LineColorConfig::DEFAULT_HUE_CYCLE
        ));
        assert!(!hsv_movement_is_active(
            true,
            &LineColorConfig::DEFAULT_SOLID
        ));
        assert!(!hsv_movement_is_active(
            false,
            &LineColorConfig::DEFAULT_HUE_CYCLE
        ));
    }

    #[test]
    fn line_color_mode_from_line_color() {
        assert_eq!(
            LineColorMode::from_line_color(&LineColorConfig::DEFAULT_SOLID),
            LineColorMode::Solid
        );
        assert_eq!(
            LineColorMode::from_line_color(&LineColorConfig::DEFAULT_GRADIENT),
            LineColorMode::Gradient
        );
        assert_eq!(
            LineColorMode::from_line_color(&LineColorConfig::DEFAULT_HUE_CYCLE),
            LineColorMode::HueCycle
        );
        assert_eq!(
            LineColorMode::from_line_color(&LineColorConfig::DEFAULT_DEPTH_GRADIENT),
            LineColorMode::DepthGradient
        );
    }

    #[test]
    fn color_memory_remembers_solid_across_mode_switch() {
        let solid = LineColorConfig::Solid {
            color: [1.0, 0.0, 0.0],
        };
        let colors = ColorConfig {
            background: None,
            line: solid,
        };
        let mut memory = ColorControlMemory::from_colors(&colors);

        // switch to gradient — solid is remembered
        memory.remember_line(LineColorConfig::DEFAULT_GRADIENT);
        assert_eq!(memory.line_for(LineColorMode::Solid), solid);
        // switch back — gradient default is returned, solid still intact
        assert_eq!(memory.line_for(LineColorMode::Solid), solid);
    }

    #[test]
    fn color_memory_falls_back_to_default_when_slot_unset() {
        let colors = ColorConfig {
            background: None,
            line: LineColorConfig::DEFAULT_SOLID,
        };
        let memory = ColorControlMemory::from_colors(&colors);
        // gradient and hue_cycle slots were never set
        assert_eq!(
            memory.line_for(LineColorMode::Gradient),
            LineColorMode::Gradient.default_line_color()
        );
        assert_eq!(
            memory.line_for(LineColorMode::HueCycle),
            LineColorMode::HueCycle.default_line_color()
        );
        assert_eq!(
            memory.line_for(LineColorMode::DepthGradient),
            LineColorMode::DepthGradient.default_line_color()
        );
    }

    #[test]
    fn color_memory_background_remembered() {
        let bg = [0.1, 0.2, 0.3];
        let colors = ColorConfig {
            background: Some(bg),
            line: LineColorConfig::DEFAULT_SOLID,
        };
        let mut memory = ColorControlMemory::from_colors(&colors);
        assert_eq!(memory.background(), bg);

        let new_bg = [0.9, 0.8, 0.7];
        memory.remember_background(new_bg);
        assert_eq!(memory.background(), new_bg);
    }
}
