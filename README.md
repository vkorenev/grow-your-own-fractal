# Grow Your Own Fractal

An interactive [L-System](https://en.wikipedia.org/wiki/L-system) (Lindenmayer
system) visualizer built with Rust, wgpu, and WebAssembly. The workspace
includes an Iced native/wasm app and a browser-first Leptos app with DOM
controls and a GPU canvas that uses WebGPU with a WebGL2 fallback on browser
wasm targets.

## What are L-Systems?

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

**3D-only symbols** (only valid when `dimensions = 3`):

| Symbol | Name | Effect |
|--------|------|--------|
| `&` | Pitch down | Rotate the heading downward by `angle` (around the left axis). |
| `^` | Pitch up | Rotate the heading upward by `angle`. |
| `/` | Roll right | Roll clockwise by `angle` (around the heading axis). |
| `\` | Roll left | Roll counter-clockwise by `angle`. |

Any other character is a validation error.

## Config format

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
background = [0.0, 0.0, 0.0]   # optional RGB 0-1; omit to use black

[colors.line]
# mode = "solid"           # single color; set with `color`
# mode = "gradient"        # linear RGB from `start` to `end` across all segments
# mode = "hue_cycle"       # full hue rotation starting from `initial`
# mode = "depth_gradient"  # linear RGB by topological bracket depth; equivalent
#                           # to `gradient` for non-branching (bracketless) fractals
mode = "solid"
color = [0.0, 0.9, 0.5]  # used by solid mode

# gradient example:
# mode  = "gradient"
# start = [1.0, 0.4, 0.0]
# end   = [0.6, 0.0, 1.0]

# hue_cycle example:
# mode    = "hue_cycle"
# initial = [0.9, 0.0, 0.0]

# depth_gradient example (branching fractals only; same as gradient otherwise):
# mode  = "depth_gradient"
# start = [1.0, 0.4, 0.0]
# end   = [0.6, 0.0, 1.0]
```

Configuration uses the nested v2 field paths: `metadata.name`, `l-system.*`,
`l-system.rules`, `turtle.*`, `colors.*`, and `colors.line.*`. Those paths may
be written with explicit tables, dotted keys, or implicit parent tables. Older
flat TOML with top-level `name`, `axiom`, `[rules]`, `background_color`, or
`[line_color]` is rejected. `colors.background` is optional and falls back to
black when omitted. All present colors are RGB arrays with finite components in
the 0-1 range, including `hue_cycle`'s RGB `initial` color.

Whitespace inside `axiom` and rule strings is stripped before processing, so
you can break long rules across lines for readability.

HSV movement is a playback control in the UI for `hue_cycle` line colors. It
temporarily offsets the rendered hue start while it is enabled; it is not a TOML
field and does not change the stored `initial` color. If another line color mode
is active, the saved movement state is ignored until `hue_cycle` is selected
again.

## Controls

**2D**

| Input | Action |
|-------|--------|
| Drag (left button) | Pan |
| Scroll wheel | Zoom in / out toward the cursor |
| `F` | Reset view to fit the fractal |

When the line color mode is **Hue cycle**, the control panel also shows an HSV
movement toggle, direction selector, and speed slider. This shifts the visible
hue cycle over time without changing the config text.

**3D** (when `dimensions = 3`)

| Input | Action |
|-------|--------|
| Drag (left button) | Orbit (rotate azimuth / elevation) |
| Scroll wheel | Zoom in / out |
| Arrow keys | Rotate azimuth (left / right) or elevation (up / down) by 5° |
| `Q` / `E` | Roll counter-clockwise / clockwise by 5° |
| `F` | Reset camera to fit the fractal |
| Auto-rotate toggle | Continuously orbit around the Y axis at the configured speed |

## Exporting

The **Export SVG** button saves the current fractal as a resolution-independent
SVG file. SVG export is only available for 2D fractals.
The **Export PNG** button renders the fractal to a raster PNG using the selected
PNG width. For 3D fractals, PNG captures the current camera orientation.
Exports use the static colors from the active config, not any transient HSV
movement phase currently visible in the UI.

## Bundled presets

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

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option.
