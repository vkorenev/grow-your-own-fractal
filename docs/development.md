# Development

This document covers local setup, build commands, verification, CI, and deploy
behavior. The system design lives in [`architecture.md`](architecture.md).

## Prerequisites

| Tool | Purpose |
|------|---------|
| [Rust](https://rustup.rs/) stable | Compiler and Cargo. The channel, components, and wasm target are pinned in `rust-toolchain.toml`. |
| [mise](https://mise.jdx.dev/) | Installs pinned development tools, including `trunk` from `mise.toml`. |
| Modern browser | Runs the browser apps with WebGPU or the WebGL2 fallback renderer. |

Install pinned tools:

```sh
mise install
```

`trunk` may fail to launch when `NO_COLOR=1` is present in the environment. Use
`NO_COLOR=true` or `NO_COLOR=false` as a workaround.

## Build And Run

Run the native Iced app:

```sh
cargo run -p lsystem-app
```

Run the browser-first Leptos app locally:

```sh
trunk serve --config crates/lsystem-web-app/Trunk.toml
```

The Leptos app serves on <http://127.0.0.1:8081/>.

Build the browser-first Leptos app for release:

```sh
trunk build --release --config crates/lsystem-web-app/Trunk.toml
```

The release output is written to `crates/lsystem-web-app/dist/`.

Run or build the retained Iced web app:

```sh
trunk serve --config crates/lsystem-app/Trunk.toml
trunk build --release --config crates/lsystem-app/Trunk.toml
```

The Iced web app serves on <http://127.0.0.1:8080/>.

## Testing

Run the workspace test suite:

```sh
cargo test --workspace --all-features --all-targets
cargo test --workspace --all-features --doc
```

Run a single core test:

```sh
cargo test -p lsystem-core config::tests::test_name
```

Run SVG export tests:

```sh
cargo test -p lsystem-core --features svg svg_export
```

## Benchmarking

Run fractal generation microbenchmarks:

```sh
cargo bench -p lsystem-core --bench generation
```

Run offscreen renderer microbenchmarks:

```sh
cargo bench -p lsystem-renderer --features png --bench offscreen_render
```

The renderer benchmark measures config compilation, generation, scene/pipeline
construction, staging upload, offscreen RGBA rendering, and GPU readback. It
does not include PNG encoding time.

## Profiling

For Linux `perf`/`cargo flamegraph` profiling, pass `-Wl,--no-rosegment` through
`RUSTFLAGS`. This matches flamegraph's Linux guidance for lld and mold,
including Rust 1.90.0 and later where lld is the default, and keeps generated
stack traces accurate.

To improve release-profile flamegraphs, include debug info for the profiled run:

```sh
RUSTFLAGS="-C link-arg=-Wl,--no-rosegment" CARGO_PROFILE_RELEASE_DEBUG=true cargo flamegraph -p lsystem-app
```

Benchmarks use Cargo's bench profile in release mode. Include bench-profile
debug info when profiling benchmarks:

```sh
RUSTFLAGS="-C link-arg=-Wl,--no-rosegment" CARGO_PROFILE_BENCH_DEBUG=true cargo flamegraph --bench generation -p lsystem-core -- --bench
```

## Full Verification

For Rust code, build configuration, or rustdoc changes, the normal local
pre-submit verification is the relevant subset of these Cargo checks:

```sh
cargo fmt --check --all
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy --workspace --all-features --all-targets -- -D warnings
cargo test --workspace --all-features --all-targets
cargo test --workspace --all-features --doc
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace --all-features
cargo clippy --target wasm32-unknown-unknown --workspace -- -D warnings
cargo clippy --target wasm32-unknown-unknown --workspace --all-features -- -D warnings
```

CI also runs release Trunk builds for both web apps. Running them locally is
usually unnecessary for a Rust-only refactor when the native and wasm Cargo
checks above succeed. A local Trunk build is worth the additional time when a
change affects:

- a `Trunk.toml`, HTML entry point, static asset, public path, or deployment
  URL;
- wasm startup, JavaScript bindings, browser-only feature gates, or code that
  is exercised only when the final wasm artifact is linked;
- dependencies, build scripts, generated shader bindings, or wgpu integration
  in a way that could fail during release linking or bundling;
- release artifact contents or size; or
- a Trunk-build failure seen in CI.

For an app-specific change, run only its corresponding command:

```sh
trunk build --release --config crates/lsystem-app/Trunk.toml
trunk build --release --config crates/lsystem-web-app/Trunk.toml
```

Run both commands for cross-cutting web, renderer, dependency, or deployment
changes. Otherwise, rely on CI for the final Trunk-build coverage.

For documentation-only changes that do not affect rustdoc or build scripts,
spellcheck/link review may be enough. For code changes, prefer the smallest
relevant command while iterating, then run the full affected verification
before submitting.

## Continuous Integration

Every push and pull request to `main` runs:

| Job | Commands |
|-----|----------|
| Format | `cargo fmt --check --all` |
| Clippy | Native default/all-features clippy with `--all-targets`, plus wasm default/all-features clippy. All use `-D warnings`. |
| Test | `cargo test --workspace --all-features --all-targets` with Mesa Vulkan drivers installed for GPU-related tests. Also runs `cargo test --workspace --all-features --doc` to exercise doctests, including `compile_fail` assertions. |
| Docs | `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace --all-features` |
| Trunk Build | Release builds for `crates/lsystem-app/Trunk.toml` and `crates/lsystem-web-app/Trunk.toml`. |

## Deploy

Successful CI runs on `main` trigger `.github/workflows/deploy.yml`. The deploy
workflow builds both web apps and publishes them to GitHub Pages:

- `lsystem-web-app` is deployed at the site root.
- `lsystem-app` is deployed under `/iced/`.

## Project Structure

```text
Cargo.toml                  workspace manifest
rust-toolchain.toml         pins stable Rust, wasm target, and components
mise.toml                   pins trunk version for local development and CI
.github/workflows/ci.yml    fmt, clippy, test, docs, and trunk builds
.github/workflows/deploy.yml deploys both web apps to GitHub Pages

crates/
  lsystem-core/             grammar expansion, turtle geometry, runtime config, SVG export
  lsystem-app-model/        shared TOML config model, defaults, presets, colors, animation, utilities
  lsystem-renderer/         shared wgpu camera, line rendering, and PNG/APNG export
  lsystem-app/              Iced native app and retained Iced web app
  lsystem-web-app/          browser-first Leptos app with DOM controls and GPU canvas

presets/                    bundled TOML L-System definitions
docs/specs/                 authoritative behavior and configuration contracts
docs/architecture.md        crate boundaries, data flow, and implementation design
docs/development.md         setup, verification, CI, deploy, and repository layout
```

## Documentation Updates

When changing behavior, commands, architecture, config format, deploy workflow,
or contributor workflow, update the relevant docs in the same change:

- `README.md` for the user-facing product overview and documentation entry
  points.
- `docs/specs/` for supported behavior, L-system semantics, config format,
  application UX, rendering/interaction, or export contracts.
- `CONTRIBUTING.md` for contributor entry-point changes.
- `AGENTS.md` for agent-only workflow rules.
- `docs/architecture.md` for design, boundaries, or invariants.
- `docs/development.md` for commands, setup, CI, deploy, or repository layout.

Keep README explanations concise and link to the canonical specification rather
than maintaining a second detailed copy of the same contract. Future designs
and implementation proposals are non-normative working documents until a
change explicitly promotes their behavior into `docs/specs/`.
