# Configuration

L-systems are authored as strict TOML documents. Parsing preserves the authored
source, while validation produces an editor model and resolution fills omitted
defaults for rendering and export.

## Document shape

A document contains the required tables `[metadata]`, `[l-system]`,
`[l-system.rules]`, and `[colors]`. Unknown top-level fields and unknown fields
in schema-defined tables are rejected. `[l-system.rules]` is an open mapping of
authored production rules; its entries are validated as rule keys and string
values rather than matched against a fixed field list.

### Metadata

| Field | Type | Requirement |
|---|---|---|
| `metadata.name` | string | Required. Used as the workspace display name and the basis of suggested filenames. |

Names do not identify workspace entries and do not need to be unique.

### L-system

| Field | Type | Requirement |
|---|---|---|
| `l-system.dimensions` | string | Required. `"2D"`, `"2d"`, `"3D"`, and `"3d"` are accepted. |
| `l-system.axiom` | string | Required. Validated according to [L-system semantics](l-system.md). |
| `l-system.iterations` | integer | Required. Range `0..=65535`. |
| `l-system.angle` | integer or float | Required. Finite after conversion to `f32`. Expressed in degrees. |
| `l-system.step` | integer or float | Optional. Finite and greater than zero. |
| `l-system.initial_heading` | integer or float | Optional. Finite. Expressed in degrees. |
| `l-system.rules` | table | Required. Keys and values follow the L-system rules below. |

Each `l-system.rules` key is exactly one ASCII letter. Each value is a string.
The axiom and rule strings have whitespace removed before symbol and bracket
validation.

An iteration value outside `0..=65535` is a TOML validation error. This authored
domain is independent of the smaller effective limit used by interactive
applications.

### Colors

The `[colors]` table is required. `colors.background` and `colors.line` are
optional.

Colors use `#rrggbb`: a leading `#` followed by exactly six ASCII hexadecimal
digits. Uppercase and lowercase hexadecimal digits are accepted. Shorthand,
alpha-bearing, named, and functional CSS colors are rejected.

`colors.line` is omitted or contains exactly one of the following externally
tagged modes:

```toml
[colors.line]
solid = "#00e680"
```

```toml
[colors.line.gradient]
start = "#0d590d"
end = "#99e61a"
topological_depth = false
```

```toml
[colors.line.hue_cycle]
initial = "#e60000"
```

The solid value is required when solid mode is present. All three gradient
fields and the hue-cycle `initial` field are optional and resolve independently
from their mode-specific defaults. Mixing line-color modes or adding unknown
fields is rejected.

## Defaults and resolution

Omitted fields resolve to these embedded defaults:

| Field | Default |
|---|---|
| `l-system.step` | `1.0` |
| `l-system.initial_heading` | `0.0` |
| `colors.background` | `#000000` |
| omitted `colors.line` | solid `#00e680` |
| gradient `start` | `#0d590d` |
| gradient `end` | `#99e61a` |
| gradient `topological_depth` | `false` |
| hue-cycle `initial` | `#e60000` |

Resolution does not insert omitted values into the authored document. It
produces a concrete runtime configuration while preserving the editor model’s
distinction between authored and defaulted values.

Interactive applications resolve `iterations` as the smaller of the authored
value and the current interactive maximum. The authored value remains in the
document, including when it exceeds that maximum. See
[Application workspace](application-workspace.md#interactive-iteration-limit).

## Source-preserving edits

Editing raw TOML preserves the draft text exactly until an operation rewrites a
specific value.

Direct controls update the corresponding applied TOML value and preserve
unrelated tables, comments, and formatting. Replacing the structured grammar
replaces the complete `l-system.rules` table, so comments and formatting inside
that table are not preserved. Switching line-color mode removes keys belonging
to the inactive mode.

Removing an optional override removes its authored key and makes resolution
use the embedded default. Removing `colors.line` selects the default solid
mode; it does not select the default values of whichever mode was previously
active.

## Examples

**Non-normative:** This complete 2D example uses solid color:

```toml
[metadata]
name = "Koch Snowflake"

[l-system]
dimensions = "2D"
axiom = "F++F++F"
iterations = 4
angle = 60.0
step = 1.0
initial_heading = 0.0

[l-system.rules]
F = "F-F++F-F"

[colors]
background = "#000000"

[colors.line]
solid = "#00e680"
```

**Non-normative:** This 3D example relies on generation defaults and uses a
topological-depth gradient:

```toml
[metadata]
name = "Branching 3D"

[l-system]
dimensions = "3D"
axiom = "F"
iterations = 5
angle = 25

[l-system.rules]
F = "F[&F][/F][\\F]"

[colors]

[colors.line.gradient]
start = "#22aa66"
end = "#ffee88"
topological_depth = true
```

**Non-normative:** Hue-cycle mode can omit its initial color and inherit the
embedded value:

```toml
[colors.line.hue_cycle]
```
