# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**Grow Your Own Fractal** — an interactive L-System (Lindenmayer system) visualizer in Rust. Supports both native desktop and browser (WebAssembly/WebGPU) from a shared codebase. The fractal wgpu pipeline (`fractal_renderer.rs`) is fully decoupled from the egui GUI (`ui.rs`); `renderer.rs` is a thin winit orchestration layer that wires them together.

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
| `fractal_renderer.rs` | `FractalRenderer` — wgpu surface/pipeline/buffers, `begin_frame`/`end_frame`; no egui dependency. On wasm `FractalRenderer` is built asynchronously and delivered via `UserEvent::GpuReady` |
| `renderer.rs` | `App` (`ApplicationHandler<UserEvent>`) — winit event handling, coordinates `FractalRenderer` and `EguiRenderer` each frame |
| `ui.rs` | `UiState` (preset/config state, egui draw logic) + `EguiRenderer` (egui context, winit integration, wgpu render pass) |
| `camera.rs` | `Camera` (pan/zoom state), `Transform` uniform, `compute_transform` |
| `input.rs` | `InputState` — mouse drag and cursor tracking |
| `shader.wgsl` | Vertex shader applies a `Transform` uniform (scale + offset); fragment shader outputs a fixed colour; topology is `LineList` |

### `presets/`

Five bundled TOML L-System definitions (`koch_snowflake.toml`, `dragon_curve.toml`, `sierpinski_triangle.toml`, `plant_a.toml`, `hilbert_curve.toml`). New fractals are added here; they are embedded at compile time via `include_dir!` in `ui.rs` and auto-discovered — no registration step needed.

## Key Design Decisions

- **Lazy expansion**: `ExpandIter` avoids materializing the full string at each iteration, keeping memory bounded for high-iteration fractals.
- **Dual target from day one**: `lsystem-core` has no platform-specific deps so it compiles for both native and `wasm32-unknown-unknown` without feature flags.
- **Strict CI**: `clippy -D warnings` and `cargo fmt --check` must pass; the `wasm-check` job catches WASM regressions early.
