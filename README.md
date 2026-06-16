# Grow Your Own Fractal

An interactive [L-System](https://en.wikipedia.org/wiki/L-system) (Lindenmayer
system) visualizer built with Rust, wgpu, and WebAssembly. The browser app uses
Leptos DOM controls and a GPU canvas with WebGPU rendering and a WebGL2
fallback. A native desktop app built with Iced is also available.

## Try it

The browser app is available on
[GitHub Pages](https://vkorenev.github.io/grow-your-own-fractal/).

## Features

- Fast GPU-accelerated rendering.
- Built-in 2D and 3D presets with editable TOML configs.
- Open and save custom configs.
- Pan and zoom 2D fractals; orbit, roll, and auto-rotate 3D fractals.
- Solid, gradient (including topological-depth mode), and hue-cycle line colors (with animatable hue rotation).
- Save still images as SVG (2D) or PNG, and animations as APNG.

## What are L-Systems?

L-Systems are formal string-rewriting grammars originally developed to model
plant growth. You define a starting string (the *axiom*) and a set of
*production rules*. The axiom is expanded iteratively — each character that has
a rule is replaced by the rule's right-hand side, and characters without a rule
are kept unchanged. After the requested number of iterations, the resulting
string is read by a *turtle* that moves around a canvas, drawing line segments.

**Example** — Koch Snowflake, one iteration:

```
axiom:   F++F++F
rule:    F → F-F++F-F

iter 0:  F++F++F
iter 1:  F-F++F-F  ++  F-F++F-F  ++  F-F++F-F
```

Each `F` is replaced; `+` has no rule, so it passes through unchanged.

## Alphabet

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

**3D-only symbols** (only valid when `dimensions = "3D"`):

| Symbol | Name | Effect |
|--------|------|--------|
| `&` | Pitch down | Rotate the heading downward by `angle` (around the left axis). |
| `^` | Pitch up | Rotate the heading upward by `angle` (around the left axis). |
| `/` | Roll right | Roll clockwise by `angle` (around the heading axis). |
| `\` | Roll left | Roll counter-clockwise by `angle`. |

Any other character is a validation error.

## Config format

Each L-System is defined in a TOML file:

```toml
[metadata]
name = "Koch Snowflake"

[l-system]
dimensions = "2D"       # "2D" or "3D"
axiom = "F++F++F"
iterations = 4          # number of times the rules are applied
angle = 60.0            # degrees; used by + - and |
step = 1.0              # optional; length of each F / f move
initial_heading = 0.0   # optional; starting direction in degrees
                        # (0 = east, counter-clockwise positive)

[l-system.rules]
F = "F-F++F-F"          # each F is replaced by this string each iteration

[colors]
background = "#000000"   # optional hex color

# Choose exactly one line color mode: solid, gradient, or hue_cycle.
# Omit [colors.line] entirely to use the built-in solid line color.
[colors.line]
solid = "#00e680"        # solid line color

# gradient example:
# [colors.line.gradient]
# start = "#ff6600"
# end = "#9900ff"
# topological_depth = false  # optional

# hue_cycle example:
# [colors.line.hue_cycle]
# initial = "#e60000"

# Topological-depth gradient example (branching fractals only, i.e. those using
# `[` / `]` brackets; same as traversal gradient otherwise):
# [colors.line.gradient]
# start = "#ff6600"
# end = "#9900ff"
# topological_depth = true
```

Whitespace inside `axiom` and rule strings is stripped before processing, so
you can break long rules across lines for readability.

## Controls

**2D**

| Input | Action |
|-------|--------|
| Drag (left button) | Pan |
| Scroll wheel | Zoom in / out toward the cursor |
| `F` | Reset view to fit the fractal |

**3D** (when `dimensions = "3D"`)

| Input | Action |
|-------|--------|
| Drag (left button) | Orbit (rotate azimuth / elevation) |
| Scroll wheel | Zoom in / out |
| Arrow keys | Rotate azimuth (left / right) or elevation (up / down) by 5° |
| `Q` / `E` | Roll counter-clockwise / clockwise by 5° |
| `F` | Reset camera to fit the fractal |
| Auto-rotate toggle | Continuously orbit around the Y axis at the configured speed |

## Bundled presets

| File | Name | Description |
|------|------|-------------|
| `presets/box_fractal.toml` | Box Fractal | Square-grid fractal built by replacing each edge with a five-segment box pattern. |
| `presets/dragon_curve.toml` | Harter-Heighway Dragon | Self-similar curve obtained by repeatedly folding a strip of paper in half. |
| `presets/gosper_curve.toml` | Gosper Curve | Space-filling curve that tiles the plane with hexagonal regions; also known as the flowsnake. |
| `presets/hilbert_curve.toml` | Hilbert Curve | Space-filling curve that maps a line continuously to a 2D square while preserving locality. |
| `presets/hilbert_curve_3d.toml` | 3D Hilbert Curve | Three-dimensional Hilbert-style space-filling curve using pitch and roll turns. |
| `presets/koch_snowflake.toml` | Koch Snowflake | Classic fractal snowflake built by iteratively replacing each edge with a triangular bump. |
| `presets/levy_c_curve.toml` | Lévy C Curve | Self-similar curve formed by recursively replacing a segment with two right-angled turns. |
| `presets/peano_curve.toml` | Peano Curve | First known space-filling curve; fills a square with a continuous self-similar path. |
| `presets/plant_a.toml` | Plant A | Branching plant-like structure modelled with push/pop brackets for recursive branching. |
| `presets/sierpinski_arrowhead_curve.toml` | Sierpiński Arrowhead Curve | Continuous self-similar curve that traces a Sierpiński triangle with alternating arrowhead turns. |
| `presets/sierpinski_curve.toml` | Sierpiński Curve | Space-filling curve traced along the boundary of a Sierpiński triangle. |
| `presets/sierpinski_triangle.toml` | Sierpiński Triangle | Self-similar triangle subdivided into progressively smaller triangular holes. |
| `presets/snowflake.toml` | Snowflake | Branching snowflake pattern with six-fold symmetry and recursive side branches. |
| `presets/plant_3d.toml` | 3D Plant | Branching 3D plant using pitch symbols to spread branches in all directions. |
| `presets/ternary_tree_3d.toml` | 3D Ternary Tree | Branching tree that splits into three pitched branches at each recursive step. |
| `presets/tree_3d.toml` | 3D Tree | Symmetric 3D tree that combines yaw, pitch, and roll for multi-directional branching. |

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option.
