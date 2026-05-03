# Grow Your Own Fractal

An interactive [L-System](https://en.wikipedia.org/wiki/L-system) (Lindenmayer
system) visualizer built with Rust, WebGPU, and WebAssembly. Runs natively on
the desktop and in the browser from the same codebase.

---

## For Users

### What are L-Systems?

L-Systems are formal string-rewriting grammars originally developed to model
plant growth. You define a starting string (the *axiom*) and a set of
*production rules*. The axiom is expanded iteratively — each character that has
a rule is replaced by the rule's right-hand side, and characters without a rule
are kept unchanged. After the requested number of iterations the resulting
string is read by a *turtle* that moves around a canvas, drawing line segments.

**Example** — Koch Snowflake, one iteration:

```
axiom:   F++F++F
rule:    F → F-F++F-F

iter 0:  F++F++F
iter 1:  F-F++F-F  ++  F-F++F-F  ++  F-F++F-F
```

Each `F` is replaced; `+` has no rule, so it passes through unchanged.

### Alphabet

Every character in the axiom and in rule right-hand sides must be one of the
following:

| Symbol | Name | Effect |
|--------|------|--------|
| `F` | Forward (draw) | Move one step forward and draw a line segment. |
| `f` | Forward (no draw) | Move one step forward without drawing. |
| `+` | Turn left | Rotate counter-clockwise by the configured `angle`. |
| `-` | Turn right | Rotate clockwise by the configured `angle`. |
| `\|` | U-turn | Rotate 180° in place. |
| `[` | Push state | Save the current position and heading on a stack. |
| `]` | Pop state | Restore the most recently saved position and heading. |
| `A`–`Z`, `a`–`z` | Non-terminal | Rewritten by rules during expansion. Any letter that has no rule and is not a reserved symbol above is silently skipped by the turtle. |

Any other character is a validation error.

### Config format

Each L-System is defined in a TOML file:

```toml
name = "Koch Snowflake"
dimensions = 2          # must be 2
axiom = "F++F++F"
iterations = 4          # number of times the rules are applied
angle = 60.0            # degrees; used by + - and |
step = 1.0              # length of each F / f move
initial_heading = 0.0   # starting direction in degrees (0 = east,
                        # counter-clockwise positive)

[rules]
F = "F-F++F-F"          # each F is replaced by this string each iteration
```

Whitespace inside `axiom` and rule strings is stripped before processing, so
you can break long rules across lines for readability.

### Controls

| Input | Action |
|-------|--------|
| Drag (left button) | Pan |
| Scroll wheel | Zoom in / out toward the cursor |
| `F` | Reset view to fit the fractal |

### Bundled presets

| File | Name | Description |
|------|------|-------------|
| `presets/koch_snowflake.toml` | Koch Snowflake | Classic fractal snowflake; angle 60°, 4 iterations. |
| `presets/dragon_curve.toml` | Dragon Curve | Self-similar curve folded from a strip of paper; angle 90°, 12 iterations. |
| `presets/sierpinski_triangle.toml` | Sierpinski Triangle | Self-similar triangle; angle 120°, 6 iterations. |
| `presets/plant_a.toml` | Plant A | Branching plant with push/pop brackets; angle 25°, 6 iterations. |
| `presets/hilbert_curve.toml` | Hilbert Curve | Space-filling curve; angle 90°, 5 iterations. |

---

## For Developers

### Prerequisites

| Tool | Purpose |
|------|---------|
| [Rust](https://rustup.rs/) stable | compiler (version pinned in `rust-toolchain.toml`) |
| [mise](https://mise.jdx.dev/) | installs pinned tools — trunk (version pinned in `mise.toml`) |
| Chrome ≥ 113 / Edge | WebGPU support in the browser |

```sh
mise install   # installs trunk at the version pinned in mise.toml
```

### Building

**Native:**

```sh
cargo run -p lsystem-app
```

**Web — development server:**

```sh
trunk serve          # opens http://localhost:8080 automatically
```

**Web — release build:**

```sh
trunk build --release    # output in dist/
```

### Running tests

```sh
cargo test --workspace
```

### Project structure

```
Cargo.toml                  workspace manifest
rust-toolchain.toml         pins stable Rust + wasm32 target + components
mise.toml                   pins trunk version (read by CI and local dev)
.github/workflows/ci.yml    fmt · clippy · test · wasm-check · trunk-build

crates/
  lsystem-core/             pure Rust, no rendering deps
    src/
      config.rs             TOML parsing + Config struct
      alphabet.rs           reserved-symbol sets, validation
      grammar.rs            axiom + rule expansion (N iterations)
      geometry.rs           Geometry type: line segments as Vec<[Vec2; 2]>
      turtle/
        turtle2d.rs         2D turtle interpreter
  lsystem-app/              native + web entry points
    src/
      main.rs               native entry point
      fractal_renderer.rs   wgpu surface + the egui paint callback that draws the fractal
      renderer.rs           winit ApplicationHandler that orchestrates each frame
      camera.rs             pan/zoom state and view transform
      shader.wgsl           vertex + fragment shaders
      lib.rs                crate entry points for native and web
      ui.rs                 egui layout (side panel + central fractal canvas) and egui-wgpu wiring

index.html                  trunk entry: canvas + WebGPU detection
Trunk.toml                  trunk build config

presets/                    bundled TOML L-System definitions
```

### CI

Every push and pull request to `main` runs five jobs:

| Job | Command |
|-----|---------|
| fmt | `cargo fmt --check --all` |
| clippy | `cargo clippy --workspace -- -D warnings` |
| test | `cargo test --workspace` |
| wasm-check | `cargo check --target wasm32-unknown-unknown -p lsystem-app` |
| trunk-build | `trunk build --release` |

### License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option.
