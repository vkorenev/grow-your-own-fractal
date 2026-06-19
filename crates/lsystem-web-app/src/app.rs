use crate::export::{download_toml, export_png, export_svg};
use crate::presets::max_iterations_for_editor_config;
use crate::renderer::{CanvasRenderer, RenderStatus};
use leptos::html::{Canvas, Input};
use leptos::prelude::*;
use lsystem_app_model::{
    CleanMut, ColorControlMemory, ConfigDefaults, ConfigWorkspace, EditorLineColorConfig,
    EntryViewMut, HueRotation, HueRotationDirection, LineColorMode, ParseConfigError,
    advance_hue_rotation_phase_degrees, line_color_for_controls, load_presets,
    selected_line_color_mode,
};
use lsystem_core::{
    ColorConfig, Config, Dimensions, GenerationConfig, LineColorConfig, Rgb, contains_3d_symbols,
};
use lsystem_renderer::animation_export::AnimationParams;
use lsystem_renderer::line_renderer::FrameSkipReason;
use lsystem_renderer::png_export::{
    MAX_DIMENSION as PNG_MAX_DIMENSION, MIN_DIMENSION as PNG_MIN_DIMENSION,
};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

type RendererState = StoredValue<Option<CanvasRenderer>, LocalStorage>;

const ROTATION_STEP_DEG: f32 = 5.0;

#[component]
pub(crate) fn App() -> impl IntoView {
    let initial_workspace =
        ConfigWorkspace::from_presets(load_presets()).expect("bundled presets should parse");
    let selected_entry = initial_workspace.selected();
    let color_memory = RwSignal::new(ColorControlMemory::from_editor_config(
        &selected_entry.editor_config().colors,
        &ConfigDefaults::embedded().colors,
    ));
    let config_workspace = RwSignal::new(initial_workspace);
    let grammar_error = RwSignal::new(None::<String>);
    let toml_error = RwSignal::new(None::<String>);
    let workspace_error = RwSignal::new(None::<String>);
    let export_error = RwSignal::new(None::<String>);
    let animation_error = RwSignal::new(None::<String>);
    let colors_error = RwSignal::new(None::<String>);
    let png_width = RwSignal::new(800u32);
    let png_height = RwSignal::new(800u32);
    let anim_fps: RwSignal<u16> = RwSignal::new(30);
    let anim_duration_secs: RwSignal<f32> = RwSignal::new(4.0);
    let anim_progress: RwSignal<Option<(u32, u32)>> = RwSignal::new(None);
    let anim_exporting: RwSignal<bool> = RwSignal::new(false);
    let anim_num_frames =
        Memo::new(move |_| (anim_duration_secs.get() * anim_fps.get() as f32).round() as u32);
    let gpu_error = RwSignal::new(None::<String>);
    let auto_rotate = RwSignal::new(true);
    let auto_rotate_speed = RwSignal::new(20.0f32);
    let hue_rotation = RwSignal::new(HueRotation::default());
    let hue_rotation_phase = StoredValue::new(0.0f32);
    let sheet_open = RwSignal::new(false);
    let sheet_drag_start: StoredValue<Option<f64>, LocalStorage> = StoredValue::new_local(None);
    // Generation counter: bumped on each animation start so older rAF loops detect
    // they have been superseded and exit.
    let animation_token = RwSignal::new(0u32);
    on_cleanup(move || animation_token.update(|t| *t = t.wrapping_add(1)));

    let toml_text =
        Memo::new(move |_| config_workspace.with(|ws| ws.selected().draft_text().into_owned()));
    let editor_generation_config = Memo::new(move |_| {
        config_workspace.with(|ws| ws.selected().editor_config().generation.clone())
    });
    let unused_rule_symbols =
        Memo::new(move |_| editor_generation_config.with(|generation| generation.unused_rules()));
    let max_iterations =
        Memo::new(move |_| editor_generation_config.with(max_iterations_for_editor_config));
    let generation_config = Memo::new(move |_| {
        let max = max_iterations.get();
        editor_generation_config
            .with(|generation| generation.resolve(ConfigDefaults::embedded(), max))
    });
    let editor_color_config =
        Memo::new(move |_| config_workspace.with(|ws| ws.selected().editor_config().colors));
    let control_line_color = Memo::new(move |_| {
        editor_color_config
            .with(|editor| line_color_for_controls(editor, &ConfigDefaults::embedded().colors.line))
    });
    let color_config = Memo::new(move |_| {
        editor_color_config.with(|colors| colors.resolve(&ConfigDefaults::embedded().colors))
    });
    let iterations = Memo::new(move |_| {
        let max = max_iterations.get();
        editor_generation_config.with(|generation| generation.iterations.min(max))
    });
    let angle = Memo::new(move |_| editor_generation_config.with(|generation| generation.angle));

    let renderer: RendererState = StoredValue::new_local(None::<CanvasRenderer>);
    let active_pointers = StoredValue::new(std::collections::HashMap::<i32, (f64, f64)>::new());

    let is_3d = move || {
        matches!(
            editor_generation_config.get().dimensions,
            Dimensions::ThreeD
        )
    };
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
    let grammar_axiom = RwSignal::new(editor_generation_config.get_untracked().axiom.clone());
    let grammar_rules: RwSignal<Vec<(String, String)>> = RwSignal::new(rules_to_editor_rows(
        &editor_generation_config.get_untracked().rules,
    ));

    let save_format = RwSignal::new("png"); // "svg" | "png" | "apng"
    let effective_save_format = Memo::new(move |_| {
        let fmt = save_format.get();
        if is_3d() && fmt == "svg" { "png" } else { fmt }
    });

    let sync_grammar_editor = move || {
        let generation = editor_generation_config.get_untracked();
        grammar_axiom.set(generation.axiom);
        grammar_rules.set(rules_to_editor_rows(&generation.rules));
    };

    let canvas_ref = NodeRef::<Canvas>::new();
    let file_input_ref = NodeRef::<Input>::new();

    let config_for_render = move || Config {
        name: config_workspace.with_untracked(|ws| ws.selected().name().to_string()),
        generation: generation_config.get_untracked(),
        colors: color_config.get_untracked(),
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
            || hue_rotation.with(|m| m.is_active(&color_config.with(|c| c.line)))
    });

    let start_animation_loop = move || {
        animation_error.set(None);
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
                        hue_rotation.update(|m| m.stop());
                        hue_rotation_phase.set_value(0.0);
                        animation_error.set(Some(
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
                let rotation = hue_rotation.get_untracked();
                let rotation_active = rotation.is_active(&line_color);

                // Clamp dt to 100 ms to prevent a large jump after the tab was backgrounded.
                let dt = prev_ts
                    .map_or(1.0_f32 / 60.0, |p| ((ts - p) / 1000.0) as f32)
                    .min(1.0 / 10.0);
                prev_ts = Some(ts);

                let auto_degrees = auto_active.then(|| auto_rotate_speed.get_untracked() * dt);
                let hue_phase = rotation_active.then(|| {
                    let next = advance_hue_rotation_phase_degrees(
                        hue_rotation_phase.get_value(),
                        rotation.speed_degrees_per_second(),
                        dt,
                        rotation.direction(),
                    );
                    hue_rotation_phase.set_value(next);
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
        color_memory.set(ColorControlMemory::from_editor_config(
            &editor_color_config.get_untracked(),
            &ConfigDefaults::embedded().colors,
        ));
    };

    let reset_hue_rotation = move || {
        let was_active = hue_rotation.with_untracked(|m| m.is_enabled())
            || hue_rotation_phase.get_value() != 0.0;
        hue_rotation.update(|m| m.stop());
        hue_rotation_phase.set_value(0.0);
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
        grammar_error.set(None);
        toml_error.set(None);
        workspace_error.set(None);
        colors_error.set(None);
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
            Some(Err(msg)) => toml_error.set(Some(msg)),
            None => {
                log::error!("apply: config_workspace signal was unavailable");
                toml_error.set(Some("Internal error: could not apply config.".to_string()));
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
                toml_error.set(Some("Internal error: could not revert config.".to_string()));
            }
        }
    };

    let commit_rename = move || {
        let name = rename_draft.get_untracked().trim().to_string();
        let idx = config_workspace.with_untracked(|ws| ws.selected_index());
        match config_workspace.try_update(|ws| ws.rename(idx, &name)) {
            Some(Ok(())) => {
                workspace_error.set(None);
                rename_mode.set(false);
            }
            Some(Err(e)) => workspace_error.set(Some(e.to_string())),
            None => workspace_error.set(Some("Internal error: could not rename.".to_string())),
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
            Some(Err(msg)) => workspace_error.set(Some(msg)),
            None => workspace_error.set(Some("Internal error: could not reset.".to_string())),
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
        let generation = editor_generation_config.get();
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
                grammar_error.set(Some(
                    "Each rule must have a symbol. Remove or complete empty rows before applying."
                        .to_string(),
                ));
                return;
            };
            if !seen.insert(c) {
                grammar_error.set(Some(format!(
                    "Duplicate rule symbol '{c}'. Each symbol may appear only once."
                )));
                return;
            }
            rules.push((c, v));
        }
        if update_clean_config(
            config_workspace,
            grammar_error,
            "grammar apply",
            move |clean| clean.set_grammar(&axiom, &rules),
        ) {
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
                editor_generation_config.get_untracked().dimensions,
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
        // Defensive: "2d" button is disabled when grammar_has_3d_symbols(); guard against bypass.
        if next == Dimensions::TwoD && grammar_has_3d_symbols() {
            return;
        }
        update_clean_config(
            config_workspace,
            grammar_error,
            "set dimensions",
            move |clean| clean.set_dimensions(next),
        );
    };

    let apply_hue_rotation = move |dir: Option<HueRotationDirection>| match dir {
        None => reset_hue_rotation(),
        Some(d) => hue_rotation.update(|m| {
            m.set_direction(d);
            m.start();
        }),
    };

    let try_set_hue_rotation = move |key: &'static str| {
        let dir = match key {
            "forward" => Some(HueRotationDirection::Forward),
            "backward" => Some(HueRotationDirection::Reverse),
            _ => None,
        };
        // Forward/Backward buttons are disabled when not in Hue-cycle mode;
        // this guard is a defensive fallback in case disabled_keys is miscalculated.
        if dir.is_some()
            && !matches!(
                control_line_color.get_untracked(),
                LineColorConfig::HueCycle { .. }
            )
        {
            log::error!(
                "try_set_hue_rotation: direction set while not in HueCycle mode; \
                 disabled_keys guard may have been bypassed"
            );
            return;
        }
        apply_hue_rotation(dir);
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
                                    workspace_error.set(Some(err));
                                }
                                None => {
                                    log::error!(
                                        "select preset: config_workspace signal was unavailable"
                                    );
                                    workspace_error.set(Some(
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
                            <div class="btn-row">
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
                                <button type="button" on:click=move |_| rename_mode.set(false)>"Cancel"</button>
                            </div>
                        }.into_any()
                    } else {
                        view! {
                            <div style="display:contents">
                                <div class="btn-row">
                                    <button
                                        type="button"
                                        on:click=move |_| {
                                            let result = config_workspace.try_update(|workspace| {
                                                workspace.copy().map(|_| ()).map_err(|e| e.to_string())
                                            });
                                            match result {
                                                Some(Ok(())) => select_current_config(),
                                                Some(Err(msg)) => workspace_error.set(Some(msg)),
                                                None => {
                                                    log::error!("copy: config_workspace signal was unavailable");
                                                    workspace_error.set(Some(
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
                                <hr class="section-divider" />
                                <div class="btn-row">
                                    <button
                                        type="button"
                                        on:click=move |_| {
                                            if let Some(el) = file_input_ref.get_untracked() {
                                                el.click();
                                            } else {
                                                workspace_error.set(Some(
                                                    "Internal error: upload input unavailable.".to_string(),
                                                ));
                                            }
                                        }
                                    >"Open"</button>
                                    <button
                                        type="button"
                                        on:click=move |_| {
                                            workspace_error.set(None);
                                            let name = config_workspace
                                                .with_untracked(|ws| ws.selected().name().to_string());
                                            download_toml(&name, &toml_text.get_untracked());
                                        }
                                    >"Save"</button>
                                </div>
                            </div>
                        }.into_any()
                    }}
                </div>
                {move || workspace_error.get().map(|msg| view! {
                    <span class="inline-status error">{msg}</span>
                })}
                <input
                    type="file"
                    accept=".toml"
                    style="display:none"
                    node_ref=file_input_ref
                    on:change=move |_| {
                        let Some(input) = file_input_ref.get_untracked() else {
                            log::error!("import_toml on:change: file_input_ref was None");
                            workspace_error.set(Some(
                                "Internal error: upload input unavailable.".to_string(),
                            ));
                            return;
                        };
                        let Some(files) = input.files() else {
                            log::error!(
                                "import_toml on:change: input.files() returned None"
                            );
                            return;
                        };
                        let Some(file) = files.get(0) else { return };
                        let file = gloo_file::File::from(file);
                        wasm_bindgen_futures::spawn_local(async move {
                            match gloo_file::futures::read_as_text(&file).await {
                                Ok(text) => {
                                    let result = config_workspace
                                        .try_update(|ws| ws.import_toml(&text));
                                    match result {
                                        Some(Ok(_)) => select_current_config(),
                                        Some(Err(e)) => {
                                            workspace_error.set(Some(e.to_string()));
                                        }
                                        None => {
                                            log::error!(
                                                "import_toml: config_workspace signal was \
                                                 unavailable"
                                            );
                                            workspace_error.set(Some(
                                                "Internal error: could not import config."
                                                    .to_string(),
                                            ));
                                        }
                                    }
                                }
                                Err(e) => {
                                    workspace_error
                                        .set(Some(format!("Failed to read file: {e}")));
                                }
                            }
                            if let Some(input) = file_input_ref.get_untracked() {
                                input.set_value("");
                            }
                        });
                    }
                />

                <crate::ui::Disclosure title="Edit Config" open=false
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
                                toml_error.set(Some(
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
                    {move || toml_error.get().map(|msg| view! {
                        <span class="inline-status error">{msg}</span>
                    })}
                </crate::ui::Disclosure>

                <crate::ui::Disclosure title="L-System" badge=Signal::derive(grammar_is_dirty)>
                    <div style="display:flex;flex-direction:column;gap:5px">
                        <span class="section-label">"Dimensions"</span>
                        <div title=dirty_tooltip>
                        <crate::ui::SegmentedToggle
                            options=vec![("2d", "2D"), ("3d", "3D")]
                            selected=Signal::derive(move || match editor_generation_config.get().dimensions {
                                Dimensions::TwoD => "2d",
                                Dimensions::ThreeD => "3d",
                            })
                            on_change=move |key| try_set_dimensions(key)
                            disabled=Signal::derive(is_dirty)
                            disabled_keys=Signal::derive(move || {
                                if grammar_has_3d_symbols() { vec!["2d"] } else { vec![] }
                            })
                        />
                        </div>
                        <Show when=move || grammar_has_3d_symbols()>
                            <span class="inline-status warning">
                                "Grammar contains 3D-only symbols (& ^ / \\) — 2D mode is unavailable"
                            </span>
                        </Show>
                    </div>

                    <hr class="section-divider" />

                    <span class="section-label">"Grammar"</span>
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
                                "Contains 3D-only symbols — switch to 3D mode in Dimensions first"
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
                    {move || grammar_error.get().map(|msg| view! {
                        <span class="inline-status error">{msg}</span>
                    })}
                    {move || {
                        let unused = unused_rule_symbols.get();
                        (!unused.is_empty()).then(|| {
                            let symbols = unused.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(", ");
                            view! {
                                <span class="inline-status warning">
                                    {format!("Unused rules: {symbols} (never used during expansion)")}
                                </span>
                            }
                        })
                    }}

                    <hr class="section-divider" />

                    <span class="section-label">"Parameters"</span>
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
                                        config_workspace, grammar_error,"angle",
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
                                generation_config.with(|g| g.initial_heading)
                            ))
                            step=1.0
                            disabled=Signal::derive(is_dirty)
                            on_commit=move |s| {
                                if let Ok(v) = s.parse::<f32>() {
                                    update_clean_config(
                                        config_workspace, grammar_error,"initial_heading",
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
                                        config_workspace, grammar_error,"iterations",
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
                    <span class="section-label">"Background"</span>
                    <div class="color-row">
                        <label class="check-row" for="background-override">
                            <input
                                id="background-override"
                                type="checkbox"
                                prop:checked=move || {
                                    editor_color_config.with(|c| c.background.is_none())
                                }
                                disabled=is_dirty
                                on:change:target=move |ev| {
                                    let use_default = ev.target().checked();
                                    if use_default {
                                        let current_bg =
                                            editor_color_config.with_untracked(|editor| {
                                                editor.background.unwrap_or(
                                                    ConfigDefaults::embedded().colors.background,
                                                )
                                            });
                                        color_memory.update(|memory| {
                                            memory.remember_background(current_bg);
                                        });
                                    }
                                    let background = if use_default {
                                        None
                                    } else {
                                        Some(color_memory.get_untracked().background())
                                    };
                                    update_clean_config(
                                        config_workspace,
                                        colors_error,
                                        "background checkbox",
                                        move |clean| clean.set_background(background),
                                    );
                                }
                            />
                            <span>"Default"</span>
                        </label>
                        <input
                            id="background-color"
                            type="color"
                            prop:value=move || {
                                editor_color_config.with(|editor| {
                                    editor
                                        .background
                                        .unwrap_or(ConfigDefaults::embedded().colors.background)
                                        .to_string()
                                })
                            }
                            disabled=is_dirty
                            on:input:target=move |ev| {
                                let Ok(color) = ev.target().value().parse::<Rgb>() else {
                                    colors_error.set(Some("Invalid color value.".to_string()));
                                    return;
                                };
                                if update_clean_config(
                                    config_workspace,
                                    colors_error,
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

                    <hr class="section-divider" />

                    <span class="section-label">"Line"</span>
                    <div style="display:flex;align-items:center;gap:8px">
                    <label for="line-color-mode">"Style"</label>
                    <select
                        id="line-color-mode"
                        style="flex:1"
                        prop:value=move || {
                            editor_color_config
                                .with(|editor| {
                                    selected_line_color_mode(editor)
                                })
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
                            let editor_line =
                                editor_color_config.with_untracked(|e| e.line);
                            let Some(new_editor_line) = color_memory.try_update(|memory| {
                                memory.remember_line(editor_line);
                                memory.line_for(mode)
                            }) else {
                                log::error!(
                                    "line color mode select: line color memory signal was unavailable"
                                );
                                colors_error.set(Some(
                                    "Internal error: could not update line color.".to_string(),
                                ));
                                return;
                            };
                            if update_clean_config(
                                config_workspace,
                                colors_error,
                                "line color mode select",
                                move |clean| clean.set_line_color(new_editor_line),
                            ) {
                                color_memory.update(|memory| {
                                    memory.remember_line(new_editor_line);
                                });
                            }
                        }
                    >
                        <option value="solid">"Solid"</option>
                        <option value="gradient">"Gradient"</option>
                        <option value="hue_cycle">"Hue cycle"</option>
                    </select>
                    </div>

                    <div
                        class:hidden=move || {
                            !matches!(
                                control_line_color.get(),
                                LineColorConfig::Solid(_)
                            )
                        }
                        style="display:flex;flex-direction:column;gap:6px"
                    >
                        <div class="color-row">
                            <label class="check-row" for="line-solid-use-default">
                                <span>"Color"</span>
                                <input
                                    id="line-solid-use-default"
                                    type="checkbox"
                                    prop:checked=move || {
                                        editor_color_config.with(|c| c.line.is_none())
                                    }
                                    disabled=is_dirty
                                    on:change:target=move |ev| {
                                        let use_default = ev.target().checked();
                                        if use_default {
                                            let editor_line =
                                                editor_color_config.with_untracked(|e| e.line);
                                            color_memory.update(|m| m.remember_line(editor_line));
                                        }
                                        let line_color = if use_default {
                                            None
                                        } else {
                                            Some(EditorLineColorConfig::Solid(
                                                color_memory.get_untracked().solid_color(),
                                            ))
                                        };
                                        update_clean_config(
                                            config_workspace,
                                            colors_error,
                                            "solid use-default checkbox",
                                            move |clean| clean.set_line_color(line_color),
                                        );
                                    }
                                />
                                <span>"Default"</span>
                            </label>
                            <input
                                id="line-solid-color"
                                type="color"
                                prop:value=move || {
                                    solid_color_for_mode(control_line_color, color_memory)
                                    .to_string()
                                }
                                disabled=is_dirty
                                on:input:target=move |ev| {
                                    let Ok(color) = ev.target().value().parse::<Rgb>() else {
                                        colors_error.set(Some("Invalid color value.".to_string()));
                                        return;
                                    };
                                    let line_color = Some(EditorLineColorConfig::Solid(color));
                                    if update_clean_config(
                                        config_workspace,
                                        colors_error,
                                        "solid line color input",
                                        move |clean| clean.set_line_color(line_color),
                                    ) {
                                        color_memory.update(|memory| {
                                            memory.remember_line(Some(
                                                EditorLineColorConfig::Solid(color),
                                            ));
                                        });
                                    }
                                }
                            />
                        </div>
                    </div>

                    <div
                        class:hidden=move || {
                            !matches!(
                                control_line_color.get(),
                                LineColorConfig::Gradient { .. }
                            )
                        }
                        style="display:flex;flex-direction:column;gap:6px"
                    >
                        <div class="color-row">
                            <label class="check-row" for="line-gradient-start-use-default">
                                <span>"Start"</span>
                                <input
                                    id="line-gradient-start-use-default"
                                    type="checkbox"
                                    prop:checked=move || {
                                        editor_color_config.with(|c| {
                                            c.line.map(|l| l.gradient_fields()).unwrap_or_default().0.is_none()
                                        })
                                    }
                                    disabled=is_dirty
                                    on:change:target=move |ev| {
                                        let use_default = ev.target().checked();
                                        let editor_line =
                                            editor_color_config.with_untracked(|e| e.line);
                                        if use_default {
                                            color_memory.update(|m| m.remember_line(editor_line));
                                        }
                                        let (_, editor_end, editor_td) =
                                            editor_line.map(|l| l.gradient_fields()).unwrap_or_default();
                                        let start = if use_default {
                                            None
                                        } else {
                                            Some(color_memory.get_untracked().gradient_fields().0)
                                        };
                                        let line_color =
                                            Some(EditorLineColorConfig::Gradient {
                                                start,
                                                end: editor_end,
                                                topological_depth: editor_td,
                                            });
                                        update_clean_config(
                                            config_workspace,
                                            colors_error,
                                            "gradient start use-default",
                                            move |clean| clean.set_line_color(line_color),
                                        );
                                    }
                                />
                                <span>"Default"</span>
                            </label>
                            <input
                                id="line-gradient-start"
                                type="color"
                                prop:value=move || {
                                    let (start, _, _) = gradient_fields_for_mode(
                                        control_line_color,
                                        color_memory,
                                    );
                                    start.to_string()
                                }
                                disabled=is_dirty
                                on:input:target=move |ev| {
                                    let Ok(start) = ev.target().value().parse::<Rgb>() else {
                                        colors_error.set(Some("Invalid color value.".to_string()));
                                        return;
                                    };
                                    let editor_line =
                                        editor_color_config.get_untracked().line;
                                    let (_, editor_end, editor_td) =
                                        editor_line.map(|l| l.gradient_fields()).unwrap_or_default();
                                    let line_color =
                                        Some(EditorLineColorConfig::Gradient {
                                            start: Some(start),
                                            end: editor_end,
                                            topological_depth: editor_td,
                                        });
                                    if update_clean_config(
                                        config_workspace,
                                        colors_error,
                                        "gradient start color input",
                                        move |clean| clean.set_line_color(line_color),
                                    ) {
                                        color_memory.update(|memory| {
                                            memory.remember_line(line_color);
                                        });
                                    }
                                }
                            />
                        </div>

                        <div class="color-row">
                            <label class="check-row" for="line-gradient-end-use-default">
                                <span>"End"</span>
                                <input
                                    id="line-gradient-end-use-default"
                                    type="checkbox"
                                    prop:checked=move || {
                                        editor_color_config.with(|c| {
                                            c.line.map(|l| l.gradient_fields()).unwrap_or_default().1.is_none()
                                        })
                                    }
                                    disabled=is_dirty
                                    on:change:target=move |ev| {
                                        let use_default = ev.target().checked();
                                        let editor_line =
                                            editor_color_config.with_untracked(|e| e.line);
                                        if use_default {
                                            color_memory.update(|m| m.remember_line(editor_line));
                                        }
                                        let (editor_start, _, editor_td) =
                                            editor_line.map(|l| l.gradient_fields()).unwrap_or_default();
                                        let end = if use_default {
                                            None
                                        } else {
                                            Some(color_memory.get_untracked().gradient_fields().1)
                                        };
                                        let line_color =
                                            Some(EditorLineColorConfig::Gradient {
                                                start: editor_start,
                                                end,
                                                topological_depth: editor_td,
                                            });
                                        update_clean_config(
                                            config_workspace,
                                            colors_error,
                                            "gradient end use-default",
                                            move |clean| clean.set_line_color(line_color),
                                        );
                                    }
                                />
                                <span>"Default"</span>
                            </label>
                            <input
                                id="line-gradient-end"
                                type="color"
                                prop:value=move || {
                                    let (_, end, _) = gradient_fields_for_mode(
                                        control_line_color,
                                        color_memory,
                                    );
                                    end.to_string()
                                }
                                disabled=is_dirty
                                on:input:target=move |ev| {
                                    let Ok(end) = ev.target().value().parse::<Rgb>() else {
                                        colors_error.set(Some("Invalid color value.".to_string()));
                                        return;
                                    };
                                    let editor_line =
                                        editor_color_config.get_untracked().line;
                                    let (editor_start, _, editor_td) =
                                        editor_line.map(|l| l.gradient_fields()).unwrap_or_default();
                                    let line_color =
                                        Some(EditorLineColorConfig::Gradient {
                                            start: editor_start,
                                            end: Some(end),
                                            topological_depth: editor_td,
                                        });
                                    if update_clean_config(
                                        config_workspace,
                                        colors_error,
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

                        <label class="check-row" for="line-gradient-topological-depth">
                            <input
                                id="line-gradient-topological-depth"
                                type="checkbox"
                                prop:checked=move || {
                                    let (_, _, topological_depth) = gradient_fields_for_mode(
                                        control_line_color,
                                        color_memory,
                                    );
                                    topological_depth
                                }
                                disabled=is_dirty
                                on:change:target=move |ev| {
                                    let editor_line = editor_color_config.get_untracked().line;
                                    let (editor_start, editor_end, _) =
                                        editor_line.map(|l| l.gradient_fields()).unwrap_or_default();
                                    let checked = ev.target().checked();
                                    let line_color = Some(EditorLineColorConfig::Gradient {
                                        start: editor_start,
                                        end: editor_end,
                                        topological_depth: Some(checked),
                                    });
                                    if update_clean_config(
                                        config_workspace,
                                        colors_error,
                                        "gradient topological depth toggle",
                                        move |clean| clean.set_line_color(line_color),
                                    ) {
                                        color_memory.update(|memory| {
                                            memory.remember_line(line_color);
                                        });
                                    }
                                }
                            />
                            <span>"Color by topological depth"</span>
                        </label>
                    </div>

                    <div
                        class:hidden=move || {
                            !matches!(
                                control_line_color.get(),
                                LineColorConfig::HueCycle { .. }
                            )
                        }
                        style="display:flex;flex-direction:column;gap:6px"
                    >
                        <div class="color-row">
                            <label class="check-row" for="line-hue-cycle-use-default">
                                <span>"Initial"</span>
                                <input
                                    id="line-hue-cycle-use-default"
                                    type="checkbox"
                                    prop:checked=move || {
                                        matches!(
                                            editor_color_config.with(|c| c.line),
                                            None | Some(EditorLineColorConfig::HueCycle {
                                                initial: None
                                            })
                                        )
                                    }
                                    disabled=is_dirty
                                    on:change:target=move |ev| {
                                        let use_default = ev.target().checked();
                                        if use_default {
                                            let editor_line =
                                                editor_color_config.with_untracked(|e| e.line);
                                            color_memory.update(|m| m.remember_line(editor_line));
                                        }
                                        let line_color = if use_default {
                                            Some(EditorLineColorConfig::HueCycle { initial: None })
                                        } else {
                                            Some(EditorLineColorConfig::HueCycle {
                                                initial: Some(
                                                    color_memory.get_untracked().hue_cycle_initial(),
                                                ),
                                            })
                                        };
                                        update_clean_config(
                                            config_workspace,
                                            colors_error,
                                            "hue-cycle use-default checkbox",
                                            move |clean| clean.set_line_color(line_color),
                                        );
                                    }
                                />
                                <span>"Default"</span>
                            </label>
                            <input
                                id="line-hue-cycle-initial"
                                type="color"
                                prop:value=move || {
                                    hue_cycle_initial_for_mode(control_line_color, color_memory)
                                    .to_string()
                                }
                                disabled=is_dirty
                                on:input:target=move |ev| {
                                    let Ok(initial) = ev.target().value().parse::<Rgb>() else {
                                        colors_error.set(Some("Invalid color value.".to_string()));
                                        return;
                                    };
                                    let line_color =
                                        Some(EditorLineColorConfig::HueCycle { initial: Some(initial) });
                                    if update_clean_config(
                                        config_workspace,
                                        colors_error,
                                        "hue-cycle initial color input",
                                        move |clean| clean.set_line_color(line_color),
                                    ) {
                                        color_memory.update(|memory| {
                                            memory.remember_line(Some(
                                                EditorLineColorConfig::HueCycle {
                                                    initial: Some(initial),
                                                },
                                            ));
                                        });
                                    }
                                }
                            />
                        </div>
                    </div>
                    </div>
                    {move || colors_error.get().map(|msg| view! {
                        <span class="inline-status error">{msg}</span>
                    })}
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
                        <Show when=move || !is_3d()>
                            <span class="inline-status warning">"Switch to 3D mode to enable auto-rotate"</span>
                        </Show>
                    </div>

                    <hr class="section-divider" />

                    <div style="display:flex;flex-direction:column;gap:6px">
                        <span class="section-label">"Hue rotation"</span>
                        <div title=move || if !matches!(control_line_color.get(), LineColorConfig::HueCycle { .. }) { "Select Hue cycle in Colors to enable hue rotation" } else { "" }>
                        <crate::ui::SegmentedToggle
                            options=vec![("off", "Off"), ("forward", "Forward"), ("backward", "Backward")]
                            selected=Signal::derive(move || {
                                hue_rotation.with(|m| {
                                    if !m.is_enabled() { "off" }
                                    else if m.direction() == HueRotationDirection::Forward { "forward" }
                                    else { "backward" }
                                })
                            })
                            on_change=move |key| try_set_hue_rotation(key)
                            disabled_keys=Signal::derive(move || {
                                if matches!(control_line_color.get(), LineColorConfig::HueCycle { .. }) {
                                    vec![]
                                } else {
                                    vec!["forward", "backward"]
                                }
                            })
                        />
                        </div>
                        <Show when=move || hue_rotation.with(|m| m.is_enabled())>
                            <div class="spinner-row">
                                <span class="spinner-label">"Speed (°/s)"</span>
                                <crate::ui::Spinner
                                    value=Signal::derive(move || hue_rotation.with(|m| format!("{:.0}", m.speed_degrees_per_second())))
                                    step=1.0
                                    on_commit=move |s| {
                                        if let Ok(v) = s.parse::<f32>() {
                                            hue_rotation.update(|m| m.set_speed(v));
                                        }
                                    }
                                />
                            </div>
                        </Show>
                        <Show when=move || !matches!(control_line_color.get(), LineColorConfig::HueCycle { .. })>
                            <span class="inline-status warning">"Select Hue cycle in Colors to enable hue rotation"</span>
                        </Show>
                    </div>
                    {move || animation_error.get().map(|msg| view! {
                        <span class="inline-status error">{msg}</span>
                    })}
                </crate::ui::Disclosure>

                <crate::ui::Disclosure title="Save image" open=false>
                    {move || if is_3d() {
                        view! {
                            <crate::ui::SegmentedToggle
                                options=vec![("png", "PNG"), ("apng", "APNG")]
                                selected=Signal::derive(move || effective_save_format.get())
                                on_change=move |key| {
                                    save_format.set(key);
                                    export_error.set(None);
                                }
                            />
                        }.into_any()
                    } else {
                        view! {
                            <crate::ui::SegmentedToggle
                                options=vec![("svg", "SVG"), ("png", "PNG"), ("apng", "APNG")]
                                selected=Signal::derive(move || effective_save_format.get())
                                on_change=move |key| {
                                    save_format.set(key);
                                    export_error.set(None);
                                }
                            />
                        }.into_any()
                    }}

                    <Show when=move || effective_save_format.get() != "svg">
                        <div class="spinner-row">
                            <span class="spinner-label">"Width (px)"</span>
                            <crate::ui::Spinner
                                value=Signal::derive(move || png_width.get().to_string())
                                step=16.0
                                on_commit=move |s| {
                                    if let Ok(v) = s.parse::<u32>() {
                                        png_width.set(v.clamp(PNG_MIN_DIMENSION, PNG_MAX_DIMENSION));
                                    }
                                }
                            />
                        </div>
                        <div class="spinner-row">
                            <span class="spinner-label">"Height (px)"</span>
                            <crate::ui::Spinner
                                value=Signal::derive(move || png_height.get().to_string())
                                step=16.0
                                on_commit=move |s| {
                                    if let Ok(v) = s.parse::<u32>() {
                                        png_height.set(v.clamp(PNG_MIN_DIMENSION, PNG_MAX_DIMENSION));
                                    }
                                }
                            />
                        </div>
                    </Show>

                    <Show when=move || effective_save_format.get() == "apng">
                        <div class="spinner-row">
                            <span class="spinner-label">"FPS"</span>
                            <select
                                on:change=move |ev| {
                                    if let Ok(v) = event_target_value(&ev).parse::<u16>() {
                                        anim_fps.set(v);
                                    }
                                }
                                prop:value=move || anim_fps.get().to_string()
                            >
                                <option value="12">"12"</option>
                                <option value="24">"24"</option>
                                <option value="30">"30"</option>
                                <option value="60">"60"</option>
                            </select>
                        </div>

                        <div class="spinner-row">
                            <span class="spinner-label">"Duration (s)"</span>
                            <crate::ui::Spinner
                                value=Signal::derive(move || format!("{:.1}", anim_duration_secs.get()))
                                step=1.0
                                on_commit=move |s| {
                                    if let Ok(v) = s.parse::<f32>() {
                                        anim_duration_secs.set(v.max(0.1));
                                    }
                                }
                            />
                        </div>

                        <Show when=move || hue_rotation.with(|m| m.is_enabled())>
                            {move || {
                                let speed = hue_rotation.with(|m| m.speed_degrees_per_second());
                                let loop_secs = 360.0 / speed;
                                view! {
                                    <button
                                        type="button"
                                        on:click=move |_| anim_duration_secs.set(loop_secs)
                                    >
                                        {format!("Hue loop ({loop_secs:.1}s)")}
                                    </button>
                                }
                            }}
                        </Show>

                        <Show when=move || auto_rotate.get() && is_3d()>
                            {move || {
                                let speed = auto_rotate_speed.get();
                                let loop_secs = 360.0 / speed;
                                view! {
                                    <button
                                        type="button"
                                        on:click=move |_| anim_duration_secs.set(loop_secs)
                                    >
                                        {format!("Orbit loop ({loop_secs:.1}s)")}
                                    </button>
                                }
                            }}
                        </Show>

                        <span class="section-label" style="color:var(--color-muted)">
                            {move || format!("{} frames", anim_num_frames.get())}
                        </span>

                        {move || anim_progress.get().map(|(n, total)| view! {
                            <span class="inline-status">{format!("Exporting frame {n} / {total}…")}</span>
                        })}
                    </Show>

                    <Show when=move || effective_save_format.get() == "apng" && (anim_num_frames.get() > AnimationParams::MAX_FRAMES)>
                        <span class="inline-status warning">
                            {move || format!(
                                "Duration produces {} frames; max is {}.",
                                anim_num_frames.get(),
                                AnimationParams::MAX_FRAMES,
                            )}
                        </span>
                    </Show>

                    <button
                        type="button"
                        disabled=move || effective_save_format.get() == "apng" && (anim_exporting.get() || anim_num_frames.get() > AnimationParams::MAX_FRAMES)
                        on:click=move |_| {
                            export_error.set(None);
                            let fmt = effective_save_format.get_untracked();
                            let config = config_for_render();
                            if fmt == "svg" {
                                export_svg(config);
                            } else if fmt == "png" {
                                let Some(Some((device, queue, camera))) =
                                    renderer.try_with_value(|opt| {
                                        opt.as_ref().map(|r| {
                                            let (d, q) = r.device_queue();
                                            (d, q, r.camera())
                                        })
                                    })
                                else {
                                    export_error.set(Some("Cannot save: GPU renderer not ready.".to_string()));
                                    return;
                                };
                                export_png(
                                    device,
                                    queue,
                                    camera,
                                    config,
                                    png_width.get_untracked(),
                                    png_height.get_untracked(),
                                    move |e| export_error.set(Some(e)),
                                );
                            } else {
                                let Some(Some((device, queue, camera))) =
                                    renderer.try_with_value(|opt| {
                                        opt.as_ref().map(|r| {
                                            let (d, q) = r.device_queue();
                                            (d, q, r.camera())
                                        })
                                    })
                                else {
                                    export_error.set(Some("Cannot save: GPU renderer not ready.".to_string()));
                                    return;
                                };
                                let fps = anim_fps.get_untracked();
                                let num_frames = anim_num_frames.get_untracked();
                                let initial_hue = hue_rotation_phase.get_value();
                                let hue_rotation_dps = hue_rotation.with_untracked(|m| {
                                    if m.is_enabled() {
                                        let sign = if m.direction() == HueRotationDirection::Forward { 1.0f32 } else { -1.0 };
                                        sign * m.speed_degrees_per_second()
                                    } else {
                                        0.0
                                    }
                                });
                                let auto_rotate_dps = if auto_rotate.get_untracked() && is_3d() {
                                    auto_rotate_speed.get_untracked()
                                } else {
                                    0.0
                                };
                                let params = AnimationParams {
                                    fps,
                                    num_frames,
                                    initial_hue_phase_degrees: initial_hue,
                                    hue_rotation_dps,
                                    auto_rotate_dps,
                                };
                                let width = png_width.get_untracked();
                                let height = png_height.get_untracked();
                                anim_exporting.set(true);
                                anim_progress.set(None);
                                crate::export::export_animation(
                                    device,
                                    queue,
                                    camera,
                                    config,
                                    width,
                                    height,
                                    params,
                                    move |n, total| anim_progress.set(Some((n, total))),
                                    move |err| {
                                        anim_exporting.set(false);
                                        anim_progress.set(None);
                                        if let Some(msg) = err {
                                            export_error.set(Some(msg));
                                        }
                                    },
                                );
                            }
                        }
                    >"Save"</button>
                    {move || export_error.get().map(|m| view! {
                        <span class="inline-status error">{m}</span>
                    })}
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
                        }
                        let id = ev.pointer_id();
                        active_pointers.update_value(|map| {
                            if map.len() >= 2 && !map.contains_key(&id) {
                                map.clear();
                            }
                            map.insert(id, (ev.client_x() as f64, ev.client_y() as f64));
                        });
                        if let Some(canvas) = canvas_ref.get_untracked() {
                            let _ = canvas.set_pointer_capture(id);
                        }
                    }
                    on:pointermove=move |ev: web_sys::PointerEvent| {
                        let x = ev.client_x() as f64;
                        let y = ev.client_y() as f64;
                        let id = ev.pointer_id();

                        let (prev, other, len) = active_pointers.with_value(|map| {
                            let prev = map.get(&id).copied();
                            let other = map.iter().find(|&(&k, _)| k != id).map(|(_, &v)| v);
                            (prev, other, map.len())
                        });

                        let Some((prev_x, prev_y)) = prev else { return; };

                        active_pointers.update_value(|map| { map.insert(id, (x, y)); });

                        let Some(canvas) = canvas_ref.get_untracked() else { return; };

                        if len == 1 {
                            let dx = x - prev_x;
                            let dy = y - prev_y;
                            with_renderer(canvas, renderer, recover_after_render,
                                |r, c| r.drag_and_render(c, dx as f32, dy as f32));
                        } else if let Some((ox, oy)) = other {
                            let prev_dist = ((prev_x - ox).powi(2) + (prev_y - oy).powi(2)).sqrt();
                            if prev_dist >= 1.0 {
                                let new_dist = ((x - ox).powi(2) + (y - oy).powi(2)).sqrt();
                                let factor = (new_dist / prev_dist) as f32;
                                let mid_x = ((x + ox) / 2.0) as f32;
                                let mid_y = ((y + oy) / 2.0) as f32;
                                with_renderer(canvas, renderer, recover_after_render,
                                    |r, c| r.zoom_by_factor_and_render(c, factor, mid_x, mid_y));
                            }
                        }
                    }
                    on:pointerup=move |ev: web_sys::PointerEvent| {
                        if let Some(canvas) = canvas_ref.get_untracked() {
                            let _ = canvas.release_pointer_capture(ev.pointer_id());
                        }
                        active_pointers.update_value(|map| { map.remove(&ev.pointer_id()); });
                    }
                    on:pointercancel=move |ev: web_sys::PointerEvent| {
                        active_pointers.update_value(|map| { map.remove(&ev.pointer_id()); });
                    }
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
    grammar_error: RwSignal<Option<String>>,
    event: &'static str,
    update: impl FnOnce(&mut CleanMut<'_>) -> Result<(), ParseConfigError>,
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
            grammar_error.set(None);
            true
        }
        Some(Ok(false)) => false,
        Some(Err(msg)) => {
            grammar_error.set(Some(msg));
            false
        }
        None => {
            log::error!("{event}: config_workspace signal was unavailable");
            grammar_error.set(Some("Internal error: could not update config.".to_string()));
            false
        }
    }
}

fn solid_color_for_mode(
    line_color: Memo<LineColorConfig>,
    memory: RwSignal<ColorControlMemory>,
) -> Rgb {
    match line_color.get() {
        LineColorConfig::Solid(color) => color,
        _ => memory.with(|m| m.solid_color()),
    }
}

fn gradient_fields_from(line: LineColorConfig, fallback: (Rgb, Rgb, bool)) -> (Rgb, Rgb, bool) {
    match line {
        LineColorConfig::Gradient {
            start,
            end,
            topological_depth,
        } => (start, end, topological_depth),
        _ => fallback,
    }
}

fn gradient_fields_for_mode(
    line_color: Memo<LineColorConfig>,
    memory: RwSignal<ColorControlMemory>,
) -> (Rgb, Rgb, bool) {
    gradient_fields_from(line_color.get(), memory.with(|m| m.gradient_fields()))
}

fn hue_cycle_initial_for_mode(
    line_color: Memo<LineColorConfig>,
    memory: RwSignal<ColorControlMemory>,
) -> Rgb {
    match line_color.get() {
        LineColorConfig::HueCycle { initial } => initial,
        _ => memory.with(|m| m.hue_cycle_initial()),
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
