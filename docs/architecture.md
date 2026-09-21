# Architecture

Grow Your Own Fractal is a Rust workspace for generating, editing, rendering,
and exporting 2D and 3D L-System fractals. The core model is toolkit-agnostic:
grammar expansion and turtle geometry live in `lsystem-core`, shared
application state lives in `lsystem-app-model`, and GPU rendering/export support
lives in `lsystem-renderer`. The native/Iced and browser-first/Leptos apps are
thin UI layers over those shared crates.

Supported behavior is defined by the
[project specifications](specs/README.md). This document owns crate boundaries,
data flow, and implementation design; it does not redefine the behavior and UX
contracts in the specifications.

## Workspace Crates

| Crate | Role |
|-------|------|
| `lsystem-core` | Pure L-System library: runtime config types, symbol validation, lazy grammar expansion, 2D/3D turtle geometry, and SVG export behind the `svg` feature. It has no rendering, TOML, serde, UI, or platform dependencies. |
| `lsystem-app-model` | Toolkit-independent app model: TOML parsing/validation/default resolution, session config workspace, embedded presets, color-control helpers, hue rotation state, and filename utilities. It must not depend on `iced`, `leptos`, `web-sys`, `wasm-bindgen`, `wgpu`, or `lsystem-renderer`. |
| `lsystem-renderer` | Toolkit-independent wgpu layer: camera math, line pipelines, L-System-to-GPU adapters, browser/native wgpu setup, and PNG/APNG export behind the `png` feature. |
| `lsystem-app` | Iced native app and retained Iced web app. The fractal viewport is an `iced::widget::shader` primitive backed by `lsystem-renderer`. |
| `lsystem-web-app` | Browser-first Leptos app with DOM controls and a dedicated wgpu canvas. It is the primary web app deployed at the GitHub Pages root. |

Bundled fractals live in [`presets/`](../presets/). They are embedded at compile
time by `lsystem-app-model::load_presets`; adding a TOML file there is enough to
make it available to both UIs.

## Data Flow

Runtime generation starts from a resolved `GenerationConfig`.

```text
GenerationConfig
  -> AnyCompiledGeneration
  -> CompiledGeneration<D2> / CompiledGeneration<D3>
  -> GenerationPlan<D> (exact count + bounded strategy selection)
  -> PreparedGeneration<D> (stamped templates or interpreter)
  -> strategy-independent world-space geometry (segments() / depth_segments())
  -> renderer scene upload
  -> wgpu instance buffer
```

The expansion and turtle layers are streaming iterators. They do not build the
full expanded string or an intermediate vertex list before yielding geometry.
`PreparedGeneration::{segments, depth_segments}` are the production geometry
boundary: they hide whether the plan selected stamped templates or direct
interpretation while preserving lazy, resumable iteration. For web and
offscreen rendering, transformed segment records stream directly into wgpu
staging memory without a stamp or segment collection. The native Iced app
generates without GPU access and intentionally collects one bounded
segment-instance `Vec` before uploading it as a slice. Direct
`CompiledGeneration::{segments, depth_segments}` iteration remains available
as the interpreter semantic oracle and for benchmarks that intentionally
measure interpretation. Do not add another collection of expanded symbols, raw
geometry, or vertices to either path.

Config editing has a separate parse/validate/resolve pipeline.

```text
ConfigSource::parse
  -> RawConfig
  -> EditorConfig
  -> ConfigDocument
  -> EditorConfig::resolve(defaults, max_iterations)
  -> Config / GenerationConfig
```

`ConfigSource` preserves TOML formatting but is only parse-valid.
`ConfigDocument` is the invariant that the authored config is valid. Runtime
rendering and export paths resolve a concrete `Config` at their boundary rather
than caching resolved values inside the editor document.

Iteration counts use `u16` throughout the authored editor model and resolved
core model. The separate authored and interactive limits are defined in the
[configuration](specs/configuration.md#defaults-and-resolution) and
[workspace](specs/application-workspace.md#interactive-iteration-limit)
specifications.

Compiled generations are consumed into an allocation-free `GenerationPlan`
that fuses exact drawn-segment counting and bounded template-depth selection in
one saturating byte-domain recurrence. Renderer scene upload uses the plan's
count to enforce the actual plain/depth record-layout cap before preparation,
template construction, output iteration, interpreter generation, or pipeline
mutation.

## Core Model

`lsystem-core` owns the grammar and turtle semantics:

- `alphabet.rs` validates reserved symbols for 2D and 3D.
- `dimension.rs` defines the sealed `D2`/`D3` markers and maps each marker to
  its point, rotation, bounds result, and mergeable bounds accumulator through
  the `Dimension` trait, alongside local-to-world point and segment transforms.
  Two-dimensional bounds are exact endpoint min/max; three-dimensional bounds
  are a conservative world-Y cylinder. Empty accumulation remains `None` in
  core and is mapped to renderer-owned unit bounds at scene collection.
- `compiled_generation.rs` is the runtime compilation boundary. It compiles
  grammar, scalar parameters, and stack metadata together, then exposes an
  opaque `CompiledGeneration<D2>` or `CompiledGeneration<D3>` payload inside
  `AnyCompiledGeneration`. Runtime consumers match once at their boundary and
  can only pass the matching typed value to 2D or 3D generation and template
  APIs. A hidden sealed marker capability constructs the dimension-specific
  turtle behind a shared streaming iterator shell, so `segments()` and
  `depth_segments()` have one generic implementation. The
  `CompiledGeneration2D` and `CompiledGeneration3D` aliases retain concise
  dimension-specific names where concrete signatures remain useful. Production
  consumers normally obtain geometry through `GenerationPlan::prepare` and
  `PreparedGeneration`, while direct compiled-generation iteration remains the
  interpreter oracle.
- `grammar.rs` provides the crate-private compiled grammar representation
  (shared byte arena plus rule table, unreachable rules dropped) and lazy
  expansion iterators used by the typed generation façade.
- `turtle/turtle2d.rs` yields 2D line segments from expanded symbols.
- `turtle/turtle3d.rs` yields 3D line segments using quaternion orientation.
  Both turtle states keep their dimension-specific symbol transitions while a
  crate-private iterator owns the common `next` and optimized `fold` paths.
- `template.rs` provides the alternative stamped generation path: per-rule
  geometry templates (a rule expanded a fixed number of iterations in the
  local frame, with its exit transform) plus a placement walk that streams
  stamps in traversal order. `TemplateSegment<D>`, `Template<D>`, `Stamp<D>`,
  and `TemplateSet<D>` keep every payload paired with its dimension;
  `TemplateSet2D/3D` and the related concrete names remain aliases for concise
  call sites. The build and placement walks share one generic algorithm over
  the two turtle representations, while a marker capability exposes budgeted
  construction and lazy placement to dimension-generic orchestration. A
  private placement iterator owns the boundary expansion, turtle state, and
  `u64` traversal order; its resumable `next` and optimized `fold` paths share
  the same symbol transition. Crate-private template-set iterators flat-map
  those placements over template-local geometry, keeping world-space
  transformation in core without collecting stamps or segments;
  `PreparedGeneration::{segments, depth_segments}` expose the resulting
  strategy-independent production walks. Explicit template construction and
  template-local inspection through `TemplateSet::templates` and public
  `Template::segments` remain supported, as does transforming a `Stamp` a
  caller already has. The public `emit_stamps` convenience walk and
  `StampStats` metadata type are removed; the public marker trait's doc-hidden
  `stamp_placements` hook remains callable for generic implementation
  dispatch, but is not the supported consumer façade. A set owns its typed
  compiled generation, so stamping needs no config re-supply. Template sets
  are small, budget-bounded collections.
  `CompiledGeneration::plan_templates` picks the largest template depth whose
  templates fit a caller-supplied budget while simultaneously counting exact
  output. `GenerationPlan::prepare` then builds that depth or returns
  `PreparedGeneration::Interpreted` with the owned generation when none fits.
  Fixed-depth tests and benchmarks use `CompiledGeneration::build_templates`,
  whose structured error returns the generation when the requested depth is
  invalid. The interpreter remains the semantic oracle.
- `config.rs` defines validated runtime config and color types.
  `GenerationConfig::new` is the only way to build a generation config; it
  enforces single-letter rule keys and bracket balance on the axiom and every
  rule RHS, so every expansion is balanced and downstream code (turtle stack
  handling, templates) relies on that invariant instead of re-validating.
  The axiom and rules are read-only after construction. Rewrite and template
  iteration counts use `u16`; output counts, traversal order, and topological
  depth retain their wider types because they measure generated structure
  rather than rewrite rounds.
- `svg_export.rs` exports resolved 2D configs when the `svg` feature is enabled
  and explicitly rejects 3D input at its public runtime-config boundary.

`GenerationConfig::compile` snapshots grammar, scalar parameters, and stack
metadata together, then returns `AnyCompiledGeneration::TwoD` or `ThreeD`.
Runtime boundaries exhaustively match that enum once. Each
`CompiledGeneration<D>` payload exposes dimension-specific `segments()` and
`depth_segments()` methods and can only be consumed by the matching template
set, so grammar and parameters cannot be separated or combined across configs.

3D turtle orientation is stored as a `glam::Quat`. Heading, left, and up vectors
are derived from that orientation, and pitch/roll/yaw symbols compose in local
space. This avoids the accumulation and ordering problems that come from
tracking heading as a single scalar angle.

Authored grammar normalization and turtle behavior are defined in the
[L-system specification](specs/l-system.md).

## App Model

`lsystem-app-model` is the shared state and config boundary for both UIs.

- `config_defaults.rs` parses embedded `defaults.toml` and validates default
  turtle/color values with the same domain checks as authored configs.
- `editor_config.rs` parses strict TOML, validates symbols and value domains,
  preserves optional authored fields, and resolves defaults at runtime
  boundaries.
- `config_workspace.rs` tracks preset/custom entries identified by opaque
  `ConfigEntryId`s, dirty drafts, last-applied documents, copy/apply/revert/reset
  operations, and derived display labels for entries with duplicate authored
  names. It owns the bundled/custom distinction and shared Reset eligibility
  (`can_reset`), and removal of custom entries (`remove_selected`), which
  re-anchors the selection to the following (or else preceding) entry and never
  reuses ids. Each UI owns only the removal confirmation and the editor and
  renderer resync.
- `persistence.rs` defines the storage-independent persisted view of a
  workspace: `PersistedKey` and `CustomId` identity, `StoredState`, the live
  `PersistedView`, a per-window `PersistedBaseline`, and the pure `diff` that
  yields a `SaveDelta`. `config_workspace.rs` adds `persisted_view`, `restore`,
  `refresh`, and `assign_custom_id` on top. Neither knows about IndexedDB,
  serialization, or tabs; the web app's `storage.rs` maps these types to its
  own representation.
- `presets.rs` embeds and sorts the `presets/` directory.
- `color.rs` centralizes line-color mode selection and per-mode picker memory.
- `animation.rs` contains hue-rotation state and phase advancement.

The authored schema and omission/default behavior are defined in the
[configuration specification](specs/configuration.md). Omitting `colors.line`
resolves to solid mode because the externally tagged TOML shape requires an
explicit subtable to select gradient or hue-cycle mode. Making either of those
the omission default would leave no equivalent omission-based way to select
solid mode.

## Rendering

`lsystem-renderer` owns the shared wgpu machinery used by both apps and by
offscreen exports.

- Shader sources live in `src/shaders/` as WESL modules: `common.wesl`
  declares the shared `ColorParams` uniform (group 0, binding 0) and the color
  helper functions, and `shader_2d.wesl`/`shader_3d.wesl` each import it
  (`import package::common::{...}`) and add their own transform uniform
  (`Transform`/`Mvp`, binding 1) and vertex/fragment entry points.
- `build.rs` compiles the `package::shader_2d` and `package::shader_3d` root
  modules to plain WGSL via the `wesl` crate (`ManglerKind::None`), writes
  `shader_2d.wgsl`/`shader_3d.wgsl` to `OUT_DIR`, and runs `wgsl_to_wgpu` on
  each compiled WGSL string to generate `shader_2d_bindings.rs`/
  `shader_3d_bindings.rs`. `lib.rs` includes those as the
  `generated_shader_2d`/`generated_shader_3d` modules, and `line_renderer.rs`
  loads the compiled WGSL text at runtime via `include_str!` on the `OUT_DIR`
  files. `line_renderer.rs` sources its uniform types (`ColorParams`,
  `Transform`, `Mvp`) and shader entry-point constants from the generated
  bindings rather than hand-mirroring them, so most field, binding, and
  entry-point renames in the WESL sources now fail the Rust build instead of
  only surfacing as a runtime wgpu validation error. Each pipeline uses a
  single bind group (group 0: `color_params` at binding 0, `transform`/`mvp`
  at binding 1); bind group layouts, bind groups, vertex instance records,
  vertex buffer layouts, and shader entry states are built from generated
  helpers, and each pipeline layout wraps that one generated bind group
  layout.
- `camera.rs` supports 2D pan/zoom and 3D orbit/elevation/roll/zoom using typed
  core bounds. The 3D camera targets the midpoint of the world-Y bounding
  cylinder and fits its exact support against all four perspective side
  planes. Its horizontal circular section keeps distance stable under azimuth
  orbit, while the elevation captured by the last position reset and the live
  roll determine the framing axes. Pointer orbiting uses direct-manipulation
  semantics, so the model follows the drag. The browser passes CSS-pixel
  deltas without device-pixel-ratio scaling, which keeps orbit sensitivity
  consistent across display densities.
- `line_renderer.rs` defines GPU instance records, growable vertex buffers,
  the marker-keyed `LinePipeline<D>`, color uniforms, and surface frame
  handling. `RenderDimension` maps `D2`/`D3` to their plain and depth record
  layouts and constructors. Generated shader and bind-group
  construction plus view-uniform writes remain specialized, while buffer
  uploads, color writes, and draw dispatch are shared. A crate-private
  `DimensionBindGroup` bound on the shared constructor ties each generated bind
  group to its dimension marker, so pairing a pipeline with the other shader's
  bind group fails to compile. Existing segment slices use `Queue::write_buffer`;
  prepared-generation iterators use `Queue::write_buffer_with` to fill mapped
  staging memory directly. `LinePipeline<D3>::new` optionally builds both its
  pipeline variants with a `wgpu::DepthStencilState` (`LINE_DEPTH_FORMAT`) when
  given a depth format, so segments can be GPU depth-tested by camera distance
  instead of only by traversal order; `LinePipeline<D2>` never does, since 2D
  scenes have no meaningful camera-space depth. Any render pass drawing a
  depth-enabled `LinePipeline<D3>` must attach a matching depth view — wgpu
  requires the pipeline's depth-stencil state and the pass's depth attachment
  to agree.
- `lsystem_bridge.rs` converts core geometry iterators into GPU segment data and
  maps `LineColorConfig` into shader color parameters. Consumers feed core's
  dimension-specific bounds accumulator from world-space endpoints before
  constructing GPU records through `RenderDimension`; native collected scene
  data carries the resulting typed bounds alongside its one segment vector.
- `scene_upload.rs` owns the single generic `upload_scene<D>` (composing
  `RenderDimension + TemplateDimension`, whose core bound includes interpreted
  generation), the public renderer operation for web and offscreen scene
  generation/upload. It plans the generation, clamps the requested layout for
  bracketless grammars, checks segment caps via per-record `record_limit`
  before preparation, then streams the prepared generation's façade iterator
  directly into wgpu staging while accumulating bounds and maximum
  topological depth. Color parameters are written only after the geometry
  drain succeeds. It returns `UploadedScene<D>` metadata with `D::Bounds`, so
  2D and 3D consumers cannot accidentally exchange rectangle and cylinder
  bounds. The prepared generation is inspected only to record stamped-versus-
  interpreted method metadata; geometry consumption always goes through its
  façade iterators. A cap error preserves the previous
  pipeline scene; a staging error clears the attempted target layout.
- `offscreen.rs`, `png_export.rs`, and `animation_export.rs` render PNG/APNG
  output with an offscreen target behind the `png` feature. Segment-limit and
  staging failures surface as typed export errors instead of empty images.
- `wgpu_util.rs` centralizes instance/device setup and error logging for native
  and browser targets.

Line rendering is instanced: one GPU record represents one segment, and the
shader selects the start or end point from `vertex_index`. Buffers grow to the
next power-of-two capacity and are reused across slice and staging uploads.

The 2D and 3D shaders remain separate because they use different vertex entry
points, vertex-buffer layouts, and transform uniforms, so they live in
`shader_2d.wesl`/`shader_3d.wesl`. Their specialized constructors feed those
dimension-specific resources into one `LinePipeline<D>` implementation; the
`LinePipeline2D` and `LinePipeline3D` aliases keep concrete call sites concise.
Both shaders import `common.wesl` for the color uniform model.

## Color And Depth

`color_params_from_config` is the boundary that maps resolved line-color state
and available scene metadata into shader uniforms. Grammars with stack
directives use depth-bearing records; the requested color mode decides whether
the shader consumes that depth. Hue animation updates uniforms without
rebuilding geometry. Color and topological-depth semantics are defined in the
[rendering specification](specs/rendering-and-interaction.md#color-modes).

## App Layers

`lsystem-app` uses Iced for native desktop and retained-mode wasm builds. Iced
owns the window, surface, event loop, and render pass; the fractal shader widget
owns only the fractal GPU pipeline state. Geometry generation is asynchronous
and tokenized so stale generation results can be ignored after rapid input
changes. Geometry and color revisions let the shader upload segment data only
when geometry changes and update only color uniforms for color-only edits.
Scene geometry is built once generically per dimension marker
(`build_typed_scene<D>` over an app-local `SceneDimension` trait). It prepares
the planned strategy and drains its lazy façade iterator through the same
incremental renderer-record builders with periodic cancellation checks. Native
telemetry reports the selected template iteration count, with zero denoting
interpreted generation. Iced's own shared render
pass never attaches a depth buffer, so 2D scenes keep using the cheap
`shader::Primitive::draw` path into that shared pass. For 3D scenes,
`FractalPrimitive::draw` returns `false`, which makes Iced defer to
`shader::Primitive::render` instead — an escape hatch that hands the
primitive its own command encoder and the real frame view, letting it open a
self-contained pass with a `LINE_DEPTH_FORMAT` depth attachment (lazily
allocated on first use, sized to the window's physical framebuffer, and
released again once the scene returns to 2D). 3D geometry is therefore GPU
depth-tested the same way as `lsystem-web-app`, not drawn purely in
traversal order.

`lsystem-web-app` uses Leptos for DOM controls and renders into a dedicated
canvas. The renderer owns both 2D and 3D pipelines, handles resize/zoom/orbit/
roll/auto-rotate/reset operations, and rebuilds GPU state after surface loss
while preserving camera and color state. Its `GpuContext` owns a
`LINE_DEPTH_FORMAT` depth attachment sized to the canvas, recreated in
lockstep with the swapchain on every resize; the 3D render pass attaches it
(2D and no-upload passes don't), so 3D geometry is GPU depth-tested by true
camera distance rather than drawn purely in traversal order. Scene rebuild uploads immediately and
retains only opaque bounds/count/layout/method metadata, not segment vectors.
Before any successful upload and after either upload error, the renderer uses a
dimensionless no-upload state. That state selects and mutates neither line
pipeline, so the canvas clears and presents only the configured background;
the app displays a structured, actionable viewport error for the failed upload.
A later successful upload restores its 2D or 3D scene metadata and clears that
error, including when the successful scene has zero segments. The generation
log reports the chosen `template_iterations` (0 = interpreter), while the
upload-frame metric measures from successful rebuild completion through the
next submitted frame's GPU completion.

Shared and platform-specific interaction behavior is defined in the
[rendering specification](specs/rendering-and-interaction.md).

`lsystem-web-app` persists the workspace to browser storage so that presets,
custom entries, and the selection survive reloads and are shared coherently
between windows. What persists and how restore, refresh, and failures behave
is defined in the
[application workspace specification](specs/application-workspace.md#persistence);
this section covers the mechanism. The work is split along the crate boundary:
`lsystem-app-model` describes storage in the app's own terms and has no notion
of IndexedDB, tabs, or windows, while `storage.rs` in the web app owns the
whole representation (store names, key encodings, transactions) and exchanges
only `StoredState` and `SaveDelta` values with the rest of the app. One
IndexedDB database (`lsystem-autosave`, version 2) holds three object stores:
`presets`, keyed by bundled file path; `customs`, an `autoIncrement` store
whose integer key is the custom entry's identity; and `meta`, which holds the
selection. The upgrade creates missing stores and deletes the legacy
single-record store. A custom entry has no identity when it is created, which
keeps Copy and Import synchronous and working with storage unavailable. Its
id is the key of the first `customs` add inside the save that persists it, so
ids are unique across windows without coordination and integer order is
creation order; a later put under an explicit key brings a removed entry back
under its original id.

Each window keeps a `PersistedBaseline` of what it last loaded or saved, per
key and in the same representation as the workspace's `PersistedView` (a
preset only while it differs from its bundled default, a custom always). A
save diffs the live view against the baseline and applies the resulting
delta in one readwrite transaction: puts for changed entries, mints for
custom entries without an id, the selection if it changed, and deletes only
for keys the baseline holds that the view no longer has (a reset preset, a
removed custom). Deletes are never derived by sweeping the store, so a window
cannot remove rows it never held, and rows it could not parse are noted apart
from the recorded entries and left as they are. Ids are assigned to
workspace entries and the baseline advances only after the transaction
commits; a failed save changes neither, and the next trigger re-diffs from the
live workspace.

Startup restore and live refresh are separate `ConfigWorkspace` operations
because they answer different questions. `restore` runs once on a workspace
just seeded from bundled presets. It applies stored presets by path through
the draft, so the bundled default and Reset survive, recreates customs in id
order carrying their ids, appends edits of no-longer-bundled presets as
id-less customs after the others, and skips invalid rows. Each orphan's old
`preset:` key stays in the baseline, so the next save deletes the stale row and
mints the custom in one transaction. `refresh` reconciles a live workspace with
a fresh read and updates the baseline in step, so it never has to start a save.
An entry is eligible for update only if it has no raw draft, no grammar draft
(a UI fact the caller passes in, and which only concerns the selected entry),
and applied content equal to its baseline record. Eligibility is judged
against the live workspace in the same synchronous step as the read being
applied, never across an await. An eligible entry takes the stored content,
reverts if reset elsewhere, or is dropped if removed elsewhere, while entries
created elsewhere are appended in id order. The stored selection is never
adopted; it moves only when the selected entry itself is removed.

After startup, every read and write runs in one serialized storage task in
`app.rs`. `idb::Database` is not `Clone`, so the task leases the handle out
of a shared slot for each pass of a single-flight loop driven by `want_save`
and `want_refresh` flags. A trigger publishes its flag and pokes the task, and
the running task re-checks both flags after every pass, so triggers that
arrive mid-operation coalesce into another pass instead of being lost. The
in-flight check comes before the availability check because the handle is out
of the slot while an operation runs, and an empty slot then must not be read as
unavailable storage. The save trigger is only the selected entry's id and
applied text, since every user-driven mutation goes through the selected entry
or through add, remove, or select, so raw TOML keystrokes start nothing. The
full `persisted_view` walk and the diff happen once per save inside the task.
Page-visible and window-focus events start a refresh. `pagehide`, page-hidden,
and window-blur events only request a save as a backstop, which writes
nothing when nothing changed.

`AppRoot` races opening and loading storage against a bounded timer, and
whichever settles first decides startup: the loader with the restored
workspace and its baseline, or the timer with bundled presets and an empty
baseline. A database that opens only after the timer has won is installed in
the shared slot so the window can save, but its load is never run, so a late
result can never be applied over what the user has already started doing. A
connection closes itself on `versionchange`, so an open window can never block
another window's upgrade; a window that has stopped running scripts still can,
which is why startup is bounded. One persistence-health signal is turned on
by any open, load, save, or refresh failure, by the startup timeout, and by
`versionchange`, and is never turned off. IndexedDB transactions stay alive
only while their futures resume from the microtask queue, so `storage::`
futures are awaited directly inside `spawn_local` tasks and must never be
wrapped in anything polled from a timer, animation-frame callback, or reactive
resource. The startup timer is a separate task for this reason.

## Export Behavior

Format availability and output behavior are defined in the
[export specification](specs/exports.md).

SVG export lives in `lsystem-core` behind the `svg` feature. PNG and APNG export
live in `lsystem-renderer` behind the `png` feature. APNG uploads geometry once
and changes uniforms per frame. Browser and native UI layers use app-specific
download/file plumbing around the shared renderer export APIs. Like the web
canvas, `offscreen.rs`'s `RenderTarget` owns a
`LINE_DEPTH_FORMAT` depth attachment and attaches it for 3D scenes, so
exported 3D images are depth-tested the same way the live web canvas is.

## Dependency Coupling

The workspace `wgpu` dependency is pinned to major version 29. Iced is pinned to
an upstream git revision that uses the same wgpu major version. Iced's shader
widget passes `wgpu` types (device, queue, render pass) to the custom primitive
at the crate boundary; mismatched major versions produce a compile-time type
error. Update those two dependencies together and verify native plus wasm
builds.

`lsystem-renderer`'s `wgsl_to_wgpu` build-dependency generates code against
`naga`/`wgpu-types` types that must match the workspace `wgpu` major version.
When updating `wgpu`, also check whether `wgsl_to_wgpu` needs a matching
version bump. The `wesl` build-dependency that compiles `src/shaders/*.wesl`
to WGSL runs at build time only and emits plain WGSL text, so it has no
naga/wgpu version coupling.
