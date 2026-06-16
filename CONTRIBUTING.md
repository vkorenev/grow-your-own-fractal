# Contributing

Thanks for helping improve Grow Your Own Fractal.

## Start Here

- For local setup, build commands, tests, CI, deploy behavior, and repository
  layout, see [`docs/development.md`](docs/development.md).
- For crate responsibilities, data flow, rendering design, and important
  invariants, see [`docs/architecture.md`](docs/architecture.md).
- For user-facing L-System syntax and TOML examples, see [`README.md`](README.md).

## Quick Setup

Install the Rust toolchain from [`rust-toolchain.toml`](rust-toolchain.toml) and
the pinned `trunk` version from [`mise.toml`](mise.toml):

```sh
mise install
```

Run the native Iced app:

```sh
cargo run -p lsystem-app
```

Run the browser-first Leptos app:

```sh
trunk serve --config crates/lsystem-web-app/Trunk.toml
```

The Leptos app serves on <http://127.0.0.1:8081/>.

## Before Submitting

Use the smallest relevant command while iterating, then run the affected checks
from [`docs/development.md`](docs/development.md#full-verification) before
submitting code changes.

When changing behavior, commands, architecture, config format, deploy workflow,
or contributor workflow, update the relevant documentation in the same change.
