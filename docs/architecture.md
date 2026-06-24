# Architecture

Grow Your Own Fractal is a Rust workspace for generating, editing, rendering,
and exporting 2D and 3D L-System fractals. The core model is toolkit-agnostic:
grammar expansion and turtle geometry live in `lsystem-core`, shared
application state lives in `lsystem-app-model`, and GPU rendering/export support
lives in `lsystem-renderer`. The native/Iced and browser-first/Leptos apps are
thin UI layers over those shared crates.

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
  -> OwnedExpandIter
  -> Segments2D / Segments3D
  -> renderer bridge segment records
  -> wgpu instance buffer
  -> GPU bounds reduction for camera/export fitting
```

The expansion and turtle layers are streaming iterators. They do not build the
full expanded string or an intermediate vertex list before yielding geometry.
The renderer bridge collects only the GPU instance records needed for upload;
renderer bounds are reduced from those records with GPU compute after upload,
falling back to a CPU reduction only when the selected adapter lacks compute
shader support. Do not introduce another collection of expanded symbols, raw
geometry, or vertices; the renderer bridge's segment-instance `Vec` is the
intentional collection point before GPU upload. Adding another large collection
breaks the memory-bounded property for high iteration counts.

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

## Core Model

`lsystem-core` owns the grammar and turtle semantics:

- `alphabet.rs` validates reserved symbols for 2D and 3D.
- `grammar.rs` provides lazy borrowed and owned expansion iterators.
- `turtle/turtle2d.rs` yields 2D line segments from expanded symbols.
- `turtle/turtle3d.rs` yields 3D line segments using quaternion orientation.
- `config.rs` defines validated runtime config and color types.
- `svg_export.rs` exports resolved 2D configs when the `svg` feature is enabled.

3D turtle orientation is stored as a `glam::Quat`. Heading, left, and up vectors
are derived from that orientation, and pitch/roll/yaw symbols compose in local
space. This avoids the accumulation and ordering problems that come from
tracking heading as a single scalar angle.

Whitespace inside `axiom` and rule right-hand-side strings is stripped before
validation and expansion. This keeps long TOML rules readable without changing
the generated grammar.

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
  names.
- `presets.rs` embeds and sorts the `presets/` directory.
- `color.rs` centralizes line-color mode selection and per-mode picker memory.
- `animation.rs` contains hue-rotation state and phase advancement.

Omitting `colors.line` deliberately resolves to a solid default color. Because
line colors use an externally tagged TOML shape (`solid`, `gradient`,
`hue_cycle`), a non-solid default would make solid mode unreachable by omission.

## Rendering

`lsystem-renderer` owns the shared wgpu machinery used by both apps and by
offscreen exports.

- `build.rs` generates Rust bindings from `shader.wgsl` at build time via
  `wgsl_to_wgpu`, validating `shader.wgsl` before compiling
  `lsystem-renderer`'s Rust sources. `line_renderer.rs` sources its uniform
  types (`ColorParams`, `Transform`, `Mvp`) and shader entry-point constants
  from these generated bindings rather than hand-mirroring them, so most
  field, binding, and entry-point renames in `shader.wgsl` now fail the Rust
  build instead of only surfacing as a runtime wgpu validation error. Bind
  group layouts, bind groups, vertex instance records, vertex buffer layouts,
  and shader entry states are built from generated helpers. Pipeline layouts
  are still assembled by hand because the 2D and 3D pipelines bind different,
  sparse subsets of the shader's three bind groups.
- `camera.rs` supports 2D pan/zoom and 3D orbit/elevation/roll/zoom.
- `line_renderer.rs` defines GPU instance records, growable storage-capable
  vertex buffers, 2D/3D line pipelines, color uniforms, and surface frame
  handling.
- `lsystem_bridge.rs` converts core geometry iterators into GPU segment data and
  maps `LineColorConfig` into shader color parameters.
- `bounds_compute.rs` reduces uploaded segment buffers to 2D/3D bounds with a
  compute shader and async readback; SVG export keeps its separate CPU bounds
  loop in `lsystem-core`.
- `offscreen.rs`, `png_export.rs`, and `animation_export.rs` render PNG/APNG
  output with an offscreen target behind the `png` feature.
- `wgpu_util.rs` centralizes instance/device setup and error logging for native
  and browser targets.

Line rendering is instanced: one GPU record represents one segment, and the
shader selects the start or end point from `vertex_index`. Buffers grow to the
next power-of-two capacity and are reused through `Queue::write_buffer`.

The 2D and 3D line pipelines are separate because they use different vertex
entry points, vertex-buffer layouts, and transform uniforms within the shared
`shader.wgsl` module. They share the same color uniform model and
segment-buffer strategy.

## Color And Depth

Line colors support solid RGB, traversal-order gradient, topological-depth
gradient, and hue-cycle modes. Topological-depth gradient uses turtle branch
depth: drawn `F` segments advance depth, `f` does not, and bracket push/pop
restores depth along with turtle state.

Depth-aware geometry is generated only when the grammar has stack directives.
At the render/export boundary, `color_params_from_config` selects
topological-depth shader mode only when the config asks for it and depth
geometry is actually available. Bracketless grammars therefore fall back to
traversal-order gradient even if `topological_depth = true` is authored.

Hue rotation is transient UI state. It changes the color uniform for hue-cycle
rendering, but it does not mutate TOML, presets, geometry, exports, or the config
schema.

## App Layers

`lsystem-app` uses Iced for native desktop and retained-mode wasm builds. Iced
owns the window, surface, event loop, and render pass; the fractal shader widget
owns only the fractal GPU pipeline state. Geometry generation is asynchronous
and tokenized so stale generation results can be ignored after rapid input
changes. Geometry and color revisions let the shader upload segment data only
when geometry changes and update only color uniforms for color-only edits.

`lsystem-web-app` uses Leptos for DOM controls and renders into a dedicated
canvas. The renderer owns both 2D and 3D pipelines, handles resize/zoom/orbit/
roll/auto-rotate/reset operations, and rebuilds GPU state after surface loss
while preserving CPU-side scene, camera, and color state.

## Export Behavior

SVG export is 2D-only and lives in `lsystem-core` behind the `svg` feature. Both
apps hide SVG export when `dimensions = "3D"`.

PNG and APNG export live in `lsystem-renderer` behind the `png` feature. APNG
uploads geometry once and changes uniforms per frame. Browser and native UI
layers use app-specific download/file plumbing around the shared renderer export
APIs.

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
version bump.
