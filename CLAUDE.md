# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**Grow Your Own Fractal** — an interactive L-System (Lindenmayer system) visualizer in Rust. Supports both native desktop and browser (WebAssembly/WebGPU) from a shared codebase. The fractal is rendered via an `egui_wgpu::CallbackTrait` adapter (in `ui.rs`) that bridges egui's paint-callback system to a toolkit-independent wgpu pipeline (`lsystem-renderer` crate); layout, hit-testing, and z-order all flow through egui, and `renderer.rs` is a thin winit orchestration layer that owns the surface and dispatches frames.

## Common Commands

```bash
# Build & run
cargo run -p lsystem-app          # native desktop
trunk serve --config crates/lsystem-app/Trunk.toml    # web dev server at localhost:8080
trunk build --release --config crates/lsystem-app/Trunk.toml  # web release build → crates/lsystem-app/dist/

# Verification (all run in CI)
cargo test --workspace
cargo fmt --check --all
cargo clippy --workspace -- -D warnings
cargo check --target wasm32-unknown-unknown -p lsystem-app

# Run a single test
cargo test -p lsystem-core config::tests::test_name

# Run SVG export tests (svg feature must be enabled explicitly)
cargo test -p lsystem-core --features svg svg_export
```

`trunk` is managed by mise; run `mise install` to get the pinned version from `mise.toml`.

## Architecture

Three-crate workspace under `crates/`:

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
| `line_renderer.rs` | `Transform` — scale + offset GPU uniform type. `GpuContext` — owns the wgpu surface; `begin_frame` acquires the next surface texture and `end_frame` submits + presents. `LinePipeline` (pipeline, bind group, vertex buffer, transform uniform, color-params uniform) — `upload()` re-uploads vertices and `ColorParams`; `write_transform()` writes the camera transform every frame; `draw()` issues the line-list draw. On wasm `GpuContext` is built asynchronously and delivered via `UserEvent::GpuReady` |
| `lsystem_bridge.rs` | L-system→GPU adapters. `geometry_to_vertices()` accepts `impl Iterator<Item = [Vec2; 2]>` and produces a flat `Vertex` array with bounding box (`VertexData`). `color_params_from_config()` maps `LineColorConfig` to the `ColorParams` GPU uniform. |
| `shader.wgsl` | Vertex shader applies a `Transform` uniform (scale + offset) and computes per-segment color from a `ColorParams` uniform using `vertex_index / 2`; supports solid, gradient, and HSV hue-cycle modes; fragment shader passes the interpolated color through; topology is `LineList` |

### `lsystem-app` — entry points and egui UI

Depends on `lsystem-core`, `lsystem-renderer`, `egui`/`egui-wgpu`/`egui-winit`, and `winit`.

| File | Role |
|------|------|
| `main.rs` | Thin native entry that calls `lib.rs::run_native()` |
| `lib.rs` | Module declarations; `run_native()` builds an `EventLoop<UserEvent>` and calls `run_app`; `#[wasm_bindgen(start)] start()` does the same on web via `EventLoopExtWebSys::spawn_app` |
| `renderer.rs` | `App` (`ApplicationHandler<UserEvent>`) — owns `Camera`, geometry buffer, side-panel state. Routes `WindowEvent::RedrawRequested` straight to its own renderer; routes everything else through `egui-winit` |
| `ui.rs` | `UiState` (preset/config state, egui layout including the central fractal canvas via `ui.allocate_painter()`, pan/zoom from the painter `Response`) + `EguiRenderer` (egui context, egui-wgpu integration, single render pass that does both the surface clear and the fractal+egui draw) + `FractalCallback` (per-frame data struct: vertices, transform, needs_upload, color_params) + `impl egui_wgpu::CallbackTrait for FractalCallback` (thin egui adapter that delegates to `LinePipeline::upload/write_transform/draw`) |
| `camera.rs` | `Camera` (pan/zoom state), `compute_transform` |

### `presets/`

Bundled TOML L-System definitions. New fractals are added here; they are embedded at compile time via `include_dir!` in `ui.rs` and auto-discovered — no registration step needed.

## Key Design Decisions

- **Streaming segment pipeline**: `generate()` returns a lazy `impl Iterator<Item = [Vec2; 2]>` — no intermediate `Vec<[Vec2; 2]>` is ever allocated. `OwnedExpandIter` (in `grammar.rs`) owns the axiom and rules as `Vec<char>` so the iterator carries no lifetime. `Segments2D` (in `turtle/turtle2d.rs`) yields one segment per `'F'` symbol, holding only position, heading, and a bracket stack. `geometry_to_vertices` streams the iterator directly into the GPU vertex buffer, so peak memory is one `Vec<Vertex>` rather than a segment vec plus a vertex vec simultaneously.
- **Lazy expansion**: `ExpandIter` / `OwnedExpandIter` avoid materializing the full rewritten string, keeping memory bounded for high-iteration fractals.
- **Dual target from day one**: `lsystem-core` has no platform-specific deps so it compiles for both native and `wasm32-unknown-unknown` without feature flags.
- **3D forward-compat seam**: the `dimensions` TOML field (currently validated to `2` only) is the extension point. To add 3D: add `turtle/turtle3d.rs` with a `Segments3D<I>` iterator analogous to `Segments2D`, then dispatch in `lib.rs::generate()` based on `cfg.dimensions`. No other registration is needed.
- **Whitespace in axiom/rules is stripped**: whitespace inside `axiom` and rule RHS strings is removed before validation and expansion, allowing multi-line formatting in TOML configs.
- **Fractal lives in egui's layout**: the fractal canvas is allocated via `ui.allocate_painter()` inside an `egui::CentralPanel { frame: Frame::NONE }`, and drawn through an `egui_wgpu::CallbackTrait`. Pan/zoom come from the painter `Response` (no raw winit mouse handling); egui automatically sets the wgpu viewport to the allocated rect before invoking `paint()`, so the callback only sets pipeline/bind group/vertex buffer.
- **One render pass per frame**: the egui-wgpu render pass uses `LoadOp::Clear` with the config's `background_color` (defaulting to black) and contains every draw — both egui shapes and the fractal callback. `GpuContext::begin_frame` only acquires the surface texture; there is no separate clear pass.
- **`RedrawRequested` is handled directly, never fed to `egui-winit`**: `egui-winit::on_window_event` returns `repaint = true` for *every* `WindowEvent` variant, including `RedrawRequested` itself — feeding it back would queue another `RedrawRequested` every frame and burn CPU. `App::window_event` short-circuits on `RedrawRequested`. This mirrors eframe's pattern.
- **Caller-driven geometry uploads**: `App` sets `needs_upload: bool` whenever it regenerates vertices; the flag is passed through `FractalCallback` to the egui adapter, which calls `LinePipeline::upload` (vertex buffer + `ColorParams`) only when `true`, and `write_transform` (camera uniform) every frame. `needs_upload` is cleared in `App::handle_redraw` after `egui.render` returns.
- **SVG export is a `lsystem-core` Cargo feature**: The `svg` feature adds `svg_export::export_svg(config) -> String`. It collects segments into a `Vec` (the only allocation — acceptable for export), computes a padded bounding box, and builds SVG XML. The Y-axis flip (turtle is Y-up, SVG is Y-down) is handled by a `<g transform="matrix(1 0 0 -1 0 0)">` group so turtle coordinates are written as-is; the viewBox compensates. `stroke-width`, `stroke-linecap`, and `fill` are set on the `<g>` and inherited by children. Solid mode emits a single `<path>`; gradient and hue-cycle modes emit per-segment `<line>` elements to match the shader's segment-index-based coloring exactly. On native, `rfd::FileDialog` handles the save dialog; on WASM, a programmatic Blob download is triggered via `web-sys`.
- **Strict CI**: `clippy -D warnings` and `cargo fmt --check` must pass; the `wasm-check` job catches WASM regressions early.
