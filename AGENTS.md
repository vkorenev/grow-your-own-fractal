# AGENTS.md

This file provides guidance to AI coding agents when working with code in this repository.

## Project Overview

**Grow Your Own Fractal** — an interactive L-System (Lindenmayer system) visualizer in Rust. The browser-first app (`lsystem-web-app`) uses Leptos/DOM controls with a WebGPU canvas backed by the toolkit-independent `lsystem-renderer` crate. The `lsystem-app` crate uses Iced for native desktop and retained wasm builds; its fractal viewport is an `iced::widget::shader` custom primitive backed by the shared wgpu line pipeline.

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

`trunk` is managed by mise; run `mise install` to get the pinned version from `mise.toml`.

## Supplemental Rules

Before running Git or GitHub CLI commands, read `.agents/rules/git-and-github.md`.

Successful CI runs on `main` trigger `.github/workflows/deploy.yml`, which deploys the Leptos browser app from `crates/lsystem-web-app/dist/` to GitHub Pages.

## Architecture

Four-crate workspace under `crates/`:

### `lsystem-core` — pure library, zero rendering deps

| Module | Role |
|--------|------|
| `config.rs` | Parses TOML `Config` struct (including `ColorConfig`/`LineColorConfig` for background and line colors); validates axiom, rules, step/angle finiteness, bracket balance |
| `alphabet.rs` | Reserved symbols (`F f + - \| [ ]`), character set validation |
| `grammar.rs` | `expand(axiom, rules, iterations)` → lazy `ExpandIter` char iterator; `expand_owned` → `OwnedExpandIter` (same logic, owns its data via `Vec<char>` so callers need no lifetime) |
| `turtle/mod.rs` | Declares `turtle2d` submodule; documents the 3D extension path (add `Segments3D<I>`, dispatch in `generate()` on `cfg.dimensions`) |
| `turtle/turtle2d.rs` | `Segments2D<I>` — pull iterator over `[Vec2; 2]` segments; owns position, heading, and bracket stack; yields one segment per `'F'` without collecting |
| `svg_export.rs` | `export_svg(config) -> String` — generates an SVG string; gated behind the `svg` Cargo feature |
| `lib.rs` | Public API: `generate(config) -> impl Iterator<Item = [Vec2; 2]>`; exposes `svg_export` as a public module when the `svg` feature is enabled |

Data flow: `Config` → `OwnedExpandIter` (owned lazy char rewriting) → `Segments2D` → streaming `[Vec2; 2]` segments.

### `lsystem-renderer` — toolkit-independent wgpu renderer

Depends on `lsystem-core` and `wgpu`.

| File | Role |
|------|------|
| `camera.rs` | Shared `Camera` pan/zoom state and view transform helpers used by both app crates |
| `line_renderer.rs` | `Transform` — scale + offset GPU uniform type. `GpuContext` — owns the wgpu surface for non-Iced canvas users; `begin_frame` returns explicit `FrameOutcome` values and `end_frame` submits + presents. `GpuInitError` preserves surface/adapter/device initialization failures. `LinePipeline` (pipeline, bind group, reusable vertex buffer, transform uniform, color-params uniform) — `upload()` grows/reuses the vertex buffer and writes `ColorParams`; `write_transform()` writes the camera transform every frame; `draw()` issues the line-list draw. |
| `lsystem_bridge.rs` | L-system→GPU adapters. `geometry_to_vertices()` accepts `impl Iterator<Item = [Vec2; 2]>` and produces a flat `Vertex` array with bounding box (`VertexData`). `color_params_from_config()` maps `LineColorConfig` to the `ColorParams` GPU uniform. |
| `png_export.rs` | Offscreen wgpu PNG renderer; gated behind the `png` Cargo feature |
| `wgpu_util.rs` | Shared wgpu instance/device descriptor and uncaptured-error logging helpers |
| `shader.wgsl` | Vertex shader applies a `Transform` uniform (scale + offset) and computes per-segment color from a `ColorParams` uniform using `vertex_index / 2`; supports solid, gradient, and HSV hue-cycle modes; fragment shader passes the interpolated color through; topology is `LineList` |

### `lsystem-app` — entry points and Iced UI

Depends on `lsystem-core`, `lsystem-renderer`, `iced`, and browser/native export support crates.

| File | Role |
|------|------|
| `main.rs` | Thin native entry that calls `lib.rs::run_native()` |
| `lib.rs` | Module declarations; `run_native()` starts the Iced app on desktop; `#[wasm_bindgen(start)] start()` starts the same Iced app on web |
| `ui.rs` | Iced UI module shell and shared UI constants |
| `ui/app_state.rs` | `FractalApp` state/update/view, preset/config controls, async geometry generation, stale-generation cancellation, exports, and pan/zoom messages |
| `ui/controls.rs` | Iced control panel widgets |
| `ui/fractal_canvas.rs` | `iced::widget::shader` integration, `Scene` camera/geometry state, viewport input handling, and GPU upload-by-scene-revision |
| `export.rs` | Native/browser SVG and PNG export helpers; PNG export creates an offscreen wgpu device instead of borrowing Iced's renderer device |

### `lsystem-web-app` — browser-first Leptos UI

Depends on `lsystem-core`, `lsystem-renderer`, `leptos`, and browser `web-sys`/`wasm-bindgen` APIs.

| File | Role |
|------|------|
| `lib.rs` | Leptos CSR entry point |
| `app.rs` | DOM controls for presets, TOML, overrides, viewport input, export buttons, and WebGPU error display |
| `presets.rs` | Embedded preset loading and effective-config helpers |
| `export.rs` | Browser SVG/PNG download helpers |
| `renderer.rs` | `CanvasRenderer` — owns the WebGPU canvas `GpuContext`, `LinePipeline`, shared `Camera`, current vertices, color params, and background; handles canvas resize, pan, zoom, reset, event-driven rendering, and web surface-loss recovery |
| `index.html` | Trunk entry that mounts the Leptos app |
| `Trunk.toml` | Browser app build config, served locally on `127.0.0.1:8081` |

### `presets/`

Bundled TOML L-System definitions. New fractals are added here; they are embedded at compile time via `include_dir!` in each app crate and auto-discovered — no registration step needed.

## Key Design Decisions

- **Streaming segment pipeline**: `generate()` returns a lazy `impl Iterator<Item = [Vec2; 2]>` — no intermediate `Vec<[Vec2; 2]>` is ever allocated. `OwnedExpandIter` (in `grammar.rs`) owns the axiom and rules as `Vec<char>` so the iterator carries no lifetime. `Segments2D` (in `turtle/turtle2d.rs`) yields one segment per `'F'` symbol, holding only position, heading, and a bracket stack. `geometry_to_vertices` streams the iterator directly into the GPU vertex buffer, so peak memory is one `Vec<Vertex>` rather than a segment vec plus a vertex vec simultaneously.
- **Lazy expansion**: `ExpandIter` / `OwnedExpandIter` avoid materializing the full rewritten string, keeping memory bounded for high-iteration fractals.
- **Dual target from day one**: `lsystem-core` has no platform-specific deps so it compiles for both native and `wasm32-unknown-unknown` without feature flags.
- **Iced/wgpu version coupling**: the workspace `wgpu` dependency is pinned to version 29 and Iced is pinned to a specific upstream git revision that uses the same wgpu major version. Do not independently bump `wgpu` or the Iced git revision; update them in lockstep and verify native + wasm builds.
- **3D forward-compat seam**: the `dimensions` TOML field (currently validated to `2` only) is the extension point. To add 3D: add `turtle/turtle3d.rs` with a `Segments3D<I>` iterator analogous to `Segments2D`, then dispatch in `lib.rs::generate()` based on `cfg.dimensions`. No other registration is needed.
- **Whitespace in axiom/rules is stripped**: whitespace inside `axiom` and rule RHS strings is removed before validation and expansion, allowing multi-line formatting in TOML configs.
- **Fractal lives in an Iced shader widget**: `lsystem-app` renders the fractal through `iced::widget::shader`. Iced owns the window, surface, event loop, and render pass; the custom primitive owns only the fractal GPU pipeline state.
- **Async scene generation**: `FractalApp` schedules geometry generation with `Task::perform` when presets, TOML, iterations, or angle change. Each request gets a monotonic generation token; stale results are ignored, so rapid slider changes do not block the UI with outdated work.
- **Scene-revision uploads**: `Scene` increments a revision whenever geometry changes. The Iced shader pipeline uploads vertices and color params only when the revision changes, while camera transforms are written during prepare.
- **Reusable vertex buffer**: `LinePipeline::upload` grows the GPU vertex buffer to the next power-of-two capacity when needed and otherwise updates it with `Queue::write_buffer`, avoiding a new buffer allocation on every geometry upload.
- **DOM browser UI with GPU canvas**: `lsystem-web-app` owns browser UI state in Leptos signals and renders the fractal into a dedicated `<canvas>`. It creates a WebGPU surface from `web_sys::HtmlCanvasElement`, reuses `LinePipeline`, and drives rendering from explicit DOM events instead of a continuous repaint loop.
- **Surface acquisition recovery**: `GpuContext::begin_frame` retries `CurrentSurfaceTexture::Outdated` once after reconfiguring the surface. Timeout and occlusion are quiet skip reasons, validation/repeated-outdated are explicit skip reasons, and true surface loss is reported to callers. The Leptos web renderer rebuilds `GpuContext` and `LinePipeline` after surface loss while preserving CPU-side scene/camera/color state and marking geometry for reupload.
- **SVG export is a `lsystem-core` Cargo feature**: The `svg` feature adds `svg_export::export_svg(config) -> String`. It collects segments into a `Vec` (the only allocation — acceptable for export), computes a padded bounding box, and builds SVG XML. The Y-axis flip (turtle is Y-up, SVG is Y-down) is handled by a `<g transform="matrix(1 0 0 -1 0 0)">` group so turtle coordinates are written as-is; the viewBox compensates. `stroke-width`, `stroke-linecap`, and `fill` are set on the `<g>` and inherited by children. Solid mode emits a single `<path>`; gradient and hue-cycle modes emit per-segment `<line>` elements to match the shader's segment-index-based coloring exactly. On native, `rfd::FileDialog` handles the save dialog; on WASM, a programmatic Blob download is triggered via `web-sys`.
- **Strict CI**: `clippy -D warnings` and `cargo fmt --check` must pass. CI tests the workspace with all features/all targets, checks and lints native default/all-features builds, checks and lints `wasm32-unknown-unknown` default/all-features builds, and builds both Trunk web apps. GitHub Pages deploys the Leptos browser app.
