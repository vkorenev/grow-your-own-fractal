# AGENTS.md

This file provides repository-specific guidance for AI coding agents.

> **REQUIRED before any interaction with Git or GitHub** — including `git`
> commands, the `gh` CLI, GitHub MCP tools, or any other mechanism that reads or
> writes repository or pull-request state: read
> [`.agents/rules/git-and-github.md`](.agents/rules/git-and-github.md).

## Shared Project Docs

Use the shared docs instead of duplicating long-lived project knowledge here:

- [`README.md`](README.md) — user-facing product overview and documentation
  entry points.
- [`CONTRIBUTING.md`](CONTRIBUTING.md) — contributor entry point.
- [`docs/development.md`](docs/development.md) — setup, commands, verification,
  CI, deploy, and repository layout.
- [`docs/architecture.md`](docs/architecture.md) — crate boundaries, data flow,
  rendering design, config model, exports, and key invariants.
- [`docs/specs/`](docs/specs/README.md) — authoritative L-system,
  configuration, application, rendering/interaction, and export behavior.

Read the relevant shared docs before making changes in that area. For example,
read `docs/architecture.md` before changing crate boundaries or render/config
data flow, and read `docs/development.md` before changing commands, CI, deploy,
or tooling. Read the relevant specification before changing supported behavior.

## Agent Rules

- Keep `AGENTS.md` focused on agent-only workflow. Put durable architecture,
  setup, and contributor information in `docs/` or `CONTRIBUTING.md`.
- When changing behavior, commands, architecture, config format, deploy
  workflow, or contributor workflow, update the relevant docs in the same
  change.
- Treat `docs/specs/` as shipped, Git-tracked documentation. The working
  directories `docs/plans/`, `docs/proposals/`, `docs/design/`,
  `docs/research/`, `docs/reviews/`, and `docs/notes/` remain untracked unless
  a task explicitly promotes a file. Never stage `docs/` broadly; stage only
  the intended specification and tracked-document paths.
- The primary browser app is `lsystem-web-app`; `lsystem-app` provides the
  native Iced app and retained Iced web build.
- Preserve crate boundaries from `docs/architecture.md`. In particular,
  `lsystem-core` stays free of rendering/TOML/UI/platform dependencies, and
  `lsystem-app-model` stays toolkit- and renderer-independent.
- `wgpu` and the pinned Iced git revision are coupled. Do not update either one
  independently; update and verify them together.
- Prefer focused verification while iterating, then run the relevant checks from
  `docs/development.md` before claiming a code change is complete.
