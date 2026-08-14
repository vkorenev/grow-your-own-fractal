# Project Specifications

These specifications define the supported behavior of Grow Your Own Fractal.
They are the authoritative source for L-system semantics, authored
configuration, application workspace behavior, rendering and interaction, and
exports.

## Reading the specifications

Unqualified declarative statements are normative. For example, “Unknown TOML
fields are rejected” defines required behavior.

The following lead-ins classify material that is not an unconditional shared
requirement:

- **Non-normative:** Background, rationale, examples, recommendations, and
  implementation notes do not define conformance.
- **Optional behavior:** The described capability is permitted but is not
  required of every conforming application.
- **Platform variant:** The described behavior applies only to the named
  application or platform.

The specifications describe intentional supported behavior. An implementation
discrepancy is a defect or a separately recorded conformance gap; it does not
silently amend the specification.

## Specification index

- [L-system semantics](l-system.md) defines grammar expansion, the accepted
  alphabet, turtle interpretation, and generated geometry.
- [Configuration](configuration.md) defines the authored TOML schema,
  validation, defaults, resolution, and source-preserving edits.
- [Application workspace](application-workspace.md) defines presets, custom
  configurations, drafts, direct controls, and application variants.
- [Rendering and interaction](rendering-and-interaction.md) defines color,
  framing, navigation, animation, empty scenes, and rendering failures.
- [Exports](exports.md) defines SVG, PNG, and APNG output and the export
  capabilities exposed by each application.

## Product variants

`lsystem-web-app`, the browser-first Leptos application, is the primary web
application. `lsystem-app` provides the native Iced application and its
retained Iced web build.

Shared requirements apply to every application that exposes the relevant
capability. Differences are identified with **Platform variant:** rather than
implying strict feature parity.

## Documentation ownership

Behavior, configuration, and user-experience contracts live in this directory.
The repository [README](../../README.md) is a product overview, and
[Architecture](../architecture.md) defines crate boundaries, data flow, and
implementation design. [Development](../development.md) defines setup,
verification, CI, and deployment.
