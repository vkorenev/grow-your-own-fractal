# Contributing

Thanks for helping improve Grow Your Own Fractal. This guide covers local
setup, build commands, verification, CI, and the workspace layout.

## Prerequisites

| Tool | Purpose |
|------|---------|
| [Rust](https://rustup.rs/) stable | compiler (version pinned in `rust-toolchain.toml`) |
| [mise](https://mise.jdx.dev/) | installs pinned tools, including `trunk` from `mise.toml` |
| Modern browser | WebGPU support, or WebGL2 for the fallback renderer |

```sh
mise install
```

The Iced dependency is pinned to a specific upstream git revision in the
workspace manifest so the Iced renderer and the shared `wgpu` dependency stay on
the same major version. Update them together.

## Building

Run the native Iced app:

```sh
cargo run -p lsystem-app
```

Run the browser-first Leptos app locally:

```sh
trunk serve --config crates/lsystem-web-app/Trunk.toml
```

This serves the Leptos/DOM app at <http://127.0.0.1:8081/>.

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

## Testing

Run the workspace test suite:

```sh
cargo test --workspace --all-features --all-targets
```

Run a single core test:

```sh
cargo test -p lsystem-core config::tests::test_name
```

Run SVG export tests:

```sh
cargo test -p lsystem-core --features svg svg_export
```

## Before Submitting

Run the same checks CI runs when the change affects code, build config, docs
that rustdoc consumes, or web app behavior:

```sh
cargo fmt --check --all
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy --workspace --all-features --all-targets -- -D warnings
cargo test --workspace --all-features --all-targets
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace --all-features
cargo clippy --target wasm32-unknown-unknown --workspace -- -D warnings
cargo clippy --target wasm32-unknown-unknown --workspace --all-features -- -D warnings
trunk build --release --config crates/lsystem-app/Trunk.toml
trunk build --release --config crates/lsystem-web-app/Trunk.toml
```

## Project Structure

```text
Cargo.toml                  workspace manifest
rust-toolchain.toml         pins stable Rust + wasm32 target + components
mise.toml                   pins trunk version (read by CI and local dev)
.github/workflows/ci.yml    fmt, clippy, test, docs, trunk-build
.github/workflows/deploy.yml deploys the Leptos web app to GitHub Pages

crates/
  lsystem-core/             L-system config parsing, validation, expansion, turtle geometry, SVG export
  lsystem-renderer/         toolkit-independent wgpu camera, line rendering, and PNG export
  lsystem-app/              Iced native app and retained Iced web app
  lsystem-web-app/          browser-first Leptos app with DOM controls and GPU canvas

crates/lsystem-app/index.html         Iced web Trunk entry
crates/lsystem-app/Trunk.toml         Iced web Trunk config
crates/lsystem-web-app/index.html     Leptos web Trunk entry
crates/lsystem-web-app/Trunk.toml     Leptos web Trunk config

presets/                    bundled TOML L-System definitions
```

## Continuous Integration

Every push and pull request to `main` runs these jobs:

| Job | Commands |
|-----|----------|
| fmt | `cargo fmt --check --all` |
| clippy | `cargo clippy --workspace --all-targets -- -D warnings`; `cargo clippy --workspace --all-features --all-targets -- -D warnings`; `cargo clippy --target wasm32-unknown-unknown --workspace -- -D warnings`; `cargo clippy --target wasm32-unknown-unknown --workspace --all-features -- -D warnings` |
| test | `cargo test --workspace --all-features --all-targets` |
| docs | `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace --all-features` |
| trunk-build | release builds for `crates/lsystem-app/Trunk.toml` and `crates/lsystem-web-app/Trunk.toml` |

Successful CI runs on `main` trigger the deploy workflow, which builds
`crates/lsystem-web-app` with the repository GitHub Pages public URL and deploys
`crates/lsystem-web-app/dist/`.

## Documentation

When changing behavior, commands, architecture, or workflow, update `README.md`,
`CONTRIBUTING.md`, or `AGENTS.md` in the same change when applicable.
