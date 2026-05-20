# Grow Your Own Fractal

An interactive [L-System](https://en.wikipedia.org/wiki/L-system) (Lindenmayer
system) visualizer built with Rust, wgpu, and WebAssembly. The workspace
includes an Iced native/wasm app and a browser-first Leptos app with DOM
controls and a GPU canvas that uses WebGPU with a WebGL2 fallback on browser
wasm targets.

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

**2D and 3D symbols** (valid for any `dimensions` value):

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

**3D-only symbols** (only valid when `dimensions = 3`):

| Symbol | Name | Effect |
|--------|------|--------|
| `&` | Pitch down | Rotate the heading downward by `angle` (around the left axis). |
| `^` | Pitch up | Rotate the heading upward by `angle`. |
| `/` | Roll right | Roll clockwise by `angle` (around the heading axis). |
| `\` | Roll left | Roll counter-clockwise by `angle`. |

Any other character is a validation error.

### Config format

Each L-System is defined in a TOML file:

```toml
[metadata]
name = "Koch Snowflake"

[l-system]
dimensions = 2          # 2 or 3
axiom = "F++F++F"
iterations = 4          # number of times the rules are applied

[l-system.rules]
F = "F-F++F-F"          # each F is replaced by this string each iteration

[turtle]
angle = 60.0            # degrees; used by + - and |
step = 1.0              # length of each F / f move
initial_heading = 0.0   # starting direction in degrees (0 = east,
                        # counter-clockwise positive)

[colors]
background = [0.0, 0.0, 0.0]   # RGB 0-1

[colors.line]
# mode = "solid"          # single color; set with `color`
# mode = "gradient"       # linear RGB from `start` to `end` across all segments
# mode = "hue_cycle"      # full hue rotation starting from `initial`
mode = "solid"
color = [0.0, 0.9, 0.5]  # used by solid mode

# gradient example:
# mode  = "gradient"
# start = [1.0, 0.4, 0.0]
# end   = [0.6, 0.0, 1.0]

# hue_cycle example:
# mode    = "hue_cycle"
# initial = [0.9, 0.0, 0.0]
```

Configuration uses the nested v2 field paths: `metadata.name`, `l-system.*`,
`l-system.rules`, `turtle.*`, `colors.*`, and `colors.line.*`. Those paths may
be written with explicit tables, dotted keys, or implicit parent tables. Older
flat TOML with top-level `name`, `axiom`, `[rules]`, `background_color`, or
`[line_color]` is rejected. All colors are RGB arrays with finite components in
the 0-1 range, including `hue_cycle`'s RGB `initial` color.

Config parsing is format-preserving: parsing and serializing an unchanged TOML
document keeps comments, spacing, and string quoting intact. Newly generated
TOML writes axiom/rule text as literal strings when possible and keeps color
arrays inline.

Each config entry keeps one last-applied TOML document and, only when needed,
an unapplied draft while the app is open. The UI owns which entry is selected,
and switching entries preserves unapplied edits. **Copy** creates a renamed
custom copy of the selected entry, preserving that entry's draft text separately
from the last-applied document, **Apply** validates and renders the current
draft, **Revert** drops the unapplied draft, and **Reset** restores the bundled
preset default after an applied preset has diverged from that default. Custom
entries exist only for the current session and do not have a bundled default to
reset to. While a draft differs from the last-applied TOML document,
iteration/angle/export controls are hidden until the draft is applied or
reverted.

Whitespace inside `axiom` and rule strings is stripped before processing, so
you can break long rules across lines for readability.

### Controls

**2D**

| Input | Action |
|-------|--------|
| Drag (left button) | Pan |
| Scroll wheel | Zoom in / out toward the cursor |
| `F` | Reset view to fit the fractal |

**3D** (when `dimensions = 3`)

| Input | Action |
|-------|--------|
| Drag (left button) | Orbit (rotate azimuth / elevation) |
| Scroll wheel | Zoom in / out |
| Arrow keys | Rotate azimuth (left / right) or elevation (up / down) by 5° |
| `Q` / `E` | Roll counter-clockwise / clockwise by 5° |
| `F` | Reset camera to fit the fractal |
| Auto-rotate toggle | Continuously orbit around the Y axis at the configured speed |

### Exporting

The **Export SVG** button saves the current fractal as a resolution-independent
SVG file. SVG export is only available for 2D fractals.
The **Export PNG** button renders the fractal to a raster PNG using the selected
PNG width. For 3D fractals, PNG captures the current camera orientation.

### Bundled presets

| File | Name | Description |
|------|------|-------------|
| `presets/dragon_curve.toml` | Harter-Heightway Dragon | Self-similar curve obtained by repeatedly folding a strip of paper in half. |
| `presets/gosper_curve.toml` | Gosper Curve | Space-filling curve that tiles the plane with hexagonal regions; also known as the flowsnake. |
| `presets/hilbert_curve.toml` | Hilbert Curve | Space-filling curve that maps a line continuously to a 2D square while preserving locality. |
| `presets/hilbert_curve_3d.toml` | 3D Hilbert Curve | Three-dimensional Hilbert-style space-filling curve using pitch and roll turns. |
| `presets/koch_snowflake.toml` | Koch Snowflake | Classic fractal snowflake built by iteratively replacing each edge with a triangular bump. |
| `presets/peano_curve.toml` | Peano Curve | First known space-filling curve; fills a square with a continuous self-similar path. |
| `presets/plant_a.toml` | Plant A | Branching plant-like structure modelled with push/pop brackets for recursive branching. |
| `presets/sierpinski_curve.toml` | Sierpinski Curve | Space-filling curve traced along the boundary of a Sierpinski triangle. |
| `presets/sierpinski_triangle.toml` | Sierpinski Triangle | Self-similar triangle subdivided into progressively smaller triangular holes. |
| `presets/snowflake.toml` | Snowflake | Branching snowflake pattern with six-fold symmetry and recursive side branches. |
| `presets/plant_3d.toml` | 3D Plant | Branching 3D plant using pitch symbols to spread branches in all directions. |
| `presets/ternary_tree_3d.toml` | 3D Ternary Tree | Branching tree that splits into three pitched branches at each recursive step. |
| `presets/tree_3d.toml` | 3D Tree | Symmetric 3D tree that combines yaw, pitch, and roll for multi-directional branching. |

---

## For Developers

### Prerequisites

| Tool | Purpose |
|------|---------|
| [Rust](https://rustup.rs/) stable | compiler (version pinned in `rust-toolchain.toml`) |
| [mise](https://mise.jdx.dev/) | installs pinned tools — trunk (version pinned in `mise.toml`) |
| Modern browser | WebGPU support, or WebGL2 for the fallback renderer |

```sh
mise install   # installs trunk at the version pinned in mise.toml
```

The Iced dependency is pinned to a specific upstream git revision in the
workspace manifest so the Iced renderer and the shared `wgpu` dependency stay on
the same major version. Update them together.

### Building

**Native:**

```sh
cargo run -p lsystem-app
```

**Web — browser app development server:**

```sh
trunk serve --config crates/lsystem-web-app/Trunk.toml
```

This serves the Leptos/DOM app at <http://127.0.0.1:8081/>.

**Web — browser app release build:**

```sh
trunk build --release --config crates/lsystem-web-app/Trunk.toml
```

The release output is written to `crates/lsystem-web-app/dist/`.

**Web — Iced app:**

```sh
trunk serve --config crates/lsystem-app/Trunk.toml
trunk build --release --config crates/lsystem-app/Trunk.toml
```

### Running tests

```sh
cargo test --workspace --all-features --all-targets
```

### Project structure

```
Cargo.toml                  workspace manifest
rust-toolchain.toml         pins stable Rust + wasm32 target + components
mise.toml                   pins trunk version (read by CI and local dev)
.github/workflows/ci.yml    fmt · clippy · test · wasm-check · trunk-build
.github/workflows/deploy.yml deploys the Leptos web app to GitHub Pages

crates/
  lsystem-core/             pure Rust, no rendering deps
    src/
      config.rs             format-preserving TOML docs + Config/GenerationConfig structs
      alphabet.rs           reserved-symbol sets, validation
      grammar.rs            axiom + rule expansion (N iterations); OwnedExpandIter for lifetime-free streaming
      turtle/
        turtle2d.rs         Segments2D<I>: pull iterator yielding [Vec2; 2] segments lazily
        turtle3d.rs         Segments3D<I>: pull iterator yielding [Vec3; 2] segments using Quat orientation
      svg_export.rs         SVG export — export_svg(config) -> String (enabled by the `svg` Cargo feature)
  lsystem-renderer/         toolkit-independent wgpu renderer (no egui)
    src/
      camera.rs             shared pan/zoom/orbit state and view transform (2D and 3D)
      line_renderer.rs      Vertex2D/Vertex3D, LinePipeline2D/LinePipeline3D, GrowableVertexBuffer, GpuContext
      lsystem_bridge.rs     L-system→GPU adapters: geometry_to_vertices, geometry_to_vertices_3d, color_params_from_config
      png_export.rs         offscreen wgpu PNG renderer (enabled by the `png` Cargo feature)
      wgpu_util.rs          shared wgpu instance/device/error-handler helpers
      shader.wgsl           2D vertex + fragment shaders
      shader3d.wgsl         3D vertex + fragment shaders with MVP matrix uniform
  lsystem-app/              Iced native app, plus retained Iced web entry point
    src/
      main.rs               native entry point
      lib.rs                crate entry points for native and web
      ui.rs                 Iced UI module shell
      ui/app_state.rs       app state, update loop, async geometry generation, preset/config handling
      ui/controls.rs        Iced controls and side panel layout
      ui/fractal_canvas.rs  Iced shader widget integration and viewport input handling
      export.rs             native/browser SVG and PNG export helpers
  lsystem-web-app/          browser-first Leptos app with DOM controls and a GPU canvas
    src/
      lib.rs                wasm entry point
      app.rs                Leptos app, DOM controls, viewport input, GPU error display
      presets.rs            embedded preset loading and effective-config helpers
      export.rs             browser SVG/PNG download helpers
      renderer.rs           canvas-owned wgpu renderer using lsystem-renderer, including surface recovery

crates/lsystem-app/index.html         Iced web Trunk entry
crates/lsystem-app/Trunk.toml         Iced web Trunk config
crates/lsystem-web-app/index.html     Leptos web Trunk entry
crates/lsystem-web-app/Trunk.toml     Leptos web Trunk config

presets/                    bundled TOML L-System definitions
```

### CI

Every push and pull request to `main` runs five jobs:

| Job | Commands |
|-----|----------|
| fmt | `cargo fmt --check --all` |
| clippy | `cargo clippy --workspace -- -D warnings`; `cargo clippy --workspace --all-features -- -D warnings` |
| test | `cargo test --workspace --all-features --all-targets` |
| wasm-check | workspace `cargo check` and `cargo clippy` for `wasm32-unknown-unknown`, with default and all features |
| trunk-build | release builds for `crates/lsystem-app/Trunk.toml` and `crates/lsystem-web-app/Trunk.toml` |

Successful CI runs on `main` trigger the deploy workflow, which builds
`crates/lsystem-web-app` with the repository GitHub Pages public URL and deploys
`crates/lsystem-web-app/dist/`.

### License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option.
