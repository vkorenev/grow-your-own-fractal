use crate::panels::grammar::GrammarRow;
use crate::presets::max_iterations_for_editor_config;
use crate::renderer::{CanvasRenderer, RebuildRenderOutcome, RenderStatus};
use glam::{DVec2, Vec2};
use leptos::html::Canvas;
use leptos::prelude::*;
use lsystem_app_model::{
    CAMERA_AUTO_ROTATION_DEFAULT_SPEED_DEGREES_PER_SECOND, CleanMut, ColorControlMemory,
    ConfigDefaults, ConfigEntryId, ConfigWorkspace, EditorColorConfig, EntryViewMut, HueRotation,
    HueRotationDirection, ParseConfigError, advance_hue_rotation_phase_degrees,
    line_color_for_controls, load_presets,
};
use lsystem_core::{Config, Dimensions, GenerationConfig, LineColorConfig, contains_3d_symbols};
use lsystem_renderer::scene_upload::SceneUploadError;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

pub(crate) type RendererState = StoredValue<Option<CanvasRenderer>, LocalStorage>;

const ROTATION_STEP_DEG: f32 = 5.0;

#[derive(Clone)]
enum ViewportError {
    RendererUnavailable(String),
    SceneUpload(SceneUploadError),
}

impl ViewportError {
    fn title(&self) -> &'static str {
        match self {
            Self::RendererUnavailable(_) => "GPU renderer unavailable",
            Self::SceneUpload(_) => "Unable to render fractal",
        }
    }

    fn message(&self) -> String {
        match self {
            Self::RendererUnavailable(message) => message.clone(),
            Self::SceneUpload(SceneUploadError::SegmentLimitExceeded {
                total_segments,
                limit,
            }) => format!(
                "This fractal generated {total_segments} segments, but this device can display at \
                 most {limit}. Reduce the iteration count or simplify the grammar."
            ),
            Self::SceneUpload(SceneUploadError::StagingUnavailable) => {
                "GPU staging memory could not be allocated. Reduce the iteration count or retry."
                    .to_string()
            }
        }
    }
}

/// Grammar-editor draft state. The draft signals live in `App` (not in the
/// grammar panel) because entry switches, a successful raw TOML apply, and a
/// bundled-preset reset all need to resync them from one place, while a raw
/// TOML *revert* must leave them untouched — see
/// `crate::app::ConfigContext::clear_toml_revert_state`.
#[derive(Clone, Copy)]
pub(crate) struct GrammarDraft {
    pub(crate) axiom: RwSignal<String>,
    pub(crate) rows: RwSignal<Vec<GrammarRow>>,
    pub(crate) row_counter: StoredValue<u32>,
    pub(crate) is_dirty: Memo<bool>,
    pub(crate) has_3d_symbols: Memo<bool>,
    pub(crate) symbols: Memo<Vec<char>>,
    /// Resets the draft to the currently applied grammar.
    pub(crate) sync: Callback<()>,
}

/// Workspace/config state shared between `App` and the control panels,
/// provided via context.
#[derive(Clone, Copy)]
pub(crate) struct ConfigContext {
    pub(crate) config_workspace: RwSignal<ConfigWorkspace>,
    pub(crate) toml_text: Memo<String>,
    pub(crate) selected_id: Memo<ConfigEntryId>,
    pub(crate) selected_name: Memo<String>,
    pub(crate) display_options: Memo<Vec<(ConfigEntryId, String)>>,
    pub(crate) differs_from_default: Memo<bool>,
    pub(crate) generation_config: Memo<GenerationConfig>,
    pub(crate) editor_color_config: Memo<EditorColorConfig>,
    pub(crate) control_line_color: Memo<LineColorConfig>,
    pub(crate) color_memory: RwSignal<ColorControlMemory>,
    pub(crate) unused_rule_symbols: Memo<Vec<char>>,
    pub(crate) iterations: Memo<u16>,
    pub(crate) max_iterations: Memo<u16>,
    pub(crate) angle: Memo<f32>,
    pub(crate) dimensions: Memo<Dimensions>,
    pub(crate) is_3d: Memo<bool>,
    pub(crate) is_dirty: Memo<bool>,
    pub(crate) grammar: GrammarDraft,
    pub(crate) grammar_error: RwSignal<Option<String>>,
    pub(crate) toml_error: RwSignal<Option<String>>,
    pub(crate) workspace_error: RwSignal<Option<String>>,
    pub(crate) colors_error: RwSignal<Option<String>>,
    /// Clears panel errors and resyncs editors after the selected entry (or its
    /// applied config) changes.
    pub(crate) select_current_config: Callback<()>,
    /// Clears every panel error except `grammar_error`, and refreshes color
    /// memory, without touching the grammar draft. Used only after
    /// reverting the raw TOML draft: reverting doesn't change the applied
    /// document, so a pending grammar draft (and any error describing it)
    /// is still valid — see the "Platform variant" grammar behavior in
    /// docs/specs/application-workspace.md. `select_current_config` above is
    /// for the cases that *do* change the applied document (entry switches,
    /// a successful apply, a reset) and must resync the grammar draft too.
    pub(crate) clear_toml_revert_state: Callback<()>,
}

/// Renderer and animation state shared between `App` and the control panels,
/// provided via context.
#[derive(Clone, Copy)]
pub(crate) struct RenderContext {
    pub(crate) renderer: RendererState,
    pub(crate) auto_rotate: RwSignal<bool>,
    pub(crate) auto_rotate_speed: RwSignal<f32>,
    pub(crate) hue_rotation: RwSignal<HueRotation>,
    pub(crate) hue_rotation_phase: StoredValue<f32>,
    pub(crate) animation_error: RwSignal<Option<String>>,
    /// Snapshot of the currently applied config for rendering/export.
    pub(crate) config_for_render: Callback<(), Config>,
    pub(crate) set_hue_rotation: Callback<Option<HueRotationDirection>>,
}

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
    let animation_error = RwSignal::new(None::<String>);
    let colors_error = RwSignal::new(None::<String>);
    let viewport_error = RwSignal::new(None::<ViewportError>);
    let auto_rotate = RwSignal::new(true);
    let auto_rotate_speed = RwSignal::new(CAMERA_AUTO_ROTATION_DEFAULT_SPEED_DEGREES_PER_SECOND);
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
    let selected_id = Memo::new(move |_| config_workspace.with(|ws| ws.selected_id()));
    let selected_name =
        Memo::new(move |_| config_workspace.with(|ws| ws.selected().name().to_string()));
    let display_options = Memo::new(move |_| config_workspace.with(|ws| ws.display_options()));
    let differs_from_default =
        Memo::new(move |_| config_workspace.with(|ws| ws.selected().differs_from_default()));
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
    let active_pointers = StoredValue::new(std::collections::HashMap::<i32, DVec2>::new());

    let dimensions = Memo::new(move |_| editor_generation_config.with(|g| g.dimensions));
    let is_3d = Memo::new(move |_| matches!(dimensions.get(), Dimensions::ThreeD));
    let is_dirty = Memo::new(move |_| config_workspace.with(|ws| ws.selected().is_dirty()));

    let grammar_axiom = RwSignal::new(editor_generation_config.get_untracked().axiom.clone());
    let grammar_row_counter = StoredValue::new(0u32);
    let grammar_rows = RwSignal::new(crate::panels::grammar::rows_from_rules(
        &editor_generation_config.get_untracked().rules,
        grammar_row_counter,
    ));

    let sync_grammar_editor = move || {
        let generation = editor_generation_config.get_untracked();
        grammar_axiom.set(generation.axiom);
        grammar_rows.set(crate::panels::grammar::rows_from_rules(
            &generation.rules,
            grammar_row_counter,
        ));
    };

    let canvas_ref = NodeRef::<Canvas>::new();

    let config_for_render = move || Config {
        name: selected_name.get_untracked(),
        generation: generation_config.get_untracked(),
        colors: color_config.get_untracked(),
    };

    let recover_after_render = move |status: RenderStatus, canvas: web_sys::HtmlCanvasElement| {
        if let Some(reason) = status.unexpected_skip_reason() {
            log::error!("Skipped GPU frame: {reason}");
        }
        if status != RenderStatus::SurfaceLost {
            return;
        }
        log::error!("GPU surface was lost; attempting to recreate it");
        wasm_bindgen_futures::spawn_local(async move {
            let Some(Some(mut renderer_state)) = renderer.try_update_value(|opt| opt.take()) else {
                return;
            };
            match renderer_state.recover_surface(canvas.clone()).await {
                Ok(()) => {
                    let config = config_for_render();
                    let outcome =
                        renderer_state.set_config_preserving_camera_and_render(&canvas, &config);
                    if let Some(reason) = outcome.render_status.unexpected_skip_reason() {
                        log::error!("Skipped GPU frame after surface recovery: {reason}");
                    }
                    if outcome.render_status == RenderStatus::SurfaceLost {
                        log::error!("GPU surface was lost again after recovery");
                        viewport_error.set(Some(ViewportError::RendererUnavailable(
                            "GPU surface was lost again after recovery.".to_string(),
                        )));
                    } else {
                        update_viewport_error(viewport_error, outcome.rebuild_result);
                        renderer.update_value(|opt| *opt = Some(renderer_state));
                    }
                }
                Err(err) => {
                    log::error!("Failed to recover GPU surface: {err}");
                    viewport_error.set(Some(ViewportError::RendererUnavailable(err.to_string())));
                }
            }
        });
    };

    let animation_active = Memo::new(move |_| {
        (auto_rotate.get() && is_3d.get())
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
                    && is_3d.get_untracked()
                    && active_pointers.with_value(|map| map.is_empty());
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

                if auto_degrees.is_none() && hue_phase.is_none() {
                    continue;
                }

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
                    renderer.update_value(|opt| *opt = Some(new_renderer));
                    let config = config_for_render();
                    with_renderer_for_rebuild(
                        canvas,
                        renderer,
                        recover_after_render,
                        viewport_error,
                        |r, c| r.set_config_and_render(c, &config),
                    );
                    if animation_active.get_untracked() {
                        start_animation_loop();
                    }
                }
                Err(err) => {
                    log::error!("Failed to initialize GPU renderer: {err}");
                    viewport_error.set(Some(ViewportError::RendererUnavailable(err.to_string())));
                }
            }
        });
    });

    let resize_handle = window_event_listener(leptos::ev::resize, move |_| {
        if let Some(canvas) = canvas_ref.get_untracked() {
            with_renderer(canvas, renderer, recover_after_render, |r, c| r.render(c));
        }
    });
    on_cleanup(move || resize_handle.remove());

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

    // The handler first runs on the first change after mount (immediate = false),
    // receiving prev = Some(initial deps); the initial render is done by
    // canvas_ref.on_load.
    Effect::watch(
        move || (generation_config.get(), color_config.get()),
        move |current, prev, _: Option<()>| {
            let Some(canvas) = canvas_ref.get_untracked() else {
                return;
            };
            let generation_changed = prev.is_none_or(|p| p.0 != current.0);
            let config = config_for_render();
            if generation_changed {
                with_renderer_for_rebuild(
                    canvas,
                    renderer,
                    recover_after_render,
                    viewport_error,
                    |r, c| r.set_config_and_render(c, &config),
                );
            } else {
                with_renderer(canvas, renderer, recover_after_render, |r, c| {
                    r.set_colors_and_render(c, &config)
                });
            }
        },
        false,
    );

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

    // Narrower than `select_current_config`: reverting the raw TOML draft
    // doesn't change the applied document, so `grammar_error` and the
    // grammar draft itself (owned by `App`, not touched here) must
    // survive. `refresh_color_memory` is kept here unchanged from today's
    // behavior: Revert already calls it whenever the entry is dirty and
    // the grammar draft is clean, so this task doesn't change its
    // reachability, only extends it to also cover a pending grammar draft.
    // Note for a future PR: `ColorControlMemory::from_editor_config`
    // rebuilds memory for only the currently-authored line-color mode, so
    // calling it here can drop a remembered override for a different mode —
    // that's a pre-existing lossiness in `refresh_color_memory` itself, not
    // introduced by this task, and out of scope to fix here.
    let clear_toml_revert_state = move || {
        toml_error.set(None);
        workspace_error.set(None);
        colors_error.set(None);
        refresh_color_memory();
    };

    // Resync editor panels whenever a different entry becomes selected (select,
    // copy, import). Same-id mutations that change the applied document
    // (apply, reset) call select_current_config() explicitly; keying this
    // watch off the applied config instead would clobber in-progress
    // grammar drafts on unrelated parameter edits. Reverting the raw TOML
    // draft is a same-id mutation too, but doesn't change the applied
    // document — it calls the narrower clear_toml_revert_state() instead,
    // which leaves a pending grammar draft untouched.
    Effect::watch(
        move || selected_id.get(),
        move |_, _, _: Option<()>| select_current_config(),
        false,
    );

    // Clear a stale "TOML Apply failed" message once the draft it described
    // is gone. TOML Apply/Revert already clear `toml_error` themselves, but a
    // direct control elsewhere (colors, grammar, dimensions) can also discard
    // the pending draft via `update_clean_config`, which doesn't own this
    // panel-specific signal. Watching the dirty→clean transition covers both,
    // instead of threading `toml_error` through every direct-control call
    // site.
    Effect::watch(
        move || is_dirty.get(),
        move |current: &bool, prev: Option<&bool>, _: Option<()>| {
            if prev == Some(&true) && !current {
                toml_error.set(None);
            }
        },
        false,
    );

    let grammar_has_3d_symbols = Memo::new(move |_| {
        grammar_axiom.with(|a| contains_3d_symbols(a))
            || grammar_rows.with(|rows| {
                rows.iter()
                    .any(|row| row.rhs.with(|rhs| contains_3d_symbols(rhs)))
            })
    });

    let applied_grammar_rows =
        Memo::new(move |_| editor_generation_config.with(|g| rules_to_editor_rows(&g.rules)));
    let grammar_is_dirty = Memo::new(move |_| {
        editor_generation_config.with(|g| grammar_axiom.with(|a| a != &g.axiom))
            || applied_grammar_rows.with(|applied| {
                grammar_rows.with(|rows| {
                    rows.len() != applied.len()
                        || rows.iter().zip(applied).any(|(row, (sym, rhs))| {
                            row.symbol.with(|s| s != sym) || row.rhs.with(|r| r != rhs)
                        })
                })
            })
    });
    let grammar_symbols = Memo::new(move |_| {
        let mut set = std::collections::BTreeSet::new();
        grammar_axiom.with(|a| set.extend(a.chars().filter(|c| c.is_ascii_alphabetic())));
        grammar_rows.with(|rows| {
            for row in rows {
                row.symbol
                    .with(|s| set.extend(s.chars().filter(|c| c.is_ascii_alphabetic())));
            }
        });
        set.into_iter().collect::<Vec<char>>()
    });

    let apply_hue_rotation = move |dir: Option<HueRotationDirection>| match dir {
        None => reset_hue_rotation(),
        Some(d) => hue_rotation.update(|m| {
            m.set_direction(d);
            m.start();
        }),
    };

    let try_set_hue_rotation = move |dir: Option<HueRotationDirection>| {
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

    provide_context(ConfigContext {
        config_workspace,
        toml_text,
        selected_id,
        selected_name,
        display_options,
        differs_from_default,
        generation_config,
        editor_color_config,
        control_line_color,
        color_memory,
        unused_rule_symbols,
        iterations,
        max_iterations,
        angle,
        dimensions,
        is_3d,
        is_dirty,
        grammar: GrammarDraft {
            axiom: grammar_axiom,
            rows: grammar_rows,
            row_counter: grammar_row_counter,
            is_dirty: grammar_is_dirty,
            has_3d_symbols: grammar_has_3d_symbols,
            symbols: grammar_symbols,
            sync: Callback::new(move |()| sync_grammar_editor()),
        },
        grammar_error,
        toml_error,
        workspace_error,
        colors_error,
        select_current_config: Callback::new(move |()| select_current_config()),
        clear_toml_revert_state: Callback::new(move |()| clear_toml_revert_state()),
    });
    provide_context(RenderContext {
        renderer,
        auto_rotate,
        auto_rotate_speed,
        hue_rotation,
        hue_rotation_phase,
        animation_error,
        config_for_render: Callback::new(move |()| config_for_render()),
        set_hue_rotation: Callback::new(try_set_hue_rotation),
    });

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
                        {move || selected_name.get()}
                    </span>
                </div>

                <div class="controls-scroll">
                <crate::panels::preset::PresetPanel />
                <crate::panels::config_toml::ConfigTomlPanel />
                <crate::panels::grammar::GrammarPanel />
                <crate::panels::colors::ColorsPanel />
                <crate::panels::animations::AnimationsPanel />
                <crate::panels::save::SavePanel />
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
                            map.insert(id, DVec2::new(ev.client_x() as f64, ev.client_y() as f64));
                        });
                        if let Some(canvas) = canvas_ref.get_untracked() {
                            let _ = canvas.set_pointer_capture(id);
                        }
                    }
                    on:pointermove=move |ev: web_sys::PointerEvent| {
                        let pos = DVec2::new(ev.client_x() as f64, ev.client_y() as f64);
                        let id = ev.pointer_id();

                        let (prev, other, len) = active_pointers.with_value(|map| {
                            let prev = map.get(&id).copied();
                            let other = map.iter().find(|&(&k, _)| k != id).map(|(_, &v)| v);
                            (prev, other, map.len())
                        });

                        let Some(prev) = prev else { return; };

                        active_pointers.update_value(|map| { map.insert(id, pos); });

                        let Some(canvas) = canvas_ref.get_untracked() else { return; };

                        if len == 1 {
                            let delta = pos - prev;
                            with_renderer(canvas, renderer, recover_after_render,
                                |r, c| r.drag_and_render(c, delta.as_vec2()));
                        } else if let Some(other) = other {
                            let prev_dist = prev.distance(other);
                            if prev_dist >= 1.0 {
                                let new_dist = pos.distance(other);
                                let factor = (new_dist / prev_dist) as f32;
                                let mid = ((pos + other) * 0.5).as_vec2();
                                with_renderer(canvas, renderer, recover_after_render,
                                    |r, c| r.zoom_by_factor_and_render(c, factor, mid));
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
                    on:lostpointercapture=move |ev: web_sys::PointerEvent| {
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
                                        Vec2::new(ev.client_x() as f32, ev.client_y() as f32),
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
                        } else if is_3d.get_untracked() {
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
                <div
                    class:hidden=move || viewport_error.get().is_none()
                    class="viewport-error"
                    role="alert"
                >
                    <div>
                        <h2>
                            {move || {
                                viewport_error
                                    .get()
                                    .map(|error| error.title())
                                    .unwrap_or_default()
                            }}
                        </h2>
                        <p>
                            {move || {
                                viewport_error
                                    .get()
                                    .map(|error| error.message())
                                    .unwrap_or_default()
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

/// Applies `update` to the selected entry, returning whether it succeeded and
/// setting/clearing `error` accordingly. If a raw TOML draft is pending, it
/// is discarded first — see docs/specs/application-workspace.md's "Direct
/// configuration controls" section — so `update` always runs against a clean
/// entry. If `update` fails, the discarded draft is restored rather than
/// lost: the direct control's change didn't end up applied either, so the
/// user should not lose both the draft and the attempted change.
pub(crate) fn update_clean_config(
    config_workspace: RwSignal<ConfigWorkspace>,
    error: RwSignal<Option<String>>,
    event: &'static str,
    update: impl FnOnce(&mut CleanMut<'_>) -> Result<(), ParseConfigError>,
) -> bool {
    let result = {
        let mut workspace = config_workspace.write();
        let entry = workspace.selected_mut();
        let pending_draft = entry.is_dirty().then(|| entry.draft_text().into_owned());
        if let EntryViewMut::Dirty(dirty) = entry.view_mut() {
            log::info!("{event}: discarding pending TOML draft to apply a direct control change");
            dirty.revert();
        }
        let update_result = match entry.view_mut() {
            EntryViewMut::Clean(mut clean) => update(&mut clean).map_err(|e| e.to_string()),
            EntryViewMut::Dirty(_) => {
                log::error!("{event}: entry still dirty after discarding its draft");
                Err("Internal error: could not apply this change.".to_string())
            }
        };
        if update_result.is_err()
            && let Some(pending_draft) = pending_draft
        {
            log::info!("{event}: restoring discarded TOML draft after a failed change");
            entry.set_draft_text(pending_draft);
        }
        update_result
    };
    match result {
        Ok(()) => {
            error.set(None);
            true
        }
        Err(msg) => {
            error.set(Some(msg));
            false
        }
    }
}

pub(crate) fn with_renderer<F, H>(
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

fn with_renderer_for_rebuild<F, H>(
    canvas: web_sys::HtmlCanvasElement,
    renderer: RendererState,
    recover_after_render: H,
    viewport_error: RwSignal<Option<ViewportError>>,
    render: F,
) where
    F: FnOnce(&mut CanvasRenderer, &web_sys::HtmlCanvasElement) -> RebuildRenderOutcome,
    H: Fn(RenderStatus, web_sys::HtmlCanvasElement),
{
    with_renderer(
        canvas,
        renderer,
        recover_after_render,
        |renderer, canvas| {
            let outcome = render(renderer, canvas);
            update_viewport_error(viewport_error, outcome.rebuild_result);
            outcome.render_status
        },
    );
}

fn update_viewport_error(
    viewport_error: RwSignal<Option<ViewportError>>,
    rebuild_result: Result<(), SceneUploadError>,
) {
    viewport_error.set(rebuild_result.err().map(ViewportError::SceneUpload));
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
