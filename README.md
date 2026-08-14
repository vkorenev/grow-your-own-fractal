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
- Solid, gradient (including topological-depth coloring), and animated
  hue-cycle line colors.
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

## Using the app

The project specifications contain the complete supported behavior:

- [L-system semantics](docs/specs/l-system.md) describes the alphabet,
  rewriting, turtle actions, and 2D/3D geometry.
- [Configuration](docs/specs/configuration.md) describes the strict TOML
  format, validation, defaults, color modes, and complete examples.
- [Application workspace](docs/specs/application-workspace.md) describes
  presets, custom configurations, editing, and platform variants.
- [Rendering and interaction](docs/specs/rendering-and-interaction.md)
  describes colors, camera controls, animation, and failure recovery.
- [Exports](docs/specs/exports.md) describes SVG, PNG, and APNG behavior.

## Bundled presets

Bundled L-System definitions live in [`presets/`](presets/). They are embedded
in both apps at compile time.

## Development

For local setup, build commands, tests, CI, and architecture notes, see
[`CONTRIBUTING.md`](CONTRIBUTING.md).

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option.
