use crate::panels::grammar::GrammarRow;
use crate::presets::max_iterations_for_editor_config;
use crate::renderer::{CanvasRenderer, RebuildRenderOutcome, RenderStatus};
use glam::{DVec2, Vec2};
use leptos::html::Canvas;
use leptos::prelude::*;
use lsystem_app_model::{
    CAMERA_AUTO_ROTATION_DEFAULT_SPEED_DEGREES_PER_SECOND, CAMERA_ROTATION_STEP_DEGREES, CleanMut,
    ColorControlMemory, ConfigDefaults, ConfigEntryId, ConfigWorkspace, EditorColorConfig,
    EntryViewMut, HueRotation, HueRotationDirection, ParseConfigError, PersistedBaseline,
    advance_hue_rotation_phase_degrees, line_color_for_controls, load_presets,
};
use lsystem_core::{Config, Dimensions, GenerationConfig, LineColorConfig, contains_3d_symbols};
use lsystem_renderer::scene_upload::SceneUploadError;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

pub(crate) type RendererState = StoredValue<Option<CanvasRenderer>, LocalStorage>;

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
    /// Whether the selected entry is a bundled preset (Reset) rather than a custom
    /// copy/import (Remove).
    pub(crate) selected_is_bundled: Memo<bool>,
    /// Whether Reset is enabled: see `ConfigEntry::can_reset`.
    pub(crate) can_reset: Memo<bool>,
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
    /// Fits and resets the camera — the same action as pressing `F`.
    pub(crate) camera_reset: Callback<()>,
    /// Changes azimuth/elevation by `(d_az, d_el)` degrees — the same action
    /// as an arrow-key press. A no-op outside a 3D scene.
    pub(crate) camera_orbit: Callback<(f32, f32)>,
    /// Rolls by the given signed degrees — the same action as `Q`/`E`. A
    /// no-op outside a 3D scene.
    pub(crate) camera_roll: Callback<f32>,
    /// `false` while no `CanvasRenderer` is currently installed (before
    /// first init, and during GPU surface recovery, when it's briefly taken
    /// out of `RendererState`) or while `viewport_error` is set (a failed
    /// rebuild or an unavailable renderer) — every camera action already
    /// silently no-ops in either state (`with_renderer` is a no-op with no
    /// renderer installed; see also `CanvasRenderer::{orbit_and_render,
    /// roll_and_render, reset_and_render}`'s own scene-state guards), so the
    /// camera pane uses this to disable its buttons instead of leaving them
    /// visibly clickable but inert.
    pub(crate) camera_ready: Memo<bool>,
}

/// How long startup waits for browser storage before proceeding without it.
///
/// An implementation choice: the spec requires only that the wait is bounded.
/// Long enough that a healthy IndexedDB open + load (normally a few
/// milliseconds) is never cut short, short enough that a blocked database
/// upgrade in another tab does not leave the user staring at an empty shell.
const STARTUP_LOAD_TIMEOUT_MS: u32 = 3_000;

/// Resolved startup state: the config workspace seeded from bundled presets
/// and, if any, restored from browser storage, plus the baseline describing
/// what this window last loaded from storage (empty when nothing was loaded).
struct InitialState {
    workspace: ConfigWorkspace,
    baseline: PersistedBaseline,
}

impl InitialState {
    /// Bundled presets with nothing recorded as persisted: the state to start
    /// from when storage is unavailable, failed to load, or timed out.
    fn bundled() -> Self {
        Self {
            workspace: ConfigWorkspace::from_presets(load_presets())
                .expect("bundled presets should parse"),
            baseline: PersistedBaseline::default(),
        }
    }
}

/// Progress of the startup decision shared by `AppRoot`'s two startup tasks.
///
/// This is a three-state value rather than an `Option` so that "already
/// decided" stays observable after `App` has taken the state: the late `open`
/// path and the timer both need to tell "still waiting" from "decided and
/// consumed".
enum Startup {
    /// Neither the loader nor the timer has decided yet.
    Pending,
    /// A decision was made and `App` has not been mounted from it yet.
    Ready(InitialState),
    /// A decision was made and `App` has been mounted from it.
    Started,
}

type StartupSignal = RwSignal<Startup, LocalStorage>;

fn startup_pending(init: StartupSignal) -> bool {
    init.with_untracked(|startup| matches!(startup, Startup::Pending))
}

/// Records `state` as the startup decision unless one has already been made.
/// Returns whether `state` was used; whichever caller gets here first wins.
fn settle_startup(init: StartupSignal, state: InitialState) -> bool {
    if !startup_pending(init) {
        return false;
    }
    init.set(Startup::Ready(state));
    true
}

/// Mounts at the document root. Gates `App` (and all its interactive
/// controls) behind the async IndexedDB restore attempt, so that, within the
/// bounded wait described below, a user cannot interact with a stale
/// bundled-preset config that a later-resolving restore then clobbers.
/// Renders an empty `app-shell` placeholder until startup has been decided.
///
/// The wait is bounded. Two tasks race to decide startup: the loader opens
/// storage and loads persisted state, and a timer
/// ([`STARTUP_LOAD_TIMEOUT_MS`]) gives up on it. Whichever settles first
/// wins. The loader settles with the restored workspace and its baseline on
/// success, or with bundled presets and an empty baseline if storage failed
/// to open or load. The timer settles with bundled presets and an empty
/// baseline, and turns the persistence warning on; a blocked database
/// upgrade in another tab can make `storage::open` hang, which is what the
/// timer is for.
///
/// If `open` resolves after the timer has won, the loader installs the handle
/// into `db_slot` and sets `storage_ready`, but never calls `storage::load`,
/// so the late result can never be applied over what the user has already
/// started doing. (A load already in progress when the timer fires is
/// likewise dropped unread.) Saving and refreshing can then work for the rest
/// of the session.
///
/// `persistence_warning` is turned on by a failed open or load, the timer
/// firing, or the connection being closed by another window's upgrade
/// (`versionchange`), and is never turned off. An empty but successful load
/// is normal and does not warn.
///
/// Every await of a `storage::` future happens directly in the loader task
/// (spawned with `spawn_local`) and the timer is a separate task, not a
/// wrapper: IndexedDB transactions only stay alive while the futures using
/// them resume from microtasks, so those futures must never be polled from a
/// timer, animation-frame callback, or resource.
#[component]
pub(crate) fn AppRoot() -> impl IntoView {
    let init: StartupSignal = RwSignal::new_local(Startup::Pending);
    let db_slot: StoredValue<Option<idb::Database>, LocalStorage> = StoredValue::new_local(None);
    let persistence_warning = RwSignal::new(false);
    let storage_ready = RwSignal::new(false);

    // Loader.
    wasm_bindgen_futures::spawn_local(async move {
        let Some(db) = crate::storage::open(move || persistence_warning.set(true)).await else {
            persistence_warning.set(true);
            settle_startup(init, InitialState::bundled());
            return;
        };

        if !startup_pending(init) {
            // The timer won. Install the handle for later saves and
            // refreshes, but do not load: the late result is discarded by
            // never being read.
            db_slot.update_value(|slot| *slot = Some(db));
            storage_ready.set(true);
            return;
        }

        let initial = match crate::storage::load(&db).await {
            Some(stored) => {
                let mut initial = InitialState::bundled();
                initial.baseline = initial.workspace.restore(&stored);
                initial
            }
            None => {
                persistence_warning.set(true);
                InitialState::bundled()
            }
        };

        // Install the handle before settling, so `App` sees it from its
        // first render.
        db_slot.update_value(|slot| *slot = Some(db));
        if !settle_startup(init, initial) {
            // The timer fired while the load was in flight: `initial` was
            // dropped unused, and this handle is a late install.
            storage_ready.set(true);
        }
    });

    // Timer. A separate task, never a wrapper around a storage future.
    wasm_bindgen_futures::spawn_local(async move {
        gloo_timers::future::TimeoutFuture::new(STARTUP_LOAD_TIMEOUT_MS).await;
        if settle_startup(init, InitialState::bundled()) {
            persistence_warning.set(true);
        }
    });

    view! {
        <Show
            when=move || init.with(|startup| !matches!(startup, Startup::Pending))
            fallback=|| view! { <main class="app-shell"></main> }
        >
            {move || {
                let InitialState { workspace, baseline } = init
                    .try_update_untracked(|startup| {
                        match std::mem::replace(startup, Startup::Started) {
                            Startup::Ready(state) => Some(state),
                            Startup::Pending | Startup::Started => None,
                        }
                    })
                    .flatten()
                    .expect("Show only renders this branch once `init` is Ready");
                view! {
                    <App
                        initial_workspace=workspace
                        baseline=baseline
                        db_slot=db_slot
                        persistence_warning=persistence_warning
                        storage_ready=storage_ready
                    />
                }
            }}
        </Show>
    }
}

#[component]
pub(crate) fn App(
    initial_workspace: ConfigWorkspace,
    baseline: PersistedBaseline,
    db_slot: StoredValue<Option<idb::Database>, LocalStorage>,
    persistence_warning: RwSignal<bool>,
    storage_ready: RwSignal<bool>,
) -> impl IntoView {
    let selected_entry = initial_workspace.selected();
    let color_memory = RwSignal::new(ColorControlMemory::from_editor_config(
        &selected_entry.editor_config().colors,
        &ConfigDefaults::embedded().colors,
    ));
    let config_workspace = RwSignal::new(initial_workspace);
    // Storage-task bookkeeping; see `run_storage_task` below. `baseline` is
    // what this window believes storage holds, and the three flags drive the
    // one single-flight loop that owns every read and write after startup.
    let baseline: StoredValue<PersistedBaseline, LocalStorage> = StoredValue::new_local(baseline);
    let want_save: StoredValue<bool, LocalStorage> = StoredValue::new_local(false);
    let want_refresh: StoredValue<bool, LocalStorage> = StoredValue::new_local(false);
    let save_in_flight: StoredValue<bool, LocalStorage> = StoredValue::new_local(false);
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
    let selected_is_bundled =
        Memo::new(move |_| config_workspace.with(|ws| ws.selected().is_bundled()));
    let can_reset = Memo::new(move |_| config_workspace.with(|ws| ws.selected().can_reset()));
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
    let renderer_ready = RwSignal::new(false);

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
            renderer_ready.set(false);
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
                        renderer_ready.set(true);
                    }
                }
                Err(err) => {
                    log::error!("Failed to recover GPU surface: {err}");
                    viewport_error.set(Some(ViewportError::RendererUnavailable(err.to_string())));
                }
            }
        });
    };

    let camera_ready =
        Memo::new(move |_| renderer_ready.get() && viewport_error.with(|e| e.is_none()));
    let camera_reset = Callback::new(move |()| {
        let Some(canvas) = canvas_ref.get_untracked() else {
            return;
        };
        with_renderer(canvas, renderer, recover_after_render, |r, c| {
            r.reset_and_render(c)
        });
    });
    let camera_orbit = Callback::new(move |(d_az, d_el): (f32, f32)| {
        let Some(canvas) = canvas_ref.get_untracked() else {
            return;
        };
        with_renderer(canvas, renderer, recover_after_render, move |r, c| {
            r.orbit_and_render(c, d_az, d_el)
        });
    });
    let camera_roll = Callback::new(move |degrees: f32| {
        let Some(canvas) = canvas_ref.get_untracked() else {
            return;
        };
        with_renderer(canvas, renderer, recover_after_render, move |r, c| {
            r.roll_and_render(c, degrees)
        });
    });

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
                    renderer_ready.set(true);
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
    // copy, import, remove). Removal always selects a different entry, so this watcher
    // is its single resync path. Same-id mutations that change the applied document
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

    // What starts a save: the selected entry's identity and its applied text.
    //
    // Invariant: user-driven mutations touch only the selected entry (plus
    // adding, removing, and selecting). Do not add a public
    // `ConfigWorkspace` method or UI action that mutates a non-selected entry
    // without also making it start a save; the trigger would silently miss
    // it. The `pagehide`/hidden flush (a full diff regardless of trigger) is
    // only a backstop.
    //
    // This is deliberately *not* the persisted view: every user-driven
    // mutation goes through `selected_mut()`, `select_by_id`, `copy`,
    // `import_toml`, or `remove_selected`, all of which change this pair
    // (a selection-only change moves its id half, and adding an entry
    // auto-selects it), so walking every entry here would buy nothing.
    // Raw-draft keystrokes change neither half, so they start nothing. The
    // full `persisted_view()` walk and the baseline diff happen once per
    // save, inside the storage task.
    let save_trigger = Memo::new(move |_| {
        config_workspace
            .with(|workspace| (workspace.selected_id(), workspace.selected().applied_text()))
    });

    // The one serialized storage task. The `idb::Database` handle is not
    // `Clone` and is leased out of `db_slot` for the duration of an
    // operation, so saves and refreshes share a single-flight loop driven by
    // the `want_save` and `want_refresh` flags: a trigger publishes its flag
    // and then calls this, and whichever task is running drains both.
    let run_storage_task = move || {
        // Check `save_in_flight` first, before ever looking at `db_slot`.
        // The in-flight task below takes the `Database` handle out of
        // `db_slot` for the duration of each pass (see the comment at the
        // take site), so `db_slot` reads as empty while an operation is
        // running — that must never be confused with "storage was never
        // available." This branch doesn't touch `db_slot` at all: if a task
        // is in flight, it already established availability when it started,
        // and it re-checks both flags after every pass, so the flag the
        // caller just set is picked up there.
        if save_in_flight.get_value() {
            return;
        }
        // No task in flight, so `db_slot` (if any) is currently held by
        // nobody — safe to check availability here.
        if db_slot.with_value(Option::is_none) {
            // Storage is unavailable, or has not finished opening. Degrade
            // silently; the flags stay set, so the `storage_ready` flush
            // below picks the work up if a handle arrives later.
            return;
        }
        // Set synchronously, before spawning and never inside the task: a
        // trigger that arrives before the task body first runs must see "in
        // flight" and leave its flag for the loop, not start a second task.
        save_in_flight.set_value(true);
        wasm_bindgen_futures::spawn_local(async move {
            while want_refresh.get_value() || want_save.get_value() {
                // Exclusively takes the handle out of `db_slot` for this
                // pass; `db_slot` reads as empty to any concurrent trigger
                // until it is restored at the bottom of the loop. That's
                // fine: those callers gate on `save_in_flight` (set above,
                // before this task ever ran), not on `db_slot`'s Some/None
                // state, so a momentarily-empty `db_slot` is never mistaken
                // for "storage unavailable." There is deliberately no early
                // exit between here and the restore, so the handle always
                // goes back.
                let Some(Some(handle)) = db_slot.try_update_value(|opt| opt.take()) else {
                    break;
                };

                if want_refresh.get_value() {
                    // Cleared before the work, so a refresh trigger arriving
                    // during it schedules another pass instead of being lost.
                    want_refresh.set_value(false);
                    match crate::storage::load(&handle).await {
                        // A failed refresh leaves this window unchanged.
                        None => persistence_warning.set(true),
                        Some(stored) => {
                            // Eligibility must be judged at call time: the
                            // grammar-draft state is read untracked here and
                            // applied in the same synchronous step, with no
                            // await in between, so the decision is never
                            // carried across one.
                            let grammar_draft_pending = grammar_is_dirty.get_untracked();
                            let outcome = config_workspace
                                .try_update(|workspace| {
                                    baseline.try_update_value(|baseline| {
                                        workspace.refresh(&stored, baseline, grammar_draft_pending)
                                    })
                                })
                                .flatten()
                                .unwrap_or_default();
                            // A same-id content change does not move
                            // `selected_id`, so its watcher never sees it —
                            // exactly as for a raw TOML apply and a reset,
                            // which also resync explicitly. `refresh` has
                            // already brought the baseline in step, so the
                            // save this may start diffs to empty.
                            if outcome.selected_content_changed || outcome.selection_moved {
                                select_current_config();
                            }
                        }
                    }
                }

                if want_save.get_value() {
                    want_save.set_value(false);
                    // The only place the full persisted view is walked and
                    // diffed against the baseline: once per save, off the
                    // live workspace read untracked.
                    let view = config_workspace.with_untracked(ConfigWorkspace::persisted_view);
                    let delta =
                        baseline.with_value(|baseline| lsystem_app_model::diff(baseline, &view));
                    if !delta.is_empty() {
                        match crate::storage::save(&handle, &delta).await {
                            Some(minted) => {
                                if !minted.is_empty() {
                                    config_workspace.update(|workspace| {
                                        for (entry, id) in &minted {
                                            if !workspace.assign_custom_id(*entry, *id) {
                                                log::warn!(
                                                    "autosave: no entry left to record minted \
                                                     custom id {} on",
                                                    id.get()
                                                );
                                            }
                                        }
                                    });
                                }
                                baseline
                                    .update_value(|baseline| baseline.apply_saved(&delta, &minted));
                            }
                            // The baseline is deliberately left unchanged and
                            // `want_save` is *not* re-set: the next trigger
                            // re-diffs from the live workspace. Replaying
                            // this delta would re-mint without re-reading.
                            None => persistence_warning.set(true),
                        }
                    }
                }

                db_slot.update_value(|opt| *opt = Some(handle));
            }
            // Reached only from the synchronous loop-condition check (or the
            // `break` above), with no await in between, so no trigger can
            // slip in between the last check of the flags and this clear.
            save_in_flight.set_value(false);
        });
    };

    // Both helpers publish the flag *before* poking the task, so a task that
    // is already running picks the work up on its next pass.
    let start_save = move || {
        want_save.set_value(true);
        run_storage_task();
    };
    let start_refresh = move || {
        want_refresh.set_value(true);
        run_storage_task();
    };

    // The handler first runs on the first genuine change after mount
    // (immediate = false): `config_workspace` is seeded with the value
    // already restored (or freshly bundled) by `AppRoot`, which `baseline`
    // already describes, so mounting must not start a save by itself.
    Effect::watch(
        move || save_trigger.get(),
        move |_, _, _: Option<()>| start_save(),
        false,
    );

    // `storage_ready` means `AppRoot` installed the handle *after* startup
    // had already been decided without it, so whatever changed in the
    // meantime is still unsaved. It can already be `true` when `App` first
    // renders, so this is a plain effect that also runs at mount rather than
    // a false->true watch. `AppRoot` fills `db_slot` before setting it, so
    // `run_storage_task` always finds the handle.
    Effect::new(move |_| {
        if storage_ready.get() {
            start_save();
        }
    });

    // Flush backstops, each only setting `want_save`: the full diff writes
    // nothing when nothing changed. Saves stay immediate; these exist for a
    // reload/close landing in an in-flight write's async gap (`pagehide`),
    // for mobile, where the page being hidden is the more reliable signal,
    // and for a switch to another window that leaves this page visible
    // (`blur`). None of them can guarantee completion if the browser
    // terminates the page immediately.
    let pagehide_handle = window_event_listener(leptos::ev::pagehide, move |_| start_save());
    let blur_handle = window_event_listener(leptos::ev::blur, move |_| start_save());
    // `visibilitychange` is a document event, but it bubbles up to the
    // window, so one window-level listener sees it. Becoming visible again
    // is a refresh trigger — this is where a backgrounded window picks up
    // what other windows saved.
    let visibility_handle = window_event_listener(leptos::ev::visibilitychange, move |_| {
        if document().hidden() {
            start_save();
        } else {
            start_refresh();
        }
    });
    // Window focus overlaps with becoming visible. `want_refresh` coalesces
    // triggers that arrive while a load is already in flight, but these two
    // usually straddle the microtask checkpoint that first polls the spawned
    // task (`spawn_local` schedules through `queueMicrotask`), so activating a
    // tab typically costs two loads — the second finding nothing to change and
    // writing nothing. That is the right trade: clearing `want_refresh` after
    // the load instead of before it would coalesce them, but would then drop a
    // trigger that arrives mid-load.
    let focus_handle = window_event_listener(leptos::ev::focus, move |_| start_refresh());
    on_cleanup(move || {
        pagehide_handle.remove();
        blur_handle.remove();
        visibility_handle.remove();
        focus_handle.remove();
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
        selected_is_bundled,
        can_reset,
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
        camera_reset,
        camera_orbit,
        camera_roll,
        camera_ready,
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
                <crate::panels::camera::CameraPanel />
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
                        let key = ev.key();
                        if key.eq_ignore_ascii_case("f") {
                            camera_reset.run(());
                        } else if is_3d.get_untracked() {
                            let handled = match key.as_str() {
                                "ArrowLeft" => {
                                    camera_orbit.run((-CAMERA_ROTATION_STEP_DEGREES, 0.0));
                                    true
                                }
                                "ArrowRight" => {
                                    camera_orbit.run((CAMERA_ROTATION_STEP_DEGREES, 0.0));
                                    true
                                }
                                "ArrowUp" => {
                                    camera_orbit.run((0.0, CAMERA_ROTATION_STEP_DEGREES));
                                    true
                                }
                                "ArrowDown" => {
                                    camera_orbit.run((0.0, -CAMERA_ROTATION_STEP_DEGREES));
                                    true
                                }
                                "q" | "Q" => {
                                    camera_roll.run(-CAMERA_ROTATION_STEP_DEGREES);
                                    true
                                }
                                "e" | "E" => {
                                    camera_roll.run(CAMERA_ROTATION_STEP_DEGREES);
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
