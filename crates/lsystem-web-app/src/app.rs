use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use leptos::html::Canvas;
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use crate::export::{export_png, export_svg};
use crate::presets::{apply_toml, effective_config, load_presets};
use crate::renderer::CanvasRenderer;

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
    let (unsupported, set_unsupported) = signal(false);

    let canvas_ref = NodeRef::<Canvas>::new();
    let renderer = Rc::new(RefCell::new(None::<CanvasRenderer>));
    let last_pointer = Rc::new(RefCell::new(None::<(i32, f64, f64)>));

    let render_current = {
        let renderer = Rc::clone(&renderer);
        move || {
            let Some(canvas) = canvas_ref.get() else {
                return;
            };
            let Some(config) = effective_config(base_config.get(), iterations.get(), angle.get())
            else {
                return;
            };
            if let Some(renderer) = renderer.borrow_mut().as_mut() {
                renderer.set_config_and_render(&canvas, &config);
            }
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
                        *renderer.borrow_mut() = Some(new_renderer);
                        render_current();
                    }
                    Err(()) => set_unsupported.set(true),
                }
            });
        }
    });

    install_resize_listener(canvas_ref, Rc::clone(&renderer));

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
                        <button type="button" on:click=move |_| export_svg(base_config.get(), iterations.get(), angle.get())>
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
                            if let Some(canvas) = canvas_ref.get()
                                && let Some(renderer) = renderer.borrow_mut().as_mut()
                            {
                                renderer.pan_and_render(&canvas, dx as f32, dy as f32);
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
                        move |ev: web_sys::WheelEvent| {
                            ev.prevent_default();
                            if let Some(canvas) = canvas_ref.get()
                                && let Some(renderer) = renderer.borrow_mut().as_mut()
                            {
                                renderer.zoom_and_render(
                                    &canvas,
                                    ev.delta_y() as f32,
                                    ev.delta_mode(),
                                    ev.client_x() as f32,
                                    ev.client_y() as f32,
                                );
                            }
                        }
                    }
                    on:keydown={
                        let renderer = Rc::clone(&renderer);
                        move |ev: web_sys::KeyboardEvent| {
                            if ev.key().eq_ignore_ascii_case("f")
                                && let Some(canvas) = canvas_ref.get()
                                && let Some(renderer) = renderer.borrow_mut().as_mut()
                            {
                                renderer.reset_and_render(&canvas);
                            }
                        }
                    }
                />
                <div class:hidden=move || !unsupported.get() class="unsupported">
                    <div>
                        <h2>"WebGPU is not available in this browser."</h2>
                        <p>"Try the latest Chrome, Edge, or Firefox Nightly with WebGPU enabled."</p>
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

fn install_resize_listener(
    canvas_ref: NodeRef<Canvas>,
    renderer: Rc<RefCell<Option<CanvasRenderer>>>,
) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let closure = Closure::<dyn FnMut()>::new(move || {
        if let Some(canvas) = canvas_ref.get()
            && let Some(renderer) = renderer.borrow_mut().as_mut()
        {
            renderer.render(&canvas);
        }
    });
    if window
        .add_event_listener_with_callback("resize", closure.as_ref().unchecked_ref())
        .is_ok()
    {
        closure.forget();
    }
}
