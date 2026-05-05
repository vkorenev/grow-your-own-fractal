# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**Grow Your Own Fractal** — an interactive L-System (Lindenmayer system) visualizer in Rust. Supports both native desktop and browser (WebAssembly/WebGPU) from a shared codebase. The fractal is rendered via an `egui_wgpu::CallbackTrait` adapter (in `ui.rs`) that bridges egui's paint-callback system to a toolkit-independent wgpu pipeline in `fractal_renderer.rs`; layout, hit-testing, and z-order all flow through egui, and `renderer.rs` is a thin winit orchestration layer that owns the surface and dispatches frames.

## Common Commands

```bash
# Build & run
cargo run -p lsystem-app          # native desktop
trunk serve                        # web dev server at localhost:8080
trunk build --release              # web release build → dist/

# Verification (all run in CI)
cargo test --workspace
cargo fmt --check --all
cargo clippy --workspace -- -D warnings
cargo check --target wasm32-unknown-unknown -p lsystem-app

# Run a single test
cargo test -p lsystem-core config::tests::test_name
```

`trunk` is managed by mise; run `mise install` to get the pinned version from `mise.toml`.

## Architecture

Two-crate workspace under `crates/`:

### `lsystem-core` — pure library, zero rendering deps

| Module | Role |
|--------|------|
| `config.rs` | Parses TOML `Config` struct; validates axiom, rules, step/angle finiteness, bracket balance |
| `alphabet.rs` | Reserved symbols (`F f + - \| [ ]`), character set validation |
| `grammar.rs` | `expand(axiom, rules, iterations)` → lazy `ExpandIter` char iterator; stack-based rewriting avoids materializing the full string |
| `geometry.rs` | `Geometry::D2` wrapping `Vec<[Vec2; 2]>` line segments |
| `turtle/mod.rs` | `Turtle` trait + `build()` factory; currently always returns `Turtle2D` (reserved for future 3D dispatch) |
| `turtle/turtle2d.rs` | `Turtle2D` — consumes char iterator, tracks position/heading via a stack, emits line segments |
| `lib.rs` | Public API: `generate(config) -> Geometry` |

Data flow: `Config` → `ExpandIter` (lazy string rewriting) → `Turtle2D` → `Geometry`.

### `lsystem-app` — entry points and rendering

| File | Role |
|------|------|
| `main.rs` | Thin native entry that calls `lib.rs::run_native()` |
| `lib.rs` | Module declarations; `run_native()` builds an `EventLoop<UserEvent>` and calls `run_app`; `#[wasm_bindgen(start)] start()` does the same on web via `EventLoopExtWebSys::spawn_app` |
| `fractal_renderer.rs` | Toolkit-independent wgpu module. `FractalRenderer` — owns the wgpu surface; `begin_frame` acquires the next surface texture and `end_frame` submits + presents. `FractalPipelineResources` (pipeline, bind group, vertex/uniform buffers) — `update()` re-uploads vertices when `geometry_version` changes and writes the camera transform; `draw()` issues the line-list draw. `FractalCallback` — plain per-frame data struct (vertices, transform, geometry_version). On wasm `FractalRenderer` is built asynchronously and delivered via `UserEvent::GpuReady` |
| `renderer.rs` | `App` (`ApplicationHandler<UserEvent>`) — owns `Camera`, geometry buffer, side-panel state. Routes `WindowEvent::RedrawRequested` straight to its own renderer; routes everything else through `egui-winit` |
| `ui.rs` | `UiState` (preset/config state, egui layout including the central fractal canvas via `ui.allocate_painter()`, pan/zoom from the painter `Response`) + `EguiRenderer` (egui context, egui-wgpu integration, single render pass that does both the surface clear and the fractal+egui draw) + `impl egui_wgpu::CallbackTrait for FractalCallback` (thin egui adapter that delegates to `FractalPipelineResources::update/draw`) |
| `camera.rs` | `Camera` (pan/zoom state), `Transform` uniform, `compute_transform` |
| `shader.wgsl` | Vertex shader applies a `Transform` uniform (scale + offset); fragment shader outputs a fixed colour; topology is `LineList` |

### `presets/`

Five bundled TOML L-System definitions (`koch_snowflake.toml`, `dragon_curve.toml`, `sierpinski_triangle.toml`, `plant_a.toml`, `hilbert_curve.toml`). New fractals are added here; they are embedded at compile time via `include_dir!` in `ui.rs` and auto-discovered — no registration step needed.

## Key Design Decisions

- **Lazy expansion**: `ExpandIter` avoids materializing the full string at each iteration, keeping memory bounded for high-iteration fractals.
- **Dual target from day one**: `lsystem-core` has no platform-specific deps so it compiles for both native and `wasm32-unknown-unknown` without feature flags.
- **3D forward-compat seams**: `Geometry::D3`, the `dimensions` TOML field (currently validated to `2` only), and the `Turtle` trait dispatch in `build()` are all present so that adding 3D is a purely additive extension — do not remove them as dead code.
- **Whitespace in axiom/rules is stripped**: whitespace inside `axiom` and rule RHS strings is removed before validation and expansion, allowing multi-line formatting in TOML configs.
- **Fractal lives in egui's layout**: the fractal canvas is allocated via `ui.allocate_painter()` inside an `egui::CentralPanel { frame: Frame::NONE }`, and drawn through an `egui_wgpu::CallbackTrait`. Pan/zoom come from the painter `Response` (no raw winit mouse handling); egui automatically sets the wgpu viewport to the allocated rect before invoking `paint()`, so the callback only sets pipeline/bind group/vertex buffer.
- **One render pass per frame**: the egui-wgpu render pass uses `LoadOp::Clear(BLACK)` and contains every draw — both egui shapes and the fractal callback. `FractalRenderer::begin_frame` only acquires the surface texture; there is no separate clear pass.
- **`RedrawRequested` is handled directly, never fed to `egui-winit`**: `egui-winit::on_window_event` returns `repaint = true` for *every* `WindowEvent` variant, including `RedrawRequested` itself — feeding it back would queue another `RedrawRequested` every frame and burn CPU. `App::window_event` short-circuits on `RedrawRequested`. This mirrors eframe's pattern.
- **Geometry uploads are versioned**: `App` increments `geometry_version: u64` whenever it regenerates vertices; `FractalPipelineResources::update` compares against its stored version and re-creates the vertex buffer only when they differ. The transform uniform is rewritten every frame.
- **Strict CI**: `clippy -D warnings` and `cargo fmt --check` must pass; the `wasm-check` job catches WASM regressions early.
