use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use leptos::html::Canvas;
use leptos::prelude::*;
use lsystem_renderer::line_renderer::FrameSkipReason;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use crate::export::{export_png, export_svg};
use crate::presets::{apply_toml, effective_config, load_presets};
use crate::renderer::{CanvasRenderer, RenderStatus};

const ROTATION_STEP_DEG: f32 = 5.0;
const AUTO_ROTATE_DT_MS: f32 = 16.0;

#[component]
pub(crate) fn App() -> impl IntoView {
    let presets = Arc::new(load_presets());
    let first_toml = presets
        .first()
        .expect("no preset TOML files found")
        .1
        .to_string();
    let initial = apply_toml(&first_toml).expect("bundled first preset should parse");

    let (preset_idx, set_preset_idx) = signal(0usize);
    let (toml_text, set_toml_text) = signal(first_toml);
    let (base_config, set_base_config) = signal(Some(initial.config.clone()));
    let (error, set_error) = signal(None::<String>);
    let (iterations, set_iterations) =
        signal(initial.config.iterations.min(initial.max_iterations));
    let (max_iterations, set_max_iterations) = signal(initial.max_iterations);
    let (angle, set_angle) = signal(initial.config.angle);
    let (png_width, set_png_width) = signal(2048u32);
    let (gpu_error, set_gpu_error) = signal(None::<String>);
    let (auto_rotate, set_auto_rotate) = signal(false);
    let (auto_rotate_speed, set_auto_rotate_speed) = signal(45.0f32);

    let is_3d = move || {
        base_config
            .get()
            .map(|c| c.dimensions == 3)
            .unwrap_or(false)
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
            let Some(canvas) = canvas_ref.get() else {
                return;
            };
            let Some(config) = effective_config(base_config.get(), iterations.get(), angle.get())
            else {
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

    let apply_text = {
        let render_current = Rc::clone(&render_current);
        move |text: String| match apply_toml(&text) {
            Ok(applied) => {
                let max = applied.max_iterations;
                set_max_iterations.set(max);
                set_iterations.set(applied.config.iterations.min(max));
                set_angle.set(applied.config.angle);
                set_base_config.set(Some(applied.config));
                set_error.set(None);
                render_current();
            }
            Err(err) => {
                set_error.set(Some(err.to_string()));
            }
        }
    };
    let apply_text = Rc::new(apply_text);

    let toggle_auto_rotate = {
        let renderer = Rc::clone(&renderer);
        let recover_after_render = Rc::clone(&recover_after_render);
        let interval_id = Rc::clone(&interval_id);
        move |_: web_sys::MouseEvent| {
            if auto_rotate.get() {
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
                    let degrees = auto_rotate_speed.get() * (AUTO_ROTATE_DT_MS / 1000.0);
                    if let Some(canvas) = canvas_ref.get() {
                        with_renderer(canvas, &renderer, &recover_after_render, |r, c| {
                            r.auto_rotate_and_render(c, degrees)
                        });
                    }
                    // Stop if auto_rotate signal was turned off from outside.
                    if !auto_rotate.get()
                        && let Some(id) = interval_id_cancel.take()
                        && let Some(window) = web_sys::window()
                    {
                        window.clear_interval_with_handle(id);
                    }
                });
                if let Some(window) = web_sys::window()
                    && let Ok(id) = window.set_interval_with_callback_and_timeout_and_arguments_0(
                        closure.as_ref().unchecked_ref(),
                        AUTO_ROTATE_DT_MS as i32,
                    )
                {
                    interval_id_store.set(Some(id));
                }
                closure.forget();
            }
        }
    };

    let preset_options = presets
        .iter()
        .enumerate()
        .map(|(idx, (name, _))| {
            view! {
                <option value=idx.to_string()>{name.clone()}</option>
            }
        })
        .collect_view();

    view! {
        <main class="app-shell">
            <aside class="controls">
                <h1>"Grow Your Own Fractal"</h1>

                <label for="preset">"Preset"</label>
                <select
                    id="preset"
                    prop:value=move || preset_idx.get().to_string()
                    on:change={
                        let presets = Arc::clone(&presets);
                        let apply_text = Rc::clone(&apply_text);
                        move |ev| {
                            let idx = select_value(ev).parse::<usize>().unwrap_or(0);
                            if let Some((_, text)) = presets.get(idx) {
                                set_preset_idx.set(idx);
                                set_toml_text.set((*text).to_string());
                                apply_text((*text).to_string());
                            }
                        }
                    }
                >
                    {preset_options}
                </select>

                <label for="config">"Config (TOML)"</label>
                <textarea
                    id="config"
                    spellcheck="false"
                    prop:value=move || toml_text.get()
                    on:input=move |ev| set_toml_text.set(textarea_value(ev))
                />

                <button
                    type="button"
                    on:click={
                        let apply_text = Rc::clone(&apply_text);
                        move |_| apply_text(toml_text.get())
                    }
                >
                    "Apply"
                </button>

                <p class=move || if error.get().is_some() { "status error" } else { "status ok" }>
                    {move || error.get().unwrap_or_else(|| "OK".to_string())}
                </p>

                <div class="group" class:hidden=move || base_config.get().is_none()>
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
                                move |ev| {
                                    let next = input_value(ev).parse::<u32>().unwrap_or(0);
                                    set_iterations.set(next.clamp(0, max_iterations.get()));
                                    render_current();
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
                                move |ev| {
                                    let next = input_value(ev).parse::<f32>().unwrap_or(60.0);
                                    set_angle.set(next.clamp(1.0, 180.0));
                                    render_current();
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
                            on:click=move |_| export_svg(base_config.get(), iterations.get(), angle.get())
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
                                        base_config.get(),
                                        iterations.get(),
                                        angle.get(),
                                        png_width.get(),
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
                            if let Some(canvas) = canvas_ref.get() {
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
                            if let Some(canvas) = canvas_ref.get() {
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
                            if let Some(canvas) = canvas_ref.get() {
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
                            if let Some(canvas) = canvas_ref.get() {
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
                            let Some(canvas) = canvas_ref.get() else { return };
                            let key = ev.key();
                            if key.eq_ignore_ascii_case("f") {
                                with_renderer(
                                    canvas,
                                    &renderer,
                                    &recover_after_render,
                                    |renderer, canvas| renderer.reset_and_render(canvas),
                                );
                            } else if is_3d() {
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
        if let Some(canvas) = canvas_ref.get() {
            with_renderer(
                canvas,
                &renderer,
                &recover_after_render,
                |renderer, canvas| renderer.render(canvas),
            );
        }
    });
    if window
        .add_event_listener_with_callback("resize", closure.as_ref().unchecked_ref())
        .is_ok()
    {
        closure.forget();
    }
}
