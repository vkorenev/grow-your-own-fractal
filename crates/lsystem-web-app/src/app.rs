use std::cell::{Cell, RefCell};
use std::rc::Rc;

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
use crate::presets::{effective_config, load_presets, max_iterations_for_config};
use crate::renderer::{CanvasRenderer, RenderStatus};

const ROTATION_STEP_DEG: f32 = 5.0;
const AUTO_ROTATE_DT_MS: f32 = 16.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum LineColorMode {
    Solid,
    Gradient,
    HueCycle,
}

impl LineColorMode {
    fn from_line_color(line_color: &LineColorConfig) -> Self {
        match line_color {
            LineColorConfig::Solid { .. } => Self::Solid,
            LineColorConfig::Gradient { .. } => Self::Gradient,
            LineColorConfig::HueCycle { .. } => Self::HueCycle,
        }
    }

    fn from_key(key: &str) -> Option<Self> {
        match key {
            "solid" => Some(Self::Solid),
            "gradient" => Some(Self::Gradient),
            "hue_cycle" => Some(Self::HueCycle),
            _ => None,
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Solid => "solid",
            Self::Gradient => "gradient",
            Self::HueCycle => "hue_cycle",
        }
    }

    fn default_line_color(self) -> LineColorConfig {
        match self {
            Self::Solid => LineColorConfig::DEFAULT_SOLID,
            Self::Gradient => LineColorConfig::DEFAULT_GRADIENT,
            Self::HueCycle => LineColorConfig::DEFAULT_HUE_CYCLE,
        }
    }
}

#[derive(Clone, Copy)]
struct ColorControlMemory {
    background: [f32; 3],
    solid: Option<[f32; 3]>,
    gradient: Option<([f32; 3], [f32; 3])>,
    hue_cycle: Option<[f32; 3]>,
}

impl ColorControlMemory {
    fn from_colors(colors: &ColorConfig) -> Self {
        let mut memory = Self {
            background: colors.background.unwrap_or(ColorConfig::DEFAULT_BACKGROUND),
            solid: None,
            gradient: None,
            hue_cycle: None,
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
            _ => mode.default_line_color(),
        }
    }
}

#[component]
pub(crate) fn App() -> impl IntoView {
    let initial_workspace =
        ConfigWorkspace::from_presets(load_presets()).expect("bundled presets should parse");
    let initial_entry = initial_workspace.selected();
    let first_toml = initial_entry.draft_text().into_owned();
    let initial_config = initial_entry.applied_config().clone();
    let initial_max_iterations = max_iterations_for_config(&initial_config.generation);

    let (config_workspace, set_config_workspace) = signal(initial_workspace);
    let (toml_text, set_toml_text) = signal(first_toml);
    let (base_config, set_base_config) = signal(Some(initial_config.clone()));
    let (color_memory, set_color_memory) =
        signal(ColorControlMemory::from_colors(&initial_config.colors));
    let (error, set_error) = signal(None::<String>);
    let (iterations, set_iterations) = signal(
        initial_config
            .generation
            .iterations
            .min(initial_max_iterations),
    );
    let (max_iterations, set_max_iterations) = signal(initial_max_iterations);
    let (angle, set_angle) = signal(initial_config.generation.angle);
    let (png_width, set_png_width) = signal(2048u32);
    let (gpu_error, set_gpu_error) = signal(None::<String>);
    let (auto_rotate, set_auto_rotate) = signal(false);
    let (auto_rotate_speed, set_auto_rotate_speed) = signal(45.0f32);

    let is_3d = move || {
        base_config
            .get()
            .map(|c| matches!(c.generation.dimensions, Dimensions::ThreeD))
            .unwrap_or(false)
    };
    let is_3d_untracked = move || {
        base_config
            .get_untracked()
            .map(|c| matches!(c.generation.dimensions, Dimensions::ThreeD))
            .unwrap_or(false)
    };
    let is_dirty = move || config_workspace.with(|workspace| workspace.selected().is_dirty());

    let canvas_ref = NodeRef::<Canvas>::new();
    let renderer = Rc::new(RefCell::new(None::<CanvasRenderer>));
    let last_pointer = Rc::new(RefCell::new(None::<(i32, f64, f64)>));
    let interval_id: Rc<Cell<Option<i32>>> = Rc::new(Cell::new(None));

    let recover_after_render = {
        let renderer = Rc::clone(&renderer);
        move |status: RenderStatus, canvas: web_sys::HtmlCanvasElement| match status {
            RenderStatus::Rendered
            | RenderStatus::Skipped(FrameSkipReason::Timeout | FrameSkipReason::Occluded) => {}
            RenderStatus::Skipped(reason) => {
                log::error!("Skipped GPU frame: {reason}");
            }
            RenderStatus::SurfaceLost => {
                log::error!("GPU surface was lost; attempting to recreate it");
                let renderer = Rc::clone(&renderer);
                wasm_bindgen_futures::spawn_local(async move {
                    let Some(mut renderer_state) = renderer.borrow_mut().take() else {
                        return;
                    };

                    match renderer_state
                        .recover_surface_and_render(canvas.clone())
                        .await
                    {
                        Ok(RenderStatus::Rendered)
                        | Ok(RenderStatus::Skipped(
                            FrameSkipReason::Timeout | FrameSkipReason::Occluded,
                        )) => {
                            set_gpu_error.set(None);
                            *renderer.borrow_mut() = Some(renderer_state);
                        }
                        Ok(RenderStatus::Skipped(reason)) => {
                            log::error!("Skipped GPU frame after surface recovery: {reason}");
                            set_gpu_error.set(None);
                            *renderer.borrow_mut() = Some(renderer_state);
                        }
                        Ok(RenderStatus::SurfaceLost) => {
                            log::error!("GPU surface was lost again after recovery");
                            set_gpu_error.set(Some(
                                "GPU surface was lost again after recovery".to_string(),
                            ));
                        }
                        Err(err) => {
                            log::error!("Failed to recover GPU surface: {err}");
                            set_gpu_error.set(Some(err.to_string()));
                        }
                    }
                });
            }
        }
    };
    let recover_after_render = Rc::new(recover_after_render);

    let render_current = {
        let renderer = Rc::clone(&renderer);
        let recover_after_render = Rc::clone(&recover_after_render);
        move || {
            let Some(canvas) = canvas_ref.get_untracked() else {
                return;
            };
            let Some(config) = effective_config(
                base_config.get_untracked(),
                iterations.get_untracked(),
                angle.get_untracked(),
            ) else {
                return;
            };
            with_renderer(
                canvas,
                &renderer,
                &recover_after_render,
                |renderer, canvas| renderer.set_config_and_render(canvas, &config),
            );
        }
    };
    let render_current = Rc::new(render_current);

    let render_current_colors = {
        let renderer = Rc::clone(&renderer);
        let recover_after_render = Rc::clone(&recover_after_render);
        move || {
            let Some(canvas) = canvas_ref.get_untracked() else {
                return;
            };
            let Some(config) = base_config.get_untracked() else {
                return;
            };
            with_renderer(
                canvas,
                &renderer,
                &recover_after_render,
                |renderer, canvas| renderer.set_colors_and_render(canvas, &config.colors),
            );
        }
    };
    let render_current_colors = Rc::new(render_current_colors);

    canvas_ref.on_load({
        let renderer = Rc::clone(&renderer);
        let render_current = Rc::clone(&render_current);
        move |canvas| {
            wasm_bindgen_futures::spawn_local(async move {
                match CanvasRenderer::new(canvas.clone()).await {
                    Ok(new_renderer) => {
                        set_gpu_error.set(None);
                        *renderer.borrow_mut() = Some(new_renderer);
                        render_current();
                    }
                    Err(err) => {
                        log::error!("Failed to initialize GPU renderer: {err}");
                        set_gpu_error.set(Some(err.to_string()));
                    }
                }
            });
        }
    });

    install_resize_listener(
        canvas_ref,
        Rc::clone(&renderer),
        Rc::clone(&recover_after_render),
    );

    let install_config = Rc::new(move |config: Config| {
        let max = max_iterations_for_config(&config.generation);
        set_max_iterations.set(max);
        set_iterations.set(config.generation.iterations.min(max));
        set_angle.set(config.generation.angle);
        set_base_config.set(Some(config));
        set_error.set(None);
    });

    let install_full_config = Rc::new({
        let install_config = Rc::clone(&install_config);
        move |config: Config| {
            set_color_memory.set(ColorControlMemory::from_colors(&config.colors));
            install_config(config);
        }
    });

    let apply_current = {
        let render_current = Rc::clone(&render_current);
        let interval_id = Rc::clone(&interval_id);
        let install_full_config = Rc::clone(&install_full_config);
        move || {
            if !config_workspace.with_untracked(|workspace| workspace.selected().is_dirty()) {
                return;
            }
            let applied = set_config_workspace.try_update(|workspace| {
                workspace
                    .selected_mut()
                    .set_draft_text(toml_text.get_untracked());
                workspace
                    .apply()
                    .map(|entry| entry.applied_config().clone())
            });
            match applied {
                Some(Ok(config)) => {
                    let new_is_3d = matches!(config.generation.dimensions, Dimensions::ThreeD);
                    install_full_config(config);
                    if !new_is_3d && auto_rotate.get_untracked() {
                        if let Some(id) = interval_id.take()
                            && let Some(window) = web_sys::window()
                        {
                            window.clear_interval_with_handle(id);
                        }
                        set_auto_rotate.set(false);
                    }
                    render_current();
                }
                Some(Err(err)) => {
                    set_error.set(Some(err.to_string()));
                }
                None => {
                    log::error!("apply: config_workspace signal was unavailable");
                    set_error.set(Some("Internal error: could not apply config.".to_string()));
                }
            }
        }
    };
    let apply_current = Rc::new(apply_current);

    let select_current_config = {
        let render_current = Rc::clone(&render_current);
        let install_full_config = Rc::clone(&install_full_config);
        move || {
            let config = config_workspace
                .with_untracked(|workspace| workspace.selected().applied_config().clone());
            install_full_config(config);
            render_current();
        }
    };
    let select_current_config = Rc::new(select_current_config);

    let toggle_auto_rotate = {
        let renderer = Rc::clone(&renderer);
        let recover_after_render = Rc::clone(&recover_after_render);
        let interval_id = Rc::clone(&interval_id);
        move |_: web_sys::MouseEvent| {
            if auto_rotate.get_untracked() {
                if let Some(id) = interval_id.take()
                    && let Some(window) = web_sys::window()
                {
                    window.clear_interval_with_handle(id);
                }
                set_auto_rotate.set(false);
            } else {
                set_auto_rotate.set(true);
                let renderer = Rc::clone(&renderer);
                let recover_after_render = Rc::clone(&recover_after_render);
                let interval_id_store = Rc::clone(&interval_id);
                let interval_id_cancel = Rc::clone(&interval_id);
                let closure = Closure::<dyn FnMut()>::new(move || {
                    let degrees = auto_rotate_speed.get_untracked() * (AUTO_ROTATE_DT_MS / 1000.0);
                    if let Some(canvas) = canvas_ref.get_untracked() {
                        with_renderer(canvas, &renderer, &recover_after_render, |r, c| {
                            r.auto_rotate_and_render(c, degrees)
                        });
                    }
                    // Stop if auto_rotate signal was turned off from outside.
                    if !auto_rotate.get_untracked()
                        && let Some(id) = interval_id_cancel.take()
                        && let Some(window) = web_sys::window()
                    {
                        window.clear_interval_with_handle(id);
                    }
                });
                if let Some(window) = web_sys::window() {
                    match window.set_interval_with_callback_and_timeout_and_arguments_0(
                        closure.as_ref().unchecked_ref(),
                        AUTO_ROTATE_DT_MS as i32,
                    ) {
                        Ok(id) => {
                            interval_id_store.set(Some(id));
                            closure.forget();
                        }
                        Err(err) => {
                            log::error!("Failed to start auto-rotate interval: {err:?}");
                            set_auto_rotate.set(false);
                        }
                    }
                } else {
                    log::error!("Failed to start auto-rotate interval: window is unavailable");
                    set_auto_rotate.set(false);
                }
            }
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
                    on:change:target={
                        let select_current_config = Rc::clone(&select_current_config);
                        move |ev| {
                            let idx = ev.target().value().parse::<usize>().unwrap_or(0);
                            let selected = set_config_workspace.try_update(|workspace| {
                                workspace
                                    .select(idx)
                                    .map(|_| workspace.selected().draft_text().into_owned())
                            });
                            match selected {
                                Some(Ok(text)) => {
                                    set_toml_text.set(text);
                                    select_current_config();
                                }
                                Some(Err(err)) => {
                                    log::error!("select preset: rejected index {idx}: {err}");
                                    set_error.set(Some(err.to_string()));
                                }
                                None => {
                                    log::error!(
                                        "select preset: config_workspace signal was unavailable"
                                    );
                                    set_error.set(Some(
                                        "Internal error: could not select config.".to_string(),
                                    ));
                                }
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
                    on:click={
                        let render_current = Rc::clone(&render_current);
                        let install_full_config = Rc::clone(&install_full_config);
                        move |_| {
                            let copied = set_config_workspace.try_update(|workspace| {
                                workspace.copy().map(|entry| {
                                    (entry.draft_text().into_owned(), entry.applied_config().clone())
                                })
                            });
                            match copied {
                                Some(Ok((text, config))) => {
                                    set_toml_text.set(text);
                                    install_full_config(config);
                                    render_current();
                                }
                                Some(Err(err)) => {
                                    set_error.set(Some(err.to_string()));
                                }
                                None => {
                                    log::error!(
                                        "copy: config_workspace signal was unavailable"
                                    );
                                    set_error.set(Some(
                                        "Internal error: could not copy config.".to_string(),
                                    ));
                                }
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
                        set_toml_text.set(text.clone());
                        let updated = set_config_workspace.try_update(|workspace| {
                            workspace.selected_mut().set_draft_text(text);
                        });
                        if updated.is_none() {
                            log::error!("textarea input: config_workspace signal was unavailable");
                            set_error
                                .set(Some("Internal error: could not update config.".to_string()));
                        }
                    }
                />

                <div class="row">
                    <button
                        type="button"
                        disabled=move || !is_dirty()
                        on:click={
                            let apply_current = Rc::clone(&apply_current);
                            move |_| apply_current()
                        }
                    >
                        "Apply"
                    </button>
                    <button
                        type="button"
                        disabled=move || !is_dirty()
                        on:click={
                            let render_current = Rc::clone(&render_current);
                            let install_full_config = Rc::clone(&install_full_config);
                            move |_| {
                                let reverted = set_config_workspace.try_update(|workspace| {
                                    match workspace.selected_mut().view_mut() {
                                        EntryViewMut::Dirty(dirty) => dirty.revert(),
                                        EntryViewMut::Clean(_) => {
                                            log::error!(
                                                "revert fired while entry is clean; UI guards bypassed"
                                            );
                                        }
                                    }
                                    let entry = workspace.selected();
                                    (entry.draft_text().into_owned(), entry.applied_config().clone())
                                });
                                match reverted {
                                    Some((text, config)) => {
                                        set_toml_text.set(text);
                                        install_full_config(config);
                                        render_current();
                                    }
                                    None => {
                                        log::error!(
                                            "revert: config_workspace signal was unavailable"
                                        );
                                        set_error.set(Some(
                                            "Internal error: could not revert config.".to_string(),
                                        ));
                                    }
                                }
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
                        on:click={
                            let render_current = Rc::clone(&render_current);
                            let install_full_config = Rc::clone(&install_full_config);
                            move |_| {
                                if is_dirty() {
                                    return;
                                }
                                let reset = set_config_workspace.try_update(|workspace| {
                                    workspace.reset().map(|opt| {
                                        opt.map(|entry| {
                                            (entry.draft_text().into_owned(), entry.applied_config().clone())
                                        })
                                    })
                                });
                                match reset {
                                    Some(Ok(Some((text, config)))) => {
                                        set_toml_text.set(text);
                                        install_full_config(config);
                                        render_current();
                                    }
                                    Some(Ok(None)) => {
                                        log::warn!("reset: no-op for entry without a bundled default; button guard may have been bypassed");
                                    }
                                    Some(Err(err)) => {
                                        set_error.set(Some(err.to_string()));
                                    }
                                    None => {
                                        log::error!(
                                            "reset: config_workspace signal was unavailable"
                                        );
                                        set_error.set(Some(
                                            "Internal error: could not reset config.".to_string(),
                                        ));
                                    }
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

                <div class="group" class:hidden=move || base_config.get().is_none() || is_dirty()>
                    <label for="iterations">"Iterations"</label>
                    <div class="row">
                        <input
                            id="iterations"
                            type="range"
                            min="0"
                            max=move || max_iterations.get().to_string()
                            prop:value=move || iterations.get().to_string()
                            on:input:target={
                                let render_current = Rc::clone(&render_current);
                                let install_config = Rc::clone(&install_config);
                                move |ev| {
                                    let next = ev.target().value()
                                        .parse::<u32>()
                                        .unwrap_or(0)
                                        .clamp(0, max_iterations.get_untracked());
                                    update_clean_config(
                                        set_config_workspace,
                                        set_toml_text,
                                        set_error,
                                        &install_config,
                                        &render_current,
                                        "iterations input",
                                        move |clean| clean.set_iterations(next),
                                    );
                                }
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
                            on:input:target={
                                let render_current = Rc::clone(&render_current);
                                let install_config = Rc::clone(&install_config);
                                move |ev| {
                                    let next = ev.target().value()
                                        .parse::<f32>()
                                        .unwrap_or(60.0)
                                        .clamp(1.0, 180.0);
                                    update_clean_config(
                                        set_config_workspace,
                                        set_toml_text,
                                        set_error,
                                        &install_config,
                                        &render_current,
                                        "angle input",
                                        move |clean| clean.set_angle(next),
                                    );
                                }
                            }
                        />
                        <output>{move || format!("{:.1}", angle.get())}</output>
                    </div>

                    <label class="check-row" for="background-override">
                        <input
                            id="background-override"
                            type="checkbox"
                            prop:checked=move || {
                                base_config
                                    .get()
                                    .and_then(|config| config.colors.background)
                                    .is_some()
                            }
                            on:change:target={
                                let render_current_colors = Rc::clone(&render_current_colors);
                                let install_config = Rc::clone(&install_config);
                                move |ev| {
                                    base_config.with_untracked(|config| {
                                        if let Some(Some(background)) =
                                            config.as_ref().map(|config| config.colors.background)
                                        {
                                            set_color_memory.update(|memory| {
                                                memory.remember_background(background);
                                            });
                                        }
                                    });
                                    let enabled = ev.target().checked();
                                    let background = if enabled {
                                        Some(color_memory.get_untracked().background())
                                    } else {
                                        None
                                    };
                                    update_clean_config(
                                        set_config_workspace,
                                        set_toml_text,
                                        set_error,
                                        &install_config,
                                        &render_current_colors,
                                        "background checkbox",
                                        move |clean| clean.set_background(background),
                                    );
                                }
                            }
                        />
                        <span>"Background"</span>
                    </label>

                    <div class="color-row" class:hidden=move || {
                        base_config
                            .get()
                            .and_then(|config| config.colors.background)
                            .is_none()
                    }>
                        <label for="background-color">"Background color"</label>
                        <input
                            id="background-color"
                            type="color"
                            prop:value=move || {
                                rgb_to_hex(
                                    base_config
                                        .get()
                                        .map(|config| config.colors.effective_background())
                                        .unwrap_or(ColorConfig::DEFAULT_BACKGROUND),
                                )
                            }
                            on:input:target={
                                let render_current_colors = Rc::clone(&render_current_colors);
                                let install_config = Rc::clone(&install_config);
                                move |ev| {
                                    let Some(color) = hex_to_rgb(&ev.target().value()) else {
                                        set_error.set(Some("Invalid color value.".to_string()));
                                        return;
                                    };
                                    if update_clean_config(
                                        set_config_workspace,
                                        set_toml_text,
                                        set_error,
                                        &install_config,
                                        &render_current_colors,
                                        "background color input",
                                        move |clean| clean.set_background(Some(color)),
                                    ) {
                                        set_color_memory.update(|memory| {
                                            memory.remember_background(color);
                                        });
                                    }
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
                        on:change:target={
                            let render_current_colors = Rc::clone(&render_current_colors);
                            let install_config = Rc::clone(&install_config);
                            move |ev| {
                                let mode_key = ev.target().value();
                                let Some(mode) = LineColorMode::from_key(&mode_key) else {
                                    log::error!("unknown line color mode selected: {mode_key}");
                                    return;
                                };
                                let current = base_config.with_untracked(line_color_of);
                                let Some(line_color) =
                                    set_color_memory.try_update(|memory| {
                                        memory.remember_line(current);
                                        memory.line_for(mode)
                                    })
                                else {
                                    log::error!(
                                        "line color mode select: line color memory signal was unavailable"
                                    );
                                    set_error.set(Some(
                                        "Internal error: could not update line color.".to_string(),
                                    ));
                                    return;
                                };
                                if update_clean_config(
                                    set_config_workspace,
                                    set_toml_text,
                                    set_error,
                                    &install_config,
                                    &render_current_colors,
                                    "line color mode select",
                                    move |clean| clean.set_line_color(line_color),
                                ) {
                                    set_color_memory.update(|memory| {
                                        memory.remember_line(line_color);
                                    });
                                }
                            }
                        }
                    >
                        <option value="solid">"Solid"</option>
                        <option value="gradient">"Gradient"</option>
                        <option value="hue_cycle">"Hue cycle"</option>
                    </select>

                    <div class="color-row" class:hidden=move || {
                        !matches!(
                            Some(current_line_color(base_config)),
                            Some(LineColorConfig::Solid { .. })
                        )
                    }>
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
                            on:input:target={
                                let render_current_colors = Rc::clone(&render_current_colors);
                                let install_config = Rc::clone(&install_config);
                                move |ev| {
                                    let Some(color) = hex_to_rgb(&ev.target().value()) else {
                                        set_error.set(Some("Invalid color value.".to_string()));
                                        return;
                                    };
                                    let line_color = LineColorConfig::Solid { color };
                                    if update_clean_config(
                                        set_config_workspace,
                                        set_toml_text,
                                        set_error,
                                        &install_config,
                                        &render_current_colors,
                                        "solid line color input",
                                        move |clean| clean.set_line_color(line_color),
                                    ) {
                                        set_color_memory.update(|memory| {
                                            memory.remember_line(line_color);
                                        });
                                    }
                                }
                            }
                        />
                    </div>

                    <div class="color-row" class:hidden=move || {
                        !matches!(
                            Some(current_line_color(base_config)),
                            Some(LineColorConfig::Gradient { .. })
                        )
                    }>
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
                            on:input:target={
                                let render_current_colors = Rc::clone(&render_current_colors);
                                let install_config = Rc::clone(&install_config);
                                move |ev| {
                                    let Some(start) = hex_to_rgb(&ev.target().value()) else {
                                        set_error.set(Some("Invalid color value.".to_string()));
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
                                    let line_color = LineColorConfig::Gradient {
                                        start,
                                        end,
                                    };
                                    if update_clean_config(
                                        set_config_workspace,
                                        set_toml_text,
                                        set_error,
                                        &install_config,
                                        &render_current_colors,
                                        "gradient start color input",
                                        move |clean| clean.set_line_color(line_color),
                                    ) {
                                        set_color_memory.update(|memory| {
                                            memory.remember_line(line_color);
                                        });
                                    }
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
                            on:input:target={
                                let render_current_colors = Rc::clone(&render_current_colors);
                                let install_config = Rc::clone(&install_config);
                                move |ev| {
                                    let Some(end) = hex_to_rgb(&ev.target().value()) else {
                                        set_error.set(Some("Invalid color value.".to_string()));
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
                                    let line_color = LineColorConfig::Gradient {
                                        start,
                                        end,
                                    };
                                    if update_clean_config(
                                        set_config_workspace,
                                        set_toml_text,
                                        set_error,
                                        &install_config,
                                        &render_current_colors,
                                        "gradient end color input",
                                        move |clean| clean.set_line_color(line_color),
                                    ) {
                                        set_color_memory.update(|memory| {
                                            memory.remember_line(line_color);
                                        });
                                    }
                                }
                            }
                        />
                    </div>

                    <div class="color-row" class:hidden=move || {
                        !matches!(
                            Some(current_line_color(base_config)),
                            Some(LineColorConfig::HueCycle { .. })
                        )
                    }>
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
                            on:input:target={
                                let render_current_colors = Rc::clone(&render_current_colors);
                                let install_config = Rc::clone(&install_config);
                                move |ev| {
                                    let Some(initial) = hex_to_rgb(&ev.target().value()) else {
                                        set_error.set(Some("Invalid color value.".to_string()));
                                        return;
                                    };
                                    let line_color = LineColorConfig::HueCycle { initial };
                                    if update_clean_config(
                                        set_config_workspace,
                                        set_toml_text,
                                        set_error,
                                        &install_config,
                                        &render_current_colors,
                                        "hue-cycle initial color input",
                                        move |clean| clean.set_line_color(line_color),
                                    ) {
                                        set_color_memory.update(|memory| {
                                            memory.remember_line(line_color);
                                        });
                                    }
                                }
                            }
                        />
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
                            set_png_width.set(next.clamp(256, 4096));
                        }
                    />

                    <div class="export-row">
                        <button
                            type="button"
                            class:hidden=move || is_3d()
                            on:click=move |_| {
                                export_svg(
                                    base_config.get_untracked(),
                                    iterations.get_untracked(),
                                    angle.get_untracked(),
                                )
                            }
                        >
                            "Export SVG"
                        </button>
                        <button
                            type="button"
                            on:click={
                                let renderer = Rc::clone(&renderer);
                                move |_| {
                                    export_png(
                                        Rc::clone(&renderer),
                                        base_config.get_untracked(),
                                        iterations.get_untracked(),
                                        angle.get_untracked(),
                                        png_width.get_untracked(),
                                        move |error| set_gpu_error.set(Some(error)),
                                    );
                                }
                            }
                        >
                            "Export PNG"
                        </button>
                    </div>

                    <div class:hidden=move || !is_3d()>
                        <button
                            type="button"
                            on:click=toggle_auto_rotate
                        >
                            {move || if auto_rotate.get() { "Auto-rotate: On" } else { "Auto-rotate: Off" }}
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
                                    let next = ev.target().value().parse::<f32>().unwrap_or(45.0);
                                    set_auto_rotate_speed.set(next.clamp(10.0, 360.0));
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
                    on:pointerdown={
                        let last_pointer = Rc::clone(&last_pointer);
                        move |ev: web_sys::PointerEvent| {
                            if let Some(canvas) = canvas_ref.get_untracked() {
                                let _ = canvas.focus();
                                let _ = canvas.set_pointer_capture(ev.pointer_id());
                            }
                            *last_pointer.borrow_mut() = Some((
                                ev.pointer_id(),
                                ev.client_x() as f64,
                                ev.client_y() as f64,
                            ));
                        }
                    }
                    on:pointermove={
                        let renderer = Rc::clone(&renderer);
                        let last_pointer = Rc::clone(&last_pointer);
                        let recover_after_render = Rc::clone(&recover_after_render);
                        move |ev: web_sys::PointerEvent| {
                            let mut last = last_pointer.borrow_mut();
                            let Some((id, last_x, last_y)) = *last else {
                                return;
                            };
                            if id != ev.pointer_id() {
                                return;
                            }
                            let x = ev.client_x() as f64;
                            let y = ev.client_y() as f64;
                            let dx = x - last_x;
                            let dy = y - last_y;
                            *last = Some((id, x, y));
                            if let Some(canvas) = canvas_ref.get_untracked() {
                                with_renderer(
                                    canvas,
                                    &renderer,
                                    &recover_after_render,
                                    |renderer, canvas| {
                                        renderer.drag_and_render(canvas, dx as f32, dy as f32)
                                    },
                                );
                            }
                        }
                    }
                    on:pointerup={
                        let last_pointer = Rc::clone(&last_pointer);
                        move |ev: web_sys::PointerEvent| {
                            if let Some(canvas) = canvas_ref.get_untracked() {
                                let _ = canvas.release_pointer_capture(ev.pointer_id());
                            }
                            *last_pointer.borrow_mut() = None;
                        }
                    }
                    on:pointercancel={
                        let last_pointer = Rc::clone(&last_pointer);
                        move |_| *last_pointer.borrow_mut() = None
                    }
                    on:wheel={
                        let renderer = Rc::clone(&renderer);
                        let recover_after_render = Rc::clone(&recover_after_render);
                        move |ev: web_sys::WheelEvent| {
                            ev.prevent_default();
                            if let Some(canvas) = canvas_ref.get_untracked() {
                                with_renderer(
                                    canvas,
                                    &renderer,
                                    &recover_after_render,
                                    |renderer, canvas| {
                                        renderer.zoom_and_render(
                                            canvas,
                                            ev.delta_y() as f32,
                                            ev.delta_mode(),
                                            ev.client_x() as f32,
                                            ev.client_y() as f32,
                                        )
                                    },
                                );
                            }
                        }
                    }
                    on:keydown={
                        let renderer = Rc::clone(&renderer);
                        let recover_after_render = Rc::clone(&recover_after_render);
                        move |ev: web_sys::KeyboardEvent| {
                            let Some(canvas) = canvas_ref.get_untracked() else {
                                return;
                            };
                            let key = ev.key();
                            if key.eq_ignore_ascii_case("f") {
                                with_renderer(
                                    canvas,
                                    &renderer,
                                    &recover_after_render,
                                    |renderer, canvas| renderer.reset_and_render(canvas),
                                );
                            } else if is_3d_untracked() {
                                let handled = match key.as_str() {
                                    "ArrowLeft" => {
                                        with_renderer(canvas, &renderer, &recover_after_render, |r, c| r.orbit_and_render(c, -ROTATION_STEP_DEG, 0.0));
                                        true
                                    }
                                    "ArrowRight" => {
                                        with_renderer(canvas, &renderer, &recover_after_render, |r, c| r.orbit_and_render(c, ROTATION_STEP_DEG, 0.0));
                                        true
                                    }
                                    "ArrowUp" => {
                                        with_renderer(canvas, &renderer, &recover_after_render, |r, c| r.orbit_and_render(c, 0.0, ROTATION_STEP_DEG));
                                        true
                                    }
                                    "ArrowDown" => {
                                        with_renderer(canvas, &renderer, &recover_after_render, |r, c| r.orbit_and_render(c, 0.0, -ROTATION_STEP_DEG));
                                        true
                                    }
                                    "q" | "Q" => {
                                        with_renderer(canvas, &renderer, &recover_after_render, |r, c| r.roll_and_render(c, -ROTATION_STEP_DEG));
                                        true
                                    }
                                    "e" | "E" => {
                                        with_renderer(canvas, &renderer, &recover_after_render, |r, c| r.roll_and_render(c, ROTATION_STEP_DEG));
                                        true
                                    }
                                    _ => false,
                                };
                                if handled {
                                    ev.prevent_default();
                                }
                            }
                        }
                    }
                />
                <div class:hidden=move || gpu_error.get().is_none() class="unsupported">
                    <div>
                        <h2>"GPU rendering is not available in this browser."</h2>
                        <p>{move || gpu_error.get().unwrap_or_else(|| "Try a browser with WebGPU or WebGL2 enabled.".to_string())}</p>
                    </div>
                </div>
            </section>
        </main>
    }
}

fn update_clean_config(
    set_config_workspace: WriteSignal<ConfigWorkspace>,
    set_toml_text: WriteSignal<String>,
    set_error: WriteSignal<Option<String>>,
    install_config: &Rc<impl Fn(Config)>,
    render_after_update: &Rc<impl Fn()>,
    event: &'static str,
    update: impl FnOnce(&mut CleanMut<'_>) -> Result<(), ConfigError>,
) -> bool {
    let updated = set_config_workspace.try_update(|workspace| {
        let entry = workspace.selected_mut();
        match entry.view_mut() {
            EntryViewMut::Clean(mut clean) => {
                update(&mut clean).map_err(|error| error.to_string())?;
            }
            EntryViewMut::Dirty(_) => {
                log::error!("{event} fired while entry is dirty; UI guards bypassed");
                return Ok(None);
            }
        }
        Ok(Some((
            entry.draft_text().into_owned(),
            entry.applied_config().clone(),
        )))
    });

    match updated {
        Some(Ok(Some((text, config)))) => {
            set_toml_text.set(text);
            install_config(config);
            render_after_update();
            true
        }
        Some(Ok(None)) => false,
        Some(Err(message)) => {
            set_error.set(Some(message));
            false
        }
        None => {
            log::error!("{event}: config_workspace signal was unavailable");
            set_error.set(Some("Internal error: could not update config.".to_string()));
            false
        }
    }
}

/// Reads the line color out of an optional config, falling back to the default.
fn line_color_of(config: &Option<Config>) -> LineColorConfig {
    config
        .as_ref()
        .map(|config| config.colors.line)
        .unwrap_or_default()
}

fn current_line_color(base_config: ReadSignal<Option<Config>>) -> LineColorConfig {
    base_config.with(line_color_of)
}

fn line_color_for_mode(
    base_config: ReadSignal<Option<Config>>,
    memory: ReadSignal<ColorControlMemory>,
    mode: LineColorMode,
) -> LineColorConfig {
    let current = current_line_color(base_config);
    if LineColorMode::from_line_color(&current) == mode {
        current
    } else {
        memory.with(|memory| memory.line_for(mode))
    }
}

fn line_color_for_mode_untracked(
    base_config: ReadSignal<Option<Config>>,
    memory: ReadSignal<ColorControlMemory>,
    mode: LineColorMode,
) -> LineColorConfig {
    let current = base_config.with_untracked(line_color_of);
    if LineColorMode::from_line_color(&current) == mode {
        current
    } else {
        memory.with_untracked(|memory| memory.line_for(mode))
    }
}

fn with_renderer<F, H>(
    canvas: web_sys::HtmlCanvasElement,
    renderer: &Rc<RefCell<Option<CanvasRenderer>>>,
    recover_after_render: &Rc<H>,
    render: F,
) where
    F: FnOnce(&mut CanvasRenderer, &web_sys::HtmlCanvasElement) -> RenderStatus,
    H: Fn(RenderStatus, web_sys::HtmlCanvasElement) + 'static,
{
    let status = renderer
        .borrow_mut()
        .as_mut()
        .map(|renderer| render(renderer, &canvas));
    if let Some(status) = status {
        recover_after_render(status, canvas);
    }
}

fn install_resize_listener<H>(
    canvas_ref: NodeRef<Canvas>,
    renderer: Rc<RefCell<Option<CanvasRenderer>>>,
    recover_after_render: Rc<H>,
) where
    H: Fn(RenderStatus, web_sys::HtmlCanvasElement) + 'static,
{
    let Some(window) = web_sys::window() else {
        return;
    };
    let closure = Closure::<dyn FnMut()>::new(move || {
        if let Some(canvas) = canvas_ref.get_untracked() {
            with_renderer(
                canvas,
                &renderer,
                &recover_after_render,
                |renderer, canvas| renderer.render(canvas),
            );
        }
    });
    match window.add_event_listener_with_callback("resize", closure.as_ref().unchecked_ref()) {
        Ok(()) => closure.forget(),
        Err(err) => log::error!("Failed to install resize listener: {err:?}"),
    }
}
