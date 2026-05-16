# AGENTS.md

This file provides guidance to AI coding agents when working with code in this repository.

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
cargo clippy --workspace -- -D warnings
cargo clippy --workspace --all-features -- -D warnings
cargo test --workspace --all-features --all-targets
cargo check --target wasm32-unknown-unknown --workspace
cargo check --target wasm32-unknown-unknown --workspace --all-features
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

Before running Git or GitHub CLI commands, read `.agents/rules/git-and-github.md`.

When making code changes, check whether `README.md` and/or `AGENTS.md` need updates for the changed behavior, commands, architecture, or workflow. Update them in the same change when applicable.

Successful CI runs on `main` trigger `.github/workflows/deploy.yml`, which deploys the Leptos browser app from `crates/lsystem-web-app/dist/` to GitHub Pages.

## Architecture

Four-crate workspace under `crates/`:

### `lsystem-core` — pure library, zero rendering deps

| Module | Role |
|--------|------|
| `config.rs` | Parses nested TOML field paths into a format-preserving `ConfigDocument` backed by `toml_edit::DocumentMut`, accepts explicit tables, dotted keys, and implicit parent tables, validates to `Config` (`GenerationConfig` plus `ColorConfig`/`LineColorConfig`), serializes unchanged documents byte-for-byte, and validates symbols, rules, step/angle finiteness, bracket balance, and per-component RGB color ranges |
| `config_workspace.rs` | Shared session config workspace: tracks draft TOML, last-applied `ConfigDocument`/`Config`, and optional bundled default document per entry; supports selection retention, custom entry copying, apply, revert, reset, and dirty detection for both UIs |
| `alphabet.rs` | Reserved symbols (`F f + - \| [ ]` for 2D; additionally `& ^ / \` for 3D), character set validation per `dimensions` |
| `grammar.rs` | `expand(axiom, rules, iterations)` → lazy `ExpandIter` char iterator; `expand_owned` → `OwnedExpandIter` (same logic, owns its data via `Vec<char>` so callers need no lifetime) |
| `turtle/mod.rs` | Declares `turtle2d` and `turtle3d` submodules |
| `turtle/turtle2d.rs` | `Segments2D<I>` — pull iterator over `[Vec2; 2]` segments; owns position, heading, and bracket stack; yields one segment per `'F'` without collecting |
| `turtle/turtle3d.rs` | `Segments3D<I>` — pull iterator over `[Vec3; 2]` segments; uses `glam::Quat` orientation (heading = `orientation * Vec3::X`); dispatches `& ^ / \` pitch/roll symbols in addition to the 2D set |
| `svg_export.rs` | `export_svg(config) -> String` — generates an SVG string; gated behind the `svg` Cargo feature |
| `lib.rs` | Public API: `generate(generation_config) -> impl Iterator<Item = [Vec2; 2]>` and `generate_3d(generation_config) -> impl Iterator<Item = [Vec3; 2]>`; exposes `svg_export` when the `svg` feature is enabled |

Data flow (2D): `ConfigDocument` → `Config` → `GenerationConfig` → `OwnedExpandIter` → `Segments2D` → streaming `[Vec2; 2]` segments.
Data flow (3D): `ConfigDocument` → `Config` → `GenerationConfig` → `OwnedExpandIter` → `Segments3D` → streaming `[Vec3; 2]` segments.

### `lsystem-renderer` — toolkit-independent wgpu renderer

Depends on `lsystem-core` and `wgpu`.

| File | Role |
|------|------|
| `camera.rs` | Shared `Camera` — 2D pan/zoom state and 3D orbit (azimuth, elevation, roll, zoom) state; `compute_transform` for 2D, `compute_mvp_3d` for perspective 3D; `reset()` resets all state, `reset_position()` preserves rotation for scene rebuilds |
| `line_renderer.rs` | `Vertex2D`/`Vertex3D` GPU vertex types. `GrowableVertexBuffer` — grows to next power-of-two capacity on demand. `LinePipeline2D` — 2D `Transform` uniform + `LinePipeline3D` — `Mvp` (64-byte MVP matrix) uniform; both share `GrowableVertexBuffer` and a private `draw_line_list()` helper. `GpuContext`, `GpuInitError`, `FrameOutcome`, `SurfaceFrame`. `MAX_SEGMENTS` / `MAX_SEGMENTS_3D` caps for segment-count safety. |
| `lsystem_bridge.rs` | L-system→GPU adapters. `geometry_to_vertices()` for 2D (`VertexData`), `geometry_to_vertices_3d()` for 3D (`VertexData3D`). `color_params_from_config()` maps `LineColorConfig` to the `ColorParams` GPU uniform. |
| `png_export.rs` | Offscreen wgpu PNG renderer; gated behind the `png` Cargo feature |
| `wgpu_util.rs` | Shared wgpu instance/device descriptor and uncaptured-error logging helpers; browser wasm creates an instance with a web display handle so wgpu can use WebGPU or fall back to WebGL2, while native and Emscripten use the normal non-web instance path |
| `shader.wgsl` | 2D vertex shader: applies `Transform` (scale + offset), computes per-segment color from `ColorParams`; topology `LineList` |
| `shader3d.wgsl` | 3D vertex shader: applies `Mvp` matrix for perspective projection, same per-segment color logic as `shader.wgsl` |

### `lsystem-app` — entry points and Iced UI

Depends on `lsystem-core`, `lsystem-renderer`, `iced`, and browser/native export support crates.

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

Depends on `lsystem-core`, `lsystem-renderer`, `leptos`, and browser `web-sys`/`wasm-bindgen` APIs.

| File | Role |
|------|------|
| `lib.rs` | Leptos CSR entry point |
| `app.rs` | DOM controls for presets, TOML, overrides, viewport input, export buttons, and GPU rendering error display |
| `presets.rs` | Embedded preset loading and effective-config helpers |
| `export.rs` | Browser SVG/PNG download helpers |
| `renderer.rs` | `CanvasRenderer` — owns `GpuContext`, `LinePipeline2D`, `LinePipeline3D`, `Camera`, and an `ActiveScene` enum (2D or 3D); dispatches drag to pan (2D) or orbit (3D); handles canvas resize, zoom, orbit, roll, auto-rotate, reset, and surface-loss recovery |
| `index.html` | Trunk entry that mounts the Leptos app |
| `Trunk.toml` | Browser app build config, served locally on `127.0.0.1:8081` |

### `presets/`

Bundled TOML L-System definitions. New fractals are added here; they are embedded at compile time via `include_dir!` in each app crate and auto-discovered — no registration step needed.

## Key Design Decisions

- **Streaming segment pipeline**: `generate()` returns a lazy `impl Iterator<Item = [Vec2; 2]>` — no intermediate `Vec<[Vec2; 2]>` is ever allocated. `OwnedExpandIter` (in `grammar.rs`) owns the axiom and rules as `Vec<char>` so the iterator carries no lifetime. `Segments2D` (in `turtle/turtle2d.rs`) yields one segment per `'F'` symbol, holding only position, heading, and a bracket stack. `geometry_to_vertices` streams the iterator directly into the GPU vertex buffer, so peak memory is one `Vec<Vertex>` rather than a segment vec plus a vertex vec simultaneously.
- **Lazy expansion**: `ExpandIter` / `OwnedExpandIter` avoid materializing the full rewritten string, keeping memory bounded for high-iteration fractals.
- **Dual target from day one**: `lsystem-core` has no platform-specific deps so it compiles for both native and `wasm32-unknown-unknown` without feature flags.
- **Iced/wgpu version coupling**: the workspace `wgpu` dependency is pinned to version 29 and Iced is pinned to a specific upstream git revision that uses the same wgpu major version. Do not independently bump `wgpu` or the Iced git revision; update them in lockstep and verify native + wasm builds.
- **3D turtle uses quaternion orientation**: `Segments3D<I>` stores a `glam::Quat` orientation instead of a scalar heading angle. Heading = `orientation * Vec3::X`; left = `* Vec3::Y`; up = `* Vec3::Z`. Each rotation symbol applies `orientation *= Quat::from_rotation_*(angle)` in local space, so rotations compose correctly regardless of prior orientation.
- **Whitespace in axiom/rules is stripped**: whitespace inside `axiom` and rule RHS strings is removed before validation and expansion, allowing multi-line formatting in TOML configs.
- **Format-preserving config documents**: `ConfigDocument` owns a `toml_edit::DocumentMut`, so parsing and serializing an unchanged preset preserves comments, spacing, and string quoting byte-for-byte. Config parsing uses the nested v2 field paths but accepts explicit tables, dotted keys, and implicit parent tables. New canonical TOML uses explicit `[metadata]`, `[l-system]`, `[l-system.rules]`, `[turtle]`, `[colors]`, and `[colors.line]` tables.
- **Session config workspace**: `ConfigWorkspace` is the shared source of config editor state for both UIs. Each entry retains draft text independently from the last-applied document/config, Copy creates a renamed custom copy that preserves draft text separately from the last-applied config, failed Apply keeps the current rendered scene, Revert restores the last-applied text, Reset restores bundled defaults for preset entries when the last-applied text differs from the default, custom entries have no bundled default, and dirty drafts disable config-affecting controls until Apply/Revert.
- **Hue-cycle config uses RGB input**: `LineColorConfig::HueCycle { initial }` stores the starting color as an RGB array. SVG export and the renderer derive HSV parameters from that RGB value at the output boundary.
- **Fractal lives in an Iced shader widget**: `lsystem-app` renders the fractal through `iced::widget::shader`. Iced owns the window, surface, event loop, and render pass; the custom primitive owns only the fractal GPU pipeline state.
- **Async scene generation**: `FractalApp` schedules geometry generation with `Task::perform` when presets, TOML, iterations, or angle change. Each request gets a monotonic generation token; stale results are ignored, so rapid slider changes do not block the UI with outdated work.
- **Scene-revision uploads**: `FractalApp::schedule_scene_generation()` increments a monotonic generation token whenever geometry is requested. Completed scene builds store that token as `Scene::revision`; the Iced shader pipeline uploads vertices and color params only when the observed revision changes, while camera transforms are written during prepare.
- **Reusable vertex buffer**: `GrowableVertexBuffer` grows the GPU vertex buffer to the next power-of-two capacity when needed and otherwise updates it with `Queue::write_buffer`. Both `LinePipeline2D` and `LinePipeline3D` use it via a generic `upload<V: Pod>` method.
- **2D/3D pipeline split**: `LinePipeline2D` and `LinePipeline3D` are separate structs with different uniform types (`Transform` vs `Mvp`) and different shaders, but share `GrowableVertexBuffer` and a private `draw_line_list()` helper to avoid duplication. `lsystem-web-app`'s `CanvasRenderer` owns both pipelines and keeps both always-initialized; `lsystem-app`'s `Scene` holds an enum that selects the active pipeline at draw time.
- **DOM browser UI with GPU canvas**: `lsystem-web-app` owns browser UI state in Leptos signals and renders the fractal into a dedicated `<canvas>`. It creates a wgpu surface from `web_sys::HtmlCanvasElement`, reuses `LinePipeline`, and drives rendering from explicit DOM events instead of a continuous repaint loop. On browser wasm targets, `wgpu_util` creates the instance with a web display handle and WebGPU detection so wgpu can use WebGPU when available and fall back to WebGL2 otherwise.
- **Surface acquisition recovery**: `GpuContext::begin_frame` retries `CurrentSurfaceTexture::Outdated` once after reconfiguring the surface. Timeout and occlusion are quiet skip reasons, validation/repeated-outdated are explicit skip reasons, and true surface loss is reported to callers. The Leptos web renderer rebuilds `GpuContext` and `LinePipeline` after surface loss while preserving CPU-side scene/camera/color state and marking geometry for reupload.
- **SVG export is 2D-only**: SVG export is a `lsystem-core` Cargo feature (`svg`). `export_svg(config) -> String` collects 2D segments, computes a padded bounding box, and builds SVG XML. The Y-axis flip is handled by a `<g transform="matrix(1 0 0 -1 0 0)">` group. Both apps hide the SVG export button when `config.dimensions == 3`.
- **Strict CI**: `clippy -D warnings` and `cargo fmt --check` must pass. CI tests the workspace with all features/all targets, checks and lints native default/all-features builds, checks and lints `wasm32-unknown-unknown` default/all-features builds, and builds both Trunk web apps. GitHub Pages deploys the Leptos browser app.
