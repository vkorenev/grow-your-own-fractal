use std::cell::{Cell, RefCell};
use std::rc::Rc;

use leptos::html::Canvas;
use leptos::prelude::*;
use lsystem_core::{Config, ConfigWorkspace, ConfigWorkspaceError, Dimensions};
use lsystem_renderer::line_renderer::FrameSkipReason;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use crate::export::{export_png, export_svg};
use crate::presets::{effective_config, load_presets, max_iterations_for_config};
use crate::renderer::{CanvasRenderer, RenderStatus};

const ROTATION_STEP_DEG: f32 = 5.0;
const AUTO_ROTATE_DT_MS: f32 = 16.0;

#[component]
pub(crate) fn App() -> impl IntoView {
    let initial_workspace =
        ConfigWorkspace::from_presets(load_presets()).expect("bundled presets should parse");
    let initial_config_index = 0usize;
    let initial_entry = initial_workspace
        .entry(initial_config_index)
        .expect("workspace should contain at least one config");
    let first_toml = initial_entry.draft_text().into_owned();
    let initial_config = initial_entry.applied_config();
    let initial_max_iterations = max_iterations_for_config(&initial_config.generation);

    let (config_workspace, set_config_workspace) = signal(initial_workspace);
    let (selected_config_index, set_selected_config_index) = signal(initial_config_index);
    let (toml_text, set_toml_text) = signal(first_toml);
    let (base_config, set_base_config) = signal(Some(initial_config.clone()));
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
    let is_dirty = move || {
        let index = selected_config_index.get();
        config_workspace
            .with(|workspace| workspace.entry(index).is_some_and(|entry| entry.is_dirty()))
    };

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

    let apply_current = {
        let render_current = Rc::clone(&render_current);
        let interval_id = Rc::clone(&interval_id);
        let install_config = Rc::clone(&install_config);
        move || {
            let applied = set_config_workspace.try_update(|workspace| {
                let index = selected_config_index.get_untracked();
                let Some(entry) = workspace.entry_mut(index) else {
                    return Err(ConfigWorkspaceError::InvalidIndex(index));
                };
                entry.set_draft_text(toml_text.get_untracked());
                workspace.apply(index)
            });
            match applied {
                Some(Ok(config)) => {
                    let new_is_3d = matches!(config.generation.dimensions, Dimensions::ThreeD);
                    install_config(config);
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
                    if let ConfigWorkspaceError::InvalidIndex(index) = err {
                        log::error!("apply: entry_mut returned None for index {index}");
                        set_error.set(Some(
                            "Internal error: selected config is unavailable.".to_string(),
                        ));
                    } else {
                        set_error.set(Some(err.to_string()));
                    }
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
        let install_config = Rc::clone(&install_config);
        move || {
            let index = selected_config_index.get_untracked();
            if let Some(config) = config_workspace.with_untracked(|workspace| {
                workspace.entry(index).map(|entry| entry.applied_config())
            }) {
                install_config(config);
                render_current();
            } else {
                set_error.set(Some(
                    "Internal error: selected config is unavailable.".to_string(),
                ));
            }
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
                    prop:value=move || selected_config_index.get().to_string()
                    on:change={
                        let select_current_config = Rc::clone(&select_current_config);
                        move |ev| {
                            let idx = select_value(ev).parse::<usize>().unwrap_or(0);
                            let selected = config_workspace.with_untracked(|workspace| {
                                workspace.entry(idx).map(|entry| entry.draft_text().into_owned())
                            });
                            match selected {
                                Some(text) => {
                                    set_selected_config_index.set(idx);
                                    set_toml_text.set(text);
                                    select_current_config();
                                }
                                None => {
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
                        let install_config = Rc::clone(&install_config);
                        move |_| {
                            let copied = set_config_workspace.try_update(|workspace| {
                                workspace.copy(selected_config_index.get_untracked())
                            });
                            match copied {
                                Some(Ok((index, entry))) => {
                                    set_selected_config_index.set(index);
                                    set_toml_text.set(entry.draft_text().into_owned());
                                    install_config(entry.applied_config());
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
                    on:input=move |ev| {
                        let text = textarea_value(ev);
                        set_toml_text.set(text.clone());
                        set_config_workspace.update(|workspace| {
                            let index = selected_config_index.get_untracked();
                            let Some(entry) = workspace.entry_mut(index) else {
                                log::error!("on_input: entry_mut returned None for index {index}");
                                set_error.set(Some(
                                    "Internal error: selected config is unavailable.".to_string(),
                                ));
                                return;
                            };
                            entry.set_draft_text(text);
                        });
                    }
                />

                <div class="row">
                    <button
                        type="button"
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
                            let install_config = Rc::clone(&install_config);
                            move |_| {
                                let reverted = set_config_workspace.try_update(|workspace| {
                                    let entry =
                                        workspace.entry_mut(selected_config_index.get_untracked())?;
                                    entry.revert();
                                    Some((
                                        entry.draft_text().into_owned(),
                                        entry.applied_config(),
                                    ))
                                });
                                match reverted {
                                    Some(Some((text, config))) => {
                                        set_toml_text.set(text);
                                        install_config(config);
                                        render_current();
                                    }
                                    Some(None) => {
                                        set_error.set(Some(
                                            "Internal error: selected config is unavailable."
                                                .to_string(),
                                        ));
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
                            let index = selected_config_index.get();
                            config_workspace.with(|workspace| {
                                workspace
                                    .entry(index)
                                    .is_none_or(|entry| entry.is_dirty())
                                    || !workspace.can_reset(index)
                            })
                        }
                        on:click={
                            let render_current = Rc::clone(&render_current);
                            let install_config = Rc::clone(&install_config);
                            move |_| {
                                if is_dirty() {
                                    return;
                                }
                                let reset = set_config_workspace.try_update(|workspace| {
                                    workspace.reset(selected_config_index.get_untracked())
                                });
                                match reset {
                                    Some(Ok(Some(entry))) => {
                                        set_toml_text.set(entry.draft_text().into_owned());
                                        install_config(entry.applied_config());
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
                            on:input={
                                let render_current = Rc::clone(&render_current);
                                let install_config = Rc::clone(&install_config);
                                move |ev| {
                                    let next = input_value(ev)
                                        .parse::<u32>()
                                        .unwrap_or(0)
                                        .clamp(0, max_iterations.get_untracked());
                                    let updated = set_config_workspace.try_update(|workspace| {
                                        let index = selected_config_index.get_untracked();
                                        let Some(entry) = workspace.entry_mut(index) else {
                                            return Err((index, None));
                                        };
                                        entry
                                            .set_iterations(next)
                                            .map_err(|error| (index, Some(error.to_string())))?;
                                        Ok((entry.draft_text().into_owned(), entry.applied_config()))
                                    });
                                    match updated {
                                        Some(Ok((text, config))) => {
                                            set_toml_text.set(text);
                                            install_config(config);
                                            render_current();
                                        }
                                        Some(Err((index, None))) => {
                                            log::error!(
                                                "iterations input: entry_mut returned None for index {index}"
                                            );
                                            set_error.set(Some(
                                                "Internal error: selected config is unavailable."
                                                    .to_string(),
                                            ));
                                        }
                                        Some(Err((_index, Some(message)))) => {
                                            set_error.set(Some(message));
                                        }
                                        None => {
                                            log::error!(
                                                "iterations input: config_workspace signal was unavailable"
                                            );
                                            set_error.set(Some(
                                                "Internal error: could not update config."
                                                    .to_string(),
                                            ));
                                        }
                                    }
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
                            on:input={
                                let render_current = Rc::clone(&render_current);
                                let install_config = Rc::clone(&install_config);
                                move |ev| {
                                    let next = input_value(ev)
                                        .parse::<f32>()
                                        .unwrap_or(60.0)
                                        .clamp(1.0, 180.0);
                                    let updated = set_config_workspace.try_update(|workspace| {
                                        let index = selected_config_index.get_untracked();
                                        let Some(entry) = workspace.entry_mut(index) else {
                                            return Err((index, None));
                                        };
                                        entry
                                            .set_angle(next)
                                            .map_err(|error| (index, Some(error.to_string())))?;
                                        Ok((entry.draft_text().into_owned(), entry.applied_config()))
                                    });
                                    match updated {
                                        Some(Ok((text, config))) => {
                                            set_toml_text.set(text);
                                            install_config(config);
                                            render_current();
                                        }
                                        Some(Err((index, None))) => {
                                            log::error!(
                                                "angle input: entry_mut returned None for index {index}"
                                            );
                                            set_error.set(Some(
                                                "Internal error: selected config is unavailable."
                                                    .to_string(),
                                            ));
                                        }
                                        Some(Err((_index, Some(message)))) => {
                                            set_error.set(Some(message));
                                        }
                                        None => {
                                            log::error!(
                                                "angle input: config_workspace signal was unavailable"
                                            );
                                            set_error.set(Some(
                                                "Internal error: could not update config."
                                                    .to_string(),
                                            ));
                                        }
                                    }
                                }
                            }
                        />
                        <output>{move || format!("{:.1}", angle.get())}</output>
                    </div>

                    <label for="png-width">"PNG width"</label>
                    <input
                        id="png-width"
                        type="number"
                        min="256"
                        max="4096"
                        step="16"
                        prop:value=move || png_width.get().to_string()
                        on:input=move |ev| {
                            let next = input_value(ev).parse::<u32>().unwrap_or(2048);
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
                                on:input=move |ev| {
                                    let next = input_value(ev).parse::<f32>().unwrap_or(45.0);
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

fn input_value(ev: web_sys::Event) -> String {
    ev.target()
        .and_then(|target| target.dyn_into::<web_sys::HtmlInputElement>().ok())
        .map(|input| input.value())
        .unwrap_or_default()
}

fn textarea_value(ev: web_sys::Event) -> String {
    ev.target()
        .and_then(|target| target.dyn_into::<web_sys::HtmlTextAreaElement>().ok())
        .map(|input| input.value())
        .unwrap_or_default()
}

fn select_value(ev: web_sys::Event) -> String {
    ev.target()
        .and_then(|target| target.dyn_into::<web_sys::HtmlSelectElement>().ok())
        .map(|select| select.value())
        .unwrap_or_default()
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
