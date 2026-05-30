use leptos::html::Canvas;
use leptos::prelude::*;
use lsystem_core::{
    CleanMut, ColorConfig, ConfigError, ConfigWorkspace, Dimensions, EntryViewMut,
    GenerationConfig, LineColorConfig,
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

fn contains_3d_symbols(s: &str) -> bool {
    s.contains(['&', '^', '/', '\\'])
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
    let color_memory = RwSignal::new(ColorControlMemory::from_colors(
        &initial_workspace.selected().applied_config().colors,
    ));
    let config_workspace = RwSignal::new(initial_workspace);
    let error = RwSignal::new(None::<String>);
    let png_width = RwSignal::new(2048u32);
    let gpu_error = RwSignal::new(None::<String>);
    let auto_rotate = RwSignal::new(true);
    let auto_rotate_speed = RwSignal::new(20.0f32);
    let hsv_movement = RwSignal::new(false);
    let hsv_movement_speed = RwSignal::new(HSV_MOVEMENT_DEFAULT_SPEED_DEGREES_PER_SECOND);
    let hsv_movement_direction = RwSignal::new(HsvMovementDirection::Forward);
    let hsv_movement_phase = RwSignal::new(0.0f32);
    let sheet_open = RwSignal::new(false);
    let sheet_drag_start: StoredValue<Option<f64>, LocalStorage> = StoredValue::new_local(None);
    // Generation counter: bumped on each animation start so older rAF loops detect
    // they have been superseded and exit.
    let animation_token = RwSignal::new(0u32);
    on_cleanup(move || animation_token.update(|t| *t = t.wrapping_add(1)));

    let toml_text =
        Memo::new(move |_| config_workspace.with(|ws| ws.selected().draft_text().into_owned()));
    let max_iterations = Memo::new(move |_| {
        config_workspace
            .with(|ws| max_iterations_for_config(&ws.selected().applied_config().generation))
    });
    let generation_config = Memo::new(move |_| {
        let max = max_iterations.get();
        config_workspace.with(|ws| {
            let mut generation = ws.selected().applied_config().generation.clone();
            generation.iterations = generation.iterations.min(max);
            generation
        })
    });
    let color_config = Memo::new(move |_| {
        config_workspace.with(|ws| ws.selected().applied_config().colors.clone())
    });
    let iterations = Memo::new(move |_| generation_config.with(|g| g.iterations));
    let angle = Memo::new(move |_| generation_config.with(|g| g.angle));

    let renderer: RendererState = StoredValue::new_local(None::<CanvasRenderer>);
    let last_pointer = StoredValue::new(None::<(i32, f64, f64)>);

    let is_3d = move || matches!(generation_config.get().dimensions, Dimensions::ThreeD);
    let is_dirty = move || config_workspace.with(|workspace| workspace.selected().is_dirty());
    let dirty_tooltip = move || {
        if is_dirty() {
            "Apply or Revert TOML changes first"
        } else {
            ""
        }
    };

    let rename_mode = RwSignal::new(false);
    let rename_draft = RwSignal::new(String::new());
    let grammar_axiom = RwSignal::new(generation_config.get_untracked().axiom.clone());
    let grammar_rules: RwSignal<Vec<(String, String)>> = RwSignal::new(rules_to_editor_rows(
        &generation_config.get_untracked().rules,
    ));

    let dims_3d_error: RwSignal<Option<String>> = RwSignal::new(None);

    let save_format = RwSignal::new("png"); // "svg" | "png"

    let sync_grammar_editor = move || {
        let generation = generation_config.get_untracked();
        grammar_axiom.set(generation.axiom);
        grammar_rules.set(rules_to_editor_rows(&generation.rules));
    };

    let canvas_ref = NodeRef::<Canvas>::new();

    let config_for_render = move || {
        let mut config =
            config_workspace.with_untracked(|ws| ws.selected().applied_config().clone());
        config.generation = generation_config.get_untracked();
        config
    };

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
                            let config = config_for_render();
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

    let animation_active = Memo::new(move |_| {
        (auto_rotate.get() && is_3d())
            || hsv_movement_is_active(hsv_movement.get(), &color_config.with(|c| c.line))
    });

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

                if !animation_active.get_untracked() {
                    break;
                }
                let auto_active = auto_rotate.get_untracked()
                    && matches!(
                        generation_config.get_untracked().dimensions,
                        Dimensions::ThreeD
                    );
                let line_color = color_config.with_untracked(|c| c.line);
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

    canvas_ref.on_load(move |canvas| {
        wasm_bindgen_futures::spawn_local(async move {
            match CanvasRenderer::new(canvas.clone()).await {
                Ok(new_renderer) => {
                    gpu_error.set(None);
                    renderer.update_value(|opt| *opt = Some(new_renderer));
                    let config = config_for_render();
                    with_renderer(canvas, renderer, recover_after_render, |r, c| {
                        r.set_config_and_render(c, &config)
                    });
                    if animation_active.get_untracked() {
                        start_animation_loop();
                    }
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
            &color_config.get_untracked(),
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

    Effect::new(move |prev: Option<(GenerationConfig, ColorConfig)>| {
        let generation = generation_config.get();
        let color = color_config.get();

        if let Some((prev_generation, prev_color)) = &prev
            && (prev_generation != &generation || prev_color != &color)
            && let Some(canvas) = canvas_ref.get_untracked()
        {
            let config = config_for_render();
            with_renderer(canvas, renderer, recover_after_render, |r, c| {
                if prev_generation != &generation {
                    r.set_config_and_render(c, &config)
                } else {
                    r.set_colors_and_render(c, &config)
                }
            });
        }

        (generation, color)
    });

    // Start the animation loop only on a false->true transition. The initial run
    // (prev == None) is intentionally skipped because on_load already starts the
    // loop when animation is active on mount.
    Effect::new(move |prev: Option<bool>| {
        let active = animation_active.get();
        if active && prev == Some(false) {
            start_animation_loop();
        }
        active
    });

    let select_current_config = move || {
        error.set(None);
        refresh_color_memory();
        sync_grammar_editor();
    };

    let apply_current = move || {
        if !config_workspace.with_untracked(|ws| ws.selected().is_dirty()) {
            return;
        }
        let result = config_workspace
            .try_update(|workspace| workspace.apply().map(|_| ()).map_err(|e| e.to_string()));
        match result {
            Some(Ok(())) => select_current_config(),
            Some(Err(msg)) => error.set(Some(msg)),
            None => {
                log::error!("apply: config_workspace signal was unavailable");
                error.set(Some("Internal error: could not apply config.".to_string()));
            }
        }
    };

    let do_revert = move || {
        let result =
            config_workspace.try_update(|workspace| match workspace.selected_mut().view_mut() {
                EntryViewMut::Dirty(dirty) => {
                    dirty.revert();
                    true
                }
                EntryViewMut::Clean(_) => {
                    log::error!("revert fired while entry is clean; UI guards bypassed");
                    false
                }
            });
        match result {
            Some(true) => select_current_config(),
            Some(false) => {}
            None => {
                log::error!("revert: config_workspace signal was unavailable");
                error.set(Some("Internal error: could not revert config.".to_string()));
            }
        }
    };

    let commit_rename = move || {
        let name = rename_draft.get_untracked().trim().to_string();
        let idx = config_workspace.with_untracked(|ws| ws.selected_index());
        match config_workspace.try_update(|ws| ws.rename(idx, &name)) {
            Some(Ok(())) => {
                error.set(None);
                rename_mode.set(false);
            }
            Some(Err(e)) => error.set(Some(e.to_string())),
            None => error.set(Some("Internal error: could not rename.".to_string())),
        }
    };

    let do_reset = move || {
        let result = config_workspace.try_update(|ws| {
            ws.reset()
                .map(|opt| opt.is_some())
                .map_err(|e| e.to_string())
        });
        match result {
            Some(Ok(true)) => select_current_config(),
            Some(Ok(false)) => {
                log::warn!(
                    "do_reset: no-op for entry without a bundled default; button guard may have been bypassed"
                );
            }
            Some(Err(msg)) => error.set(Some(msg)),
            None => error.set(Some("Internal error: could not reset.".to_string())),
        }
    };

    let grammar_has_3d_symbols = move || {
        contains_3d_symbols(&grammar_axiom.get())
            || grammar_rules
                .get()
                .iter()
                .any(|(_, rhs)| contains_3d_symbols(rhs))
    };

    let grammar_is_dirty = move || {
        let generation = generation_config.get();
        let applied_rules = rules_to_editor_rows(&generation.rules);
        grammar_axiom.get() != generation.axiom || grammar_rules.get() != applied_rules
    };
    let grammar_dirty_tooltip = move || {
        if grammar_is_dirty() {
            "Apply or Revert grammar changes first"
        } else {
            ""
        }
    };

    let do_apply_grammar = move || {
        let axiom = grammar_axiom.get_untracked();
        let rules_raw = grammar_rules.get_untracked();
        let mut seen = std::collections::HashSet::new();
        let mut rules: Vec<(char, String)> = Vec::with_capacity(rules_raw.len());
        for (k, v) in rules_raw {
            let Some(c) = k.chars().next() else {
                error.set(Some(
                    "Each rule must have a symbol. Remove or complete empty rows before applying."
                        .to_string(),
                ));
                return;
            };
            if !seen.insert(c) {
                error.set(Some(format!(
                    "Duplicate rule symbol '{c}'. Each symbol may appear only once."
                )));
                return;
            }
            rules.push((c, v));
        }
        if update_clean_config(config_workspace, error, "grammar apply", move |clean| {
            clean.set_grammar(&axiom, &rules)
        }) {
            sync_grammar_editor();
        }
    };

    // Grammar Apply is enabled only when grammar has changes, the entry is clean, and
    // no 3D-only symbols are present in 2D mode.
    let grammar_can_apply =
        move || grammar_is_dirty() && (is_3d() || !grammar_has_3d_symbols()) && !is_dirty();

    let try_apply_grammar = move || {
        // The Apply button is disabled when grammar has 3D symbols in 2D mode;
        // this guard is a defensive fallback.
        if grammar_has_3d_symbols()
            && !matches!(
                generation_config.get_untracked().dimensions,
                Dimensions::ThreeD
            )
        {
            return;
        }
        do_apply_grammar();
    };

    let try_set_dimensions = move |key: &'static str| {
        let next = if key == "3d" {
            Dimensions::ThreeD
        } else {
            Dimensions::TwoD
        };
        if next == Dimensions::TwoD {
            let generation = generation_config.get_untracked();
            if contains_3d_symbols(&generation.axiom)
                || generation
                    .rules
                    .values()
                    .any(|rhs| contains_3d_symbols(rhs))
            {
                dims_3d_error.set(Some(
                    "Remove 3D-only symbols (& ^ / \\) from the grammar before switching to 2D."
                        .to_string(),
                ));
                return;
            }
        }
        dims_3d_error.set(None);
        update_clean_config(config_workspace, error, "set dimensions", move |clean| {
            clean.set_dimensions(next)
        });
    };

    let apply_hue_movement = move |dir: Option<HsvMovementDirection>| match dir {
        None => reset_hsv_movement(),
        Some(d) => {
            hsv_movement_direction.set(d);
            hsv_movement.set(true);
        }
    };

    let try_set_hue_movement = move |key: &'static str| {
        let dir = match key {
            "forward" => Some(HsvMovementDirection::Forward),
            "backward" => Some(HsvMovementDirection::Reverse),
            _ => None,
        };
        // Forward/Backward buttons are disabled when not in Hue-cycle mode;
        // this guard is a defensive fallback in case disabled_keys is miscalculated.
        if dir.is_some()
            && !matches!(
                color_config.with_untracked(|c| c.line),
                LineColorConfig::HueCycle { .. }
            )
        {
            log::error!(
                "try_set_hue_movement: direction set while not in HueCycle mode; \
                 disabled_keys guard may have been bypassed"
            );
            return;
        }
        apply_hue_movement(dir);
    };

    view! {
        <main
            class="app-shell"
        >
            <aside
                class="controls"
                class:sheet-open=move || sheet_open.get()
            >
                <div
                    class="sheet-handle-area"
                    on:pointerdown=move |ev: web_sys::PointerEvent| {
                        let target: web_sys::Element = ev.target().unwrap().unchecked_into();
                        let _ = target.set_pointer_capture(ev.pointer_id());
                        sheet_drag_start.set_value(Some(ev.client_y() as f64));
                    }
                    on:pointermove=move |ev: web_sys::PointerEvent| {
                        let Some(start) = sheet_drag_start.get_value() else { return; };
                        let dy = ev.client_y() as f64 - start;
                        if dy < -30.0 {
                            sheet_open.set(true);
                            sheet_drag_start.set_value(None);
                        } else if dy > 30.0 {
                            sheet_open.set(false);
                            sheet_drag_start.set_value(None);
                        }
                    }
                    on:pointerup=move |_| { sheet_drag_start.set_value(None); }
                    on:click=move |_| sheet_open.update(|v| *v = !*v)
                >
                    <div class="sheet-handle"></div>
                    <span class="sheet-preset-name">
                        {move || config_workspace.with(|ws| ws.selected().name().to_string())}
                    </span>
                </div>

                <div class="controls-scroll">
                <div class="preset-row">
                    <select
                        class:hidden=move || rename_mode.get()
                        prop:value=move || config_workspace.with(|ws| ws.selected_index().to_string())
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

                    <input
                        type="text"
                        class="rename-input"
                        class:hidden=move || !rename_mode.get()
                        prop:value=move || rename_draft.get()
                        on:input:target=move |ev| rename_draft.set(ev.target().value())
                        on:keydown=move |ev: web_sys::KeyboardEvent| {
                            if ev.key() == "Enter" { commit_rename(); }
                            if ev.key() == "Escape" { rename_mode.set(false); }
                        }
                    />
                </div>

                <div class="preset-actions">
                    {move || if rename_mode.get() {
                        view! {
                            <div style="display:contents">
                                <button
                                    type="button"
                                    disabled=move || {
                                        let d = rename_draft.get();
                                        d.trim().is_empty()
                                            || config_workspace.with(|ws| {
                                                ws.index_by_name(d.trim())
                                                    .is_some_and(|i| i != ws.selected_index())
                                            })
                                    }
                                    on:click=move |_| commit_rename()
                                >"Save"</button>
                                <button type="button" on:click=move |_| rename_mode.set(false)>"✕"</button>
                            </div>
                        }.into_any()
                    } else {
                        view! {
                            <div style="display:contents">
                                <button
                                    type="button"
                                    on:click=move |_| {
                                        let result = config_workspace.try_update(|workspace| {
                                            workspace.copy().map(|_| ()).map_err(|e| e.to_string())
                                        });
                                        match result {
                                            Some(Ok(())) => select_current_config(),
                                            Some(Err(msg)) => error.set(Some(msg)),
                                            None => {
                                                log::error!("copy: config_workspace signal was unavailable");
                                                error.set(Some(
                                                    "Internal error: could not copy config.".to_string(),
                                                ));
                                            }
                                        }
                                    }
                                >"Copy"</button>
                                <button
                                    type="button"
                                    on:click=move |_| {
                                        rename_draft.set(
                                            config_workspace.with(|ws| ws.selected().name().to_string())
                                        );
                                        rename_mode.set(true);
                                    }
                                >"Rename"</button>
                                <button
                                    type="button"
                                    disabled=move || config_workspace.with(|ws| !ws.can_reset())
                                    on:click=move |_| do_reset()
                                >"Reset"</button>
                            </div>
                        }.into_any()
                    }}
                </div>

                <crate::ui::Disclosure title="Edit TOML" open=false
                    badge=Signal::derive(is_dirty)>
                    <div title=grammar_dirty_tooltip>
                    <textarea
                        id="config"
                        spellcheck="false"
                        prop:value=move || toml_text.get()
                        disabled=grammar_is_dirty
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
                    </div>
                    <div title=grammar_dirty_tooltip>
                    <div class="btn-row">
                        <button
                            type="button"
                            disabled=move || !is_dirty() || grammar_is_dirty()
                            on:click=move |_| apply_current()
                        >
                            "Apply"
                        </button>
                        <button
                            type="button"
                            disabled=move || !is_dirty() || grammar_is_dirty()
                            on:click=move |_| do_revert()
                        >
                            "Revert"
                        </button>
                    </div>
                    </div>
                </crate::ui::Disclosure>

                <crate::ui::Disclosure title="Grammar" badge=Signal::derive(grammar_is_dirty)>
                    <div title=dirty_tooltip>
                    <table class="grammar-table">
                        <tbody>
                            <tr>
                                <td class="g-symbol">
                                    <span class="grammar-axiom-label">"axiom"</span>
                                </td>
                                <td>
                                    <input
                                        type="text"
                                        class="grammar-rhs"
                                        prop:value=move || grammar_axiom.get()
                                        disabled=is_dirty
                                        on:input:target=move |ev| grammar_axiom.set(ev.target().value())
                                    />
                                </td>
                                <td class="g-delete"></td>
                            </tr>
                            {move || {
                                let rules = grammar_rules.get();
                                let axiom = grammar_axiom.get();
                                let all_syms: Vec<char> = axiom
                                    .chars()
                                    .chain(rules.iter().flat_map(|(k, _)| k.chars()))
                                    .filter(|c| c.is_ascii_alphabetic())
                                    .collect::<std::collections::BTreeSet<_>>()
                                    .into_iter()
                                    .collect();
                                rules.into_iter().enumerate().map(|(idx, (sym, rhs))| {
                                    let list_id = format!("sym-dl-{idx}");
                                    view! {
                                        <tr>
                                            <td class="g-symbol">
                                                <input
                                                    type="text"
                                                    class="grammar-combo"
                                                    list=list_id.clone()
                                                    maxlength="1"
                                                    prop:value=sym.clone()
                                                    disabled=is_dirty
                                                    on:input:target=move |ev| {
                                                        grammar_rules.update(|rules| {
                                                            if let Some(r) = rules.get_mut(idx) {
                                                                r.0 = ev.target().value();
                                                            }
                                                        });
                                                    }
                                                />
                                                <datalist id=list_id>
                                                    {all_syms.iter().map(|c| {
                                                        let s = c.to_string();
                                                        view! { <option value=s /> }
                                                    }).collect_view()}
                                                </datalist>
                                            </td>
                                            <td>
                                                <input
                                                    type="text"
                                                    class="grammar-rhs"
                                                    prop:value=rhs.clone()
                                                    disabled=is_dirty
                                                    on:input:target=move |ev| {
                                                        grammar_rules.update(|rules| {
                                                            if let Some(r) = rules.get_mut(idx) {
                                                                r.1 = ev.target().value();
                                                            }
                                                        });
                                                    }
                                                />
                                            </td>
                                            <td class="g-delete">
                                                <button
                                                    type="button"
                                                    class="grammar-delete-btn"
                                                    disabled=is_dirty
                                                    on:click=move |_| {
                                                        grammar_rules.update(|rules| { rules.remove(idx); });
                                                    }
                                                >"×"</button>
                                            </td>
                                        </tr>
                                    }
                                }).collect_view()
                            }}
                        </tbody>
                    </table>
                    </div>

                    <div class="btn-row">
                        <div title=dirty_tooltip>
                            <button
                                type="button"
                                disabled=is_dirty
                                on:click=move |_| {
                                    grammar_rules.update(|rules| rules.push((String::new(), String::new())));
                                }
                            >"Add rule"</button>
                        </div>
                        <div title=move || {
                            if is_dirty() { "Apply or Revert TOML changes first" }
                            else if grammar_has_3d_symbols() && !is_3d() {
                                "Contains 3D-only symbols — switch to 3D mode in Parameters first"
                            } else { "" }
                        }>
                            <button
                                type="button"
                                disabled=move || !grammar_can_apply()
                                on:click=move |_| try_apply_grammar()
                            >"Apply"</button>
                        </div>
                        <button
                            type="button"
                            disabled=move || !grammar_is_dirty()
                            on:click=move |_| sync_grammar_editor()
                        >"Revert"</button>
                    </div>
                    {move || error.get().map(|msg| view! {
                        <span class="inline-status error">{msg}</span>
                    })}
                </crate::ui::Disclosure>

                <crate::ui::Disclosure title="Parameters">
                    <div style="display:flex;flex-direction:column;gap:5px">
                        <span class="section-label">"Dimensions"</span>
                        <div title=dirty_tooltip>
                        <crate::ui::SegmentedToggle
                            options=vec![("2d", "2D"), ("3d", "3D")]
                            selected=Signal::derive(move || match generation_config.get().dimensions {
                                Dimensions::TwoD => "2d",
                                Dimensions::ThreeD => "3d",
                            })
                            on_change=move |key| try_set_dimensions(key)
                            disabled=Signal::derive(is_dirty)
                        />
                        </div>
                        {move || dims_3d_error.get().map(|msg| view! {
                            <span class="inline-status error">{msg}</span>
                        })}
                    </div>

                    <div class="spinner-row" title=dirty_tooltip>
                        <span class="spinner-label">"Angle (°)"</span>
                        <crate::ui::Spinner
                            value=Signal::derive(move || format!("{:.1}", angle.get()))
                            step=0.5
                            disabled=Signal::derive(is_dirty)
                            on_commit=move |s| {
                                if let Ok(v) = s.parse::<f32>() {
                                    let v = v.clamp(1.0, 180.0);
                                    update_clean_config(
                                        config_workspace, error, "angle",
                                        move |clean| clean.set_angle(v),
                                    );
                                }
                            }
                        />
                    </div>

                    <div class="spinner-row" title=dirty_tooltip>
                        <span class="spinner-label">"Initial heading (°)"</span>
                        <crate::ui::Spinner
                            value=Signal::derive(move || format!(
                                "{:.1}",
                                config_workspace.with(|ws| ws.selected().applied_config().generation.initial_heading)
                            ))
                            step=1.0
                            disabled=Signal::derive(is_dirty)
                            on_commit=move |s| {
                                if let Ok(v) = s.parse::<f32>() {
                                    update_clean_config(
                                        config_workspace, error, "initial_heading",
                                        move |clean| clean.set_initial_heading(v),
                                    );
                                }
                            }
                        />
                    </div>

                    <div class="spinner-row" title=dirty_tooltip>
                        <span class="spinner-label">"Iterations"</span>
                        <crate::ui::Spinner
                            value=Signal::derive(move || iterations.get().to_string())
                            step=1.0
                            disabled=Signal::derive(is_dirty)
                            on_commit=move |s| {
                                if let Ok(v) = s.parse::<u32>() {
                                    let v = v.clamp(0, max_iterations.get_untracked());
                                    update_clean_config(
                                        config_workspace, error, "iterations",
                                        move |clean| clean.set_iterations(v),
                                    );
                                }
                            }
                        />
                    </div>
                </crate::ui::Disclosure>

                <crate::ui::Disclosure title="Colors">
                    <div
                        style="display:flex;flex-direction:column;gap:9px"
                        title=dirty_tooltip
                    >
                    <label class="check-row" for="background-override">
                        <input
                            id="background-override"
                            type="checkbox"
                            prop:checked=move || color_config.with(|c| c.background.is_some())
                            disabled=is_dirty
                            on:change:target=move |ev| {
                                let current_bg =
                                    color_config.with_untracked(|c| c.background);
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
                                    "background checkbox",
                                    move |clean| clean.set_background(background),
                                );
                            }
                        />
                        <span>"Background"</span>
                    </label>

                    <div
                        class="color-row"
                        class:hidden=move || color_config.with(|c| c.background.is_none())
                    >
                        <label for="background-color">"Background color"</label>
                        <input
                            id="background-color"
                            type="color"
                            prop:value=move || {
                                rgb_to_hex(color_config.with(|c| c.effective_background()))
                            }
                            disabled=is_dirty
                            on:input:target=move |ev| {
                                let Some(color) = hex_to_rgb(&ev.target().value()) else {
                                    error.set(Some("Invalid color value.".to_string()));
                                    return;
                                };
                                if update_clean_config(
                                    config_workspace,
                                    error,
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
                            LineColorMode::from_line_color(&current_line_color(color_config))
                                .key()
                                .to_string()
                        }
                        disabled=is_dirty
                        on:change:target=move |ev| {
                            let mode_key = ev.target().value();
                            let Some(mode) = LineColorMode::from_key(&mode_key) else {
                                log::error!("unknown line color mode selected: {mode_key}");
                                return;
                            };
                            let current = color_config.with_untracked(|c| c.line);
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
                                "line color mode select",
                                move |clean| clean.set_line_color(line_color),
                            ) {
                                color_memory.update(|memory| {
                                    memory.remember_line(line_color);
                                });
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
                            !matches!(current_line_color(color_config), LineColorConfig::Solid { .. })
                        }
                    >
                        <label for="line-solid-color">"Line color"</label>
                        <input
                            id="line-solid-color"
                            type="color"
                            prop:value=move || {
                                match line_color_for_mode(
                                    color_config,
                                    color_memory,
                                    LineColorMode::Solid,
                                ) {
                                    LineColorConfig::Solid { color } => rgb_to_hex(color),
                                    _ => unreachable!("solid mode must provide solid color"),
                                }
                            }
                            disabled=is_dirty
                            on:input:target=move |ev| {
                                let Some(color) = hex_to_rgb(&ev.target().value()) else {
                                    error.set(Some("Invalid color value.".to_string()));
                                    return;
                                };
                                let line_color = LineColorConfig::Solid { color };
                                if update_clean_config(
                                    config_workspace,
                                    error,
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
                                current_line_color(color_config),
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
                                    color_config,
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
                            disabled=is_dirty
                            on:input:target=move |ev| {
                                let Some(start) = hex_to_rgb(&ev.target().value()) else {
                                    error.set(Some("Invalid color value.".to_string()));
                                    return;
                                };
                                let current = line_color_for_mode_untracked(
                                    color_config,
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
                                    color_config,
                                    color_memory,
                                    LineColorMode::DepthGradient,
                                ) {
                                    LineColorConfig::DepthGradient { end, .. } => rgb_to_hex(end),
                                    _ => unreachable!(
                                        "depth-gradient mode must provide depth-gradient color"
                                    ),
                                }
                            }
                            disabled=is_dirty
                            on:input:target=move |ev| {
                                let Some(end) = hex_to_rgb(&ev.target().value()) else {
                                    error.set(Some("Invalid color value.".to_string()));
                                    return;
                                };
                                let current = line_color_for_mode_untracked(
                                    color_config,
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
                                current_line_color(color_config),
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
                                    color_config,
                                    color_memory,
                                    LineColorMode::Gradient,
                                ) {
                                    LineColorConfig::Gradient { start, .. } => rgb_to_hex(start),
                                    _ => unreachable!("gradient mode must provide gradient color"),
                                }
                            }
                            disabled=is_dirty
                            on:input:target=move |ev| {
                                let Some(start) = hex_to_rgb(&ev.target().value()) else {
                                    error.set(Some("Invalid color value.".to_string()));
                                    return;
                                };
                                let current = line_color_for_mode_untracked(
                                    color_config,
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
                                    color_config,
                                    color_memory,
                                    LineColorMode::Gradient,
                                ) {
                                    LineColorConfig::Gradient { end, .. } => rgb_to_hex(end),
                                    _ => unreachable!("gradient mode must provide gradient color"),
                                }
                            }
                            disabled=is_dirty
                            on:input:target=move |ev| {
                                let Some(end) = hex_to_rgb(&ev.target().value()) else {
                                    error.set(Some("Invalid color value.".to_string()));
                                    return;
                                };
                                let current = line_color_for_mode_untracked(
                                    color_config,
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
                                current_line_color(color_config),
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
                                    color_config,
                                    color_memory,
                                    LineColorMode::HueCycle,
                                ) {
                                    LineColorConfig::HueCycle { initial } => rgb_to_hex(initial),
                                    _ => unreachable!("hue-cycle mode must provide hue-cycle color"),
                                }
                            }
                            disabled=is_dirty
                            on:input:target=move |ev| {
                                let Some(initial) = hex_to_rgb(&ev.target().value()) else {
                                    error.set(Some("Invalid color value.".to_string()));
                                    return;
                                };
                                let line_color = LineColorConfig::HueCycle { initial };
                                if update_clean_config(
                                    config_workspace,
                                    error,
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
                    </div>
                </crate::ui::Disclosure>

                <crate::ui::Disclosure title="Animations">
                    <div style="display:flex;flex-direction:column;gap:6px">
                        <span class="section-label">"Auto-rotate"</span>
                        <div title=move || if !is_3d() { "Switch to 3D mode in Parameters to enable auto-rotate" } else { "" }>
                        <crate::ui::SegmentedToggle
                            options=vec![("off", "Off"), ("on", "On")]
                            selected=Signal::derive(move || if auto_rotate.get() { "on" } else { "off" })
                            on_change=move |key| {
                                if key == "on" { auto_rotate.set(true); }
                                else { auto_rotate.set(false); }
                            }
                            disabled=Signal::derive(move || !is_3d())
                        />
                        </div>
                        <Show when=move || auto_rotate.get() && is_3d()>
                            <div class="spinner-row">
                                <span class="spinner-label">"Speed (°/s)"</span>
                                <crate::ui::Spinner
                                    value=Signal::derive(move || format!("{:.0}", auto_rotate_speed.get()))
                                    step=5.0
                                    on_commit=move |s| {
                                        if let Ok(v) = s.parse::<f32>() {
                                            auto_rotate_speed.set(v.clamp(5.0, 360.0));
                                        }
                                    }
                                />
                            </div>
                        </Show>
                    </div>

                    <hr class="section-divider" />

                    <div style="display:flex;flex-direction:column;gap:6px">
                        <span class="section-label">"Hue movement"</span>
                        <div title=move || if !matches!(current_line_color(color_config), LineColorConfig::HueCycle { .. }) { "Select Hue cycle in Colors to enable hue movement" } else { "" }>
                        <crate::ui::SegmentedToggle
                            options=vec![("off", "Off"), ("forward", "Forward"), ("backward", "Backward")]
                            selected=Signal::derive(move || {
                                if !hsv_movement.get() { "off" }
                                else if hsv_movement_direction.get() == HsvMovementDirection::Forward { "forward" }
                                else { "backward" }
                            })
                            on_change=move |key| try_set_hue_movement(key)
                            disabled_keys=Signal::derive(move || {
                                if matches!(current_line_color(color_config), LineColorConfig::HueCycle { .. }) {
                                    vec![]
                                } else {
                                    vec!["forward", "backward"]
                                }
                            })
                        />
                        </div>
                        <Show when=move || hsv_movement.get()>
                            <div class="spinner-row">
                                <span class="spinner-label">"Speed (°/s)"</span>
                                <crate::ui::Spinner
                                    value=Signal::derive(move || format!("{:.0}", hsv_movement_speed.get()))
                                    step=1.0
                                    on_commit=move |s| {
                                        if let Ok(v) = s.parse::<f32>() {
                                            hsv_movement_speed.set(v.clamp(
                                                HSV_MOVEMENT_MIN_SPEED_DEGREES_PER_SECOND,
                                                HSV_MOVEMENT_MAX_SPEED_DEGREES_PER_SECOND,
                                            ));
                                        }
                                    }
                                />
                            </div>
                        </Show>
                    </div>
                </crate::ui::Disclosure>

                <crate::ui::Disclosure title="Save image" open=false>
                    {move || if is_3d() {
                        view! { <span class="section-label">"PNG"</span> }.into_any()
                    } else {
                        view! {
                            <crate::ui::SegmentedToggle
                                options=vec![("svg", "SVG"), ("png", "PNG")]
                                selected=Signal::derive(move || save_format.get())
                                on_change=move |key| save_format.set(key)
                            />
                        }.into_any()
                    }}

                    <Show when=move || save_format.get() == "png">
                        <div class="spinner-row">
                            <span class="spinner-label">"Width (px)"</span>
                            <crate::ui::Spinner
                                value=Signal::derive(move || png_width.get().to_string())
                                step=16.0
                                on_commit=move |s| {
                                    if let Ok(v) = s.parse::<u32>() {
                                        png_width.set(v.clamp(256, 4096));
                                    }
                                }
                            />
                        </div>
                    </Show>

                    <button
                        type="button"
                        on:click=move |_| {
                            let config = config_for_render();
                            if save_format.get_untracked() == "svg" {
                                export_svg(config);
                            } else {
                                let Some(Some((device, queue, camera))) =
                                    renderer.try_with_value(|opt| {
                                        opt.as_ref().map(|r| {
                                            let (d, q) = r.device_queue();
                                            (d, q, r.camera())
                                        })
                                    })
                                else {
                                    error.set(Some("Cannot save: GPU renderer not ready.".to_string()));
                                    return;
                                };
                                export_png(
                                    device,
                                    queue,
                                    camera,
                                    config,
                                    png_width.get_untracked(),
                                    move |e| error.set(Some(e)),
                                );
                            }
                        }
                    >"Save"</button>
                </crate::ui::Disclosure>

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
                        } else if matches!(
                            generation_config.get_untracked().dimensions,
                            Dimensions::ThreeD
                        ) {
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

fn rules_to_editor_rows(rules: &std::collections::BTreeMap<char, String>) -> Vec<(String, String)> {
    rules
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect()
}

fn update_clean_config(
    config_workspace: RwSignal<ConfigWorkspace>,
    error: RwSignal<Option<String>>,
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

fn current_line_color(color_config: Memo<ColorConfig>) -> LineColorConfig {
    color_config.with(|c| c.line)
}

fn line_color_for_mode(
    color_config: Memo<ColorConfig>,
    memory: RwSignal<ColorControlMemory>,
    mode: LineColorMode,
) -> LineColorConfig {
    let current = current_line_color(color_config);
    if LineColorMode::from_line_color(&current) == mode {
        current
    } else {
        memory.with(|m| m.line_for(mode))
    }
}

fn line_color_for_mode_untracked(
    color_config: Memo<ColorConfig>,
    memory: RwSignal<ColorControlMemory>,
    mode: LineColorMode,
) -> LineColorConfig {
    let current = color_config.with_untracked(|c| c.line);
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
