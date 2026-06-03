# AGENTS.md

This file provides guidance to AI coding agents when working with code in this repository.

> **REQUIRED before any interaction with Git or GitHub** — including `git` commands, the `gh` CLI, GitHub MCP tools, or any other mechanism that reads or writes repository or pull-request state: read `.agents/rules/git-and-github.md`.

## Project Overview

**Grow Your Own Fractal** — an interactive L-System (Lindenmayer system) visualizer in Rust. The browser-first app (`lsystem-web-app`) uses Leptos/DOM controls with a wgpu canvas that uses WebGPU with a WebGL2 fallback on browser wasm targets, backed by the toolkit-independent `lsystem-renderer` crate. The `lsystem-app` crate uses Iced for native desktop and retained wasm builds; its fractal viewport is an `iced::widget::shader` custom primitive backed by the shared wgpu line pipeline.

## Common Commands

```bash
# Build & run
cargo run -p lsystem-app          # native desktop Iced app
trunk serve --config crates/lsystem-web-app/Trunk.toml    # browser app at localhost:8081
trunk build --release --config crates/lsystem-web-app/Trunk.toml  # browser release → crates/lsystem-web-app/dist/
trunk serve --config crates/lsystem-app/Trunk.toml    # Iced web app at localhost:8080

# Verification (all run in CI)
cargo fmt --check --all
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy --workspace --all-features --all-targets -- -D warnings
cargo test --workspace --all-features --all-targets
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace --all-features
cargo clippy --target wasm32-unknown-unknown --workspace -- -D warnings
cargo clippy --target wasm32-unknown-unknown --workspace --all-features -- -D warnings
trunk build --release --config crates/lsystem-app/Trunk.toml
trunk build --release --config crates/lsystem-web-app/Trunk.toml

# Run a single test
cargo test -p lsystem-core config::tests::test_name

# Run SVG export tests (svg feature must be enabled explicitly)
cargo test -p lsystem-core --features svg svg_export
```

`trunk` is managed by mise; run `mise install` to get the pinned version from `mise.toml`. `trunk` may fail to launch when `NO_COLOR=1` is present in the environment; use `NO_COLOR=true` or `NO_COLOR=false` as a workaround.

## Supplemental Rules

When making code changes, check whether `README.md`, `CONTRIBUTING.md`, and/or `AGENTS.md` need updates for the changed behavior, commands, architecture, or workflow. Update them in the same change when applicable.

Successful CI runs on `main` trigger `.github/workflows/deploy.yml`, which deploys the Leptos browser app from `crates/lsystem-web-app/dist/` to GitHub Pages.

## Architecture

Five-crate workspace under `crates/`:

### `lsystem-core` — pure library, zero rendering deps

| Module | Role |
|--------|------|
| `config.rs` | Two-phase config pipeline: `ConfigSource` wraps a `toml_edit::DocumentMut` (format-preserving, parse-only); `TryFrom<ConfigSource> for ConfigDocument` validates and produces a `ConfigDocument` that pairs the source with its validated `Config`; `From<ConfigDocument> for Config` extracts ownership. Validates symbols, rules, step/angle finiteness, bracket balance, optional background color, line colors including `depth_gradient`, and hex color strings, then stores colors as validated `Rgb` values |
| `alphabet.rs` | Reserved symbols (`F f + - \| [ ]` for 2D; additionally `& ^ / \` for 3D), character set validation per `dimensions`; `contains_3d_symbols(s: &str) -> bool` re-exported from `lsystem-core` |
| `grammar.rs` | `expand(axiom, rules, iterations)` → lazy `ExpandIter` char iterator; `expand_owned` → `OwnedExpandIter` (same logic, owns its data via `Vec<char>` so callers need no lifetime) |
| `turtle/mod.rs` | Declares `turtle2d` and `turtle3d` submodules |
| `turtle/turtle2d.rs` | `Segments2D<I>` — pull iterator over `[Vec2; 2]` segments; owns position, heading, and bracket stack; yields one segment per `'F'` without collecting |
| `turtle/turtle3d.rs` | `Segments3D<I>` — pull iterator over `[Vec3; 2]` segments; uses `glam::Quat` orientation (heading = `orientation * Vec3::X`); dispatches `& ^ / \` pitch/roll symbols in addition to the 2D set |
| `svg_export.rs` | `export_svg(config) -> String` — generates an SVG string; gated behind the `svg` Cargo feature |
| `lib.rs` | Public API: `generate(generation_config) -> impl Iterator<Item = [Vec2; 2]>`, `generate_3d(generation_config) -> impl Iterator<Item = [Vec3; 2]>`, `generate_with_topological_depth(generation_config) -> impl Iterator<Item = Segment2DWithTopologicalDepth>`, and `generate_3d_with_topological_depth(generation_config) -> impl Iterator<Item = Segment3DWithTopologicalDepth>`; re-exports `contains_3d_symbols`, `Rgb`, and `RgbError`; exposes `svg_export` when the `svg` feature is enabled |

Data flow (2D): `ConfigSource::parse` → `ConfigDocument::try_from` → `Config::from` → `GenerationConfig` → `OwnedExpandIter` → `Segments2D` → streaming `[Vec2; 2]` segments.
Data flow (3D): `ConfigSource::parse` → `ConfigDocument::try_from` → `Config::from` → `GenerationConfig` → `OwnedExpandIter` → `Segments3D` → streaming `[Vec3; 2]` segments.

### `lsystem-app-model` — toolkit-independent application model

Depends on `lsystem-core`. Renderer-free and toolkit-free; must not depend on `iced`, `leptos`, `web-sys`, `wasm-bindgen`, `wgpu`, or `lsystem-renderer`.

| Module | Role |
|--------|------|
| `config_workspace.rs` | Moved from `lsystem-core`: shared session config workspace; tracks optional draft TOML, a last-applied `ConfigDocument`, and an optional bundled default document per entry; exposes indexed copy/apply/revert/reset operations while each UI owns selection |
| `presets.rs` | `load_presets() -> Vec<(String, String)>` — embeds the `presets/` directory at compile time via `include_dir!` and returns sorted `(label, TOML text)` pairs; replaces identical helpers that previously lived in each GUI crate |
| `color.rs` | `LineColorMode` — unified enum with `ALL`, `Display`, `from_key`/`key`, `from_line_color`, `default_line_color`; `ColorControlMemory` — per-mode `Rgb` slot store for UI color picker memory |
| `animation.rs` | `HueRotation` (no `phase_degrees` field), `HueRotationDirection` (with `ALL`, `Display`, `sign`), speed constants, and `advance_hue_rotation_phase_degrees(phase, speed, dt, direction) -> f32` free function; phase accumulator stored separately by each GUI |
| `util.rs` | `sanitize_filename` — pure filename utility with no toolkit dependencies |

### `lsystem-renderer` — toolkit-independent wgpu renderer

Depends on `lsystem-core` and `wgpu`.

| File | Role |
|------|------|
| `camera.rs` | Shared `Camera` — 2D pan/zoom state and 3D orbit (azimuth, elevation, roll, zoom) state; `compute_transform` for 2D, `compute_mvp_3d` for perspective 3D; `reset()` resets all state, `reset_position()` preserves rotation for scene rebuilds |
| `line_renderer.rs` | `Segment2D`/`Segment3D` GPU instance types plus `TopologicalDepthSegment2D`/`TopologicalDepthSegment3D` for depth-aware rendering. `GrowableVertexBuffer` — grows to next power-of-two capacity on demand and counts segment instances. Vertex instance buffers remain `bytemuck`-cast records, while `Transform`, `Mvp`, and `ColorParams` uniform writes use `encase::ShaderType` WGSL layout serialization. `LinePipeline2D` — 2D `Transform` uniform + `LinePipeline3D` — `Mvp` (64-byte MVP matrix) uniform; both own normal and topological-depth render pipelines that each serve all color modes, and share a private `draw_line_list()` helper using `draw(0..2, 0..segment_count)`. `write_color_params()` updates only the color uniform without re-uploading segment data. `GpuContext`, `GpuInitError`, `FrameOutcome`, `SurfaceFrame`. Dimension and stack-directive-aware segment-count caps keep normal and depth segment buffers within wgpu's guaranteed 256 MiB vertex-buffer limit. |
| `lsystem_bridge.rs` | L-system→GPU adapters. `geometry_to_segments()` for 2D (`SegmentData`), `geometry_to_segments_3d()` for 3D (`SegmentData3D`), and topological-depth variants that preserve per-segment depth and compute `max_topological_depth`. `color_params_from_config()` maps `LineColorConfig` to the `ColorParams` GPU uniform, including `ColorParams.max_topological_depth` for `depth_gradient`. |
| `png_export.rs` | Offscreen wgpu PNG renderer; gated behind the `png` Cargo feature |
| `wgpu_util.rs` | Shared wgpu instance/device descriptor and uncaptured-error logging helpers; browser wasm creates an instance with a web display handle so wgpu can use WebGPU or fall back to WebGL2, while native and Emscripten use the normal non-web instance path |
| `shader.wgsl` | 2D vertex shader: applies `Transform` (scale + offset), computes per-segment color from `ColorParams`; topology `LineList` |
| `shader3d.wgsl` | 3D vertex shader: applies `Mvp` matrix for perspective projection, same per-segment color logic as `shader.wgsl` |

### `lsystem-app` — entry points and Iced UI

Depends on `lsystem-core`, `lsystem-app-model`, `lsystem-renderer`, `iced`, and browser/native export support crates.

| File | Role |
|------|------|
| `main.rs` | Thin native entry that calls `lib.rs::run_native()` |
| `lib.rs` | Module declarations; `run_native()` starts the Iced app on desktop; `#[wasm_bindgen(start)] start()` starts the same Iced app on web |
| `ui.rs` | Iced UI module shell and shared UI constants |
| `ui/app_state.rs` | `FractalApp` state/update/view, preset/config controls, async geometry generation, stale-generation cancellation, exports, and pan/zoom messages |
| `ui/controls.rs` | Iced control panel widgets; hides SVG export and shows auto-rotate controls for 3D scenes |
| `ui/fractal_canvas.rs` | `iced::widget::shader` integration; `Scene` holds either 2D or 3D geometry plus camera; mouse drag orbits in 3D, pans in 2D; GPU upload-by-scene-revision |
| `export.rs` | Native/browser SVG and PNG export helpers; PNG export creates an offscreen wgpu device instead of borrowing Iced's renderer device |

### `lsystem-web-app` — browser-first Leptos UI

Depends on `lsystem-core`, `lsystem-app-model`, `lsystem-renderer`, `leptos`, and browser `web-sys`/`wasm-bindgen` APIs.

| File | Role |
|------|------|
| `lib.rs` | Leptos CSR entry point |
| `app.rs` | DOM controls for presets, TOML, overrides, viewport input, export buttons, and GPU rendering error display |
| `presets.rs` | Effective-config helpers (`max_iterations_for_config`); preset loading delegated to `lsystem_app_model::load_presets` |
| `export.rs` | Browser SVG/PNG download helpers |
| `renderer.rs` | `CanvasRenderer` — owns `GpuContext`, `LinePipeline2D`, `LinePipeline3D`, `Camera`, and an `ActiveScene` enum (2D or 3D); dispatches drag to pan (2D) or orbit (3D); handles canvas resize, zoom, orbit, roll, auto-rotate, reset, and surface-loss recovery |
| `index.html` | Trunk entry that mounts the Leptos app |
| `Trunk.toml` | Browser app build config, served locally on `127.0.0.1:8081` |

### `presets/`

Bundled TOML L-System definitions. New fractals are added here; they are embedded at compile time via `include_dir!` in `lsystem-app-model` (via `load_presets()`) and auto-discovered — no registration step needed.

## Key Design Decisions

- **Streaming segment pipeline**: `generate()` returns a lazy `impl Iterator<Item = [Vec2; 2]>` — no intermediate `Vec<[Vec2; 2]>` is ever allocated. `OwnedExpandIter` (in `grammar.rs`) owns the axiom and rules as `Vec<char>` so the iterator carries no lifetime. `Segments2D` (in `turtle/turtle2d.rs`) yields one segment per `'F'` symbol, holding only position, heading, and a bracket stack. `geometry_to_segments` streams the iterator directly into the GPU segment instance buffer, so peak memory is one `Vec<Segment2D>` rather than a segment vec plus a vertex vec simultaneously.
- **Lazy expansion**: `ExpandIter` / `OwnedExpandIter` avoid materializing the full rewritten string, keeping memory bounded for high-iteration fractals.
- **Dual target from day one**: `lsystem-core` has no platform-specific deps so it compiles for both native and `wasm32-unknown-unknown` without feature flags.
- **Iced/wgpu version coupling**: the workspace `wgpu` dependency is pinned to version 29 and Iced is pinned to a specific upstream git revision that uses the same wgpu major version. Do not independently bump `wgpu` or the Iced git revision; update them in lockstep and verify native + wasm builds.
- **3D turtle uses quaternion orientation**: `Segments3D<I>` stores a `glam::Quat` orientation instead of a scalar heading angle. Heading = `orientation * Vec3::X`; left = `* Vec3::Y`; up = `* Vec3::Z`. Each rotation symbol applies `orientation *= Quat::from_rotation_*(angle)` in local space, so rotations compose correctly regardless of prior orientation.
- **Whitespace in axiom/rules is stripped**: whitespace inside `axiom` and rule RHS strings is removed before validation and expansion, allowing multi-line formatting in TOML configs.
- **Two-phase config parse/validate**: `ConfigSource` (a `toml_edit::DocumentMut` wrapper) is the output of `ConfigSource::parse`; it is format-preserving but carries no validity guarantee. `TryFrom<ConfigSource> for ConfigDocument` validates the document and, on success, stores both the source and the resulting `Config` together. Holding a `ConfigDocument` is a runtime invariant that `config()` always returns a valid value. `From<ConfigDocument> for Config` transfers ownership of the cached `Config` without re-parsing. Config parsing accepts nested v2 field paths, explicit tables, dotted keys, and implicit parent tables; canonical TOML uses explicit `[metadata]`, `[l-system]`, `[l-system.rules]`, `[turtle]`, `[colors]`, and `[colors.line]` tables. `colors.background` is optional; missing background is stored as `None` and render/export paths fall back to black. Color values in TOML use `"#rrggbb"` hex strings.
- **Session config workspace**: `ConfigWorkspace` (in `lsystem-app-model`) is the shared source of config entry state for both UIs, while each client owns its selected entry index. Each entry stores only an optional dirty draft, a last-applied `ConfigDocument`, and an optional bundled default document; names, applied text, and `Config` values are derived from those on demand. Indexed Copy creates a renamed custom copy that preserves dirty draft text separately from the last-applied document and returns the new entry index, failed Apply keeps the current rendered scene, Revert drops the dirty draft, Reset restores bundled defaults for preset entries when the last-applied document differs from the default, custom entries have no bundled default, clean iteration/angle/background/line color changes update the entry's last-applied TOML through validated `ConfigEntry` setters, and dirty drafts disable config-affecting controls until Apply/Revert.
- **Line color modes**: `LineColorConfig` supports solid RGB, traversal-order gradient, hue-cycle, and topological-depth gradient. Color fields are validated `Rgb` values; TOML uses `"#rrggbb"` hex strings; `HueCycle { initial }` derives HSV at the output boundary. Both UIs also expose transient hue rotation for hue-cycle mode; it advances a phase value and writes an adjusted `hue_start` color uniform, without changing TOML, presets, geometry, exports, or the config schema. The rotation state is preserved but ignored while another line color mode is active. `DepthGradient { start, end }` uses turtle topological depth: the first drawn `F` segment is depth 0, each drawn `F` increments depth, `f` does not, and bracket stack push/pop restores depth with turtle position and orientation. Topological-depth geometry (`TopologicalDepthSegment2D`/`3D`) is generated when `GenerationConfig::has_stack_directives()` is true — i.e., the axiom or any rule RHS contains `[` — independent of the active color mode. On bracketless fractals `DepthGradient` is normalized to `Gradient` by `effective_colors()` before reaching the shader; the two modes produce identical output because on bracketless fractals topological depth equals segment index. All render and export paths call `Config::effective_colors()` instead of reading `config.colors` directly; `effective_colors()` normalizes `DepthGradient` to `Gradient` for bracketless fractals, ensuring PNG/SVG export never allocates the larger depth-segment buffer when it is not needed. The stored `Config.colors` and the UI color-mode picker are unaffected — a user may select `DepthGradient` before adding brackets, and the correct depth geometry activates as soon as the fractal gains a stack directive. Color-mode changes and hue rotation never trigger geometry recomputation; both pipelines handle all color modes by dispatching on `color_params.mode` in the vertex shader.
- **Fractal lives in an Iced shader widget**: `lsystem-app` renders the fractal through `iced::widget::shader`. Iced owns the window, surface, event loop, and render pass; the custom primitive owns only the fractal GPU pipeline state.
- **Async scene generation**: `FractalApp` schedules geometry generation with `Task::perform` when presets, TOML, iterations, or angle change. Each request gets a monotonic generation token; stale results are ignored, so rapid slider changes do not block the UI with outdated work.
- **Scene-revision uploads**: `FractalApp::schedule_scene_generation()` increments a monotonic generation token whenever geometry is requested. Completed scene builds store that token as `Scene::geometry_revision`; color-only changes increment `Scene::color_revision` independently. The Iced shader pipeline re-uploads segment instances and color params when `geometry_revision` changes, writes only the color params uniform when `color_revision` changes, and skips both when neither changes. Camera transforms are always written during prepare.
- **Reusable segment instance buffer**: `GrowableVertexBuffer` grows the GPU vertex buffer to the next power-of-two capacity when needed and otherwise updates it with `Queue::write_buffer`. Both `LinePipeline2D` and `LinePipeline3D` use it via a generic `upload<V: Pod>` method. Line rendering is instanced: one GPU record per segment, shader `vertex_index` selects start/end, and `instance_index` drives traversal gradient and hue-cycle colors.
- **2D/3D pipeline split**: `LinePipeline2D` and `LinePipeline3D` are separate structs with different uniform types (`Transform` vs `Mvp`) and different shaders, but share `GrowableVertexBuffer` and a private `draw_line_list()` helper to avoid duplication. `lsystem-web-app`'s `CanvasRenderer` owns both pipelines and keeps both always-initialized; `lsystem-app`'s `Scene` holds an enum that selects the active pipeline at draw time.
- **DOM browser UI with GPU canvas**: `lsystem-web-app` owns browser UI state in Leptos signals and renders the fractal into a dedicated `<canvas>`. It creates a wgpu surface from `web_sys::HtmlCanvasElement`, reuses `LinePipeline`, and drives rendering from explicit DOM events instead of a continuous repaint loop. On browser wasm targets, `wgpu_util` creates the instance with a web display handle and WebGPU detection so wgpu can use WebGPU when available and fall back to WebGL2 otherwise.
- **Surface acquisition recovery**: `GpuContext::begin_frame` retries `CurrentSurfaceTexture::Outdated` once after reconfiguring the surface. Timeout and occlusion are quiet skip reasons, validation/repeated-outdated are explicit skip reasons, and true surface loss is reported to callers. The Leptos web renderer rebuilds `GpuContext` and `LinePipeline` after surface loss while preserving CPU-side scene/camera/color state and marking geometry for reupload.
- **SVG export is 2D-only**: SVG export is a `lsystem-core` Cargo feature (`svg`). `export_svg(config) -> String` collects 2D segments, computes a padded bounding box, and builds SVG XML. For `depth_gradient`, it collects topological-depth segments and colors by depth normalized to the maximum emitted topological depth so SVG matches canvas rendering. The Y-axis flip is handled by a `<g transform="matrix(1 0 0 -1 0 0)">` group. Both apps hide the SVG export button when `config.dimensions == 3`.
- **Strict CI**: `clippy -D warnings` and `cargo fmt --check` must pass. A single Clippy job lints native default/all-features builds (with `--all-targets`) and `wasm32-unknown-unknown` default/all-features builds. CI also tests the workspace with all features/all targets, builds rustdoc with `RUSTDOCFLAGS=-D warnings`, and builds both Trunk web apps. Toolchain channel/components/targets come from `rust-toolchain.toml`; rustup auto-installs them on first cargo invocation. GitHub Pages deploys the Leptos browser app.
