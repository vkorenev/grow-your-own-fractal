# Application Workspace

The applications maintain a session workspace of bundled presets and custom
configuration entries. This specification covers shared state behavior first,
then identifies UI differences between the primary Leptos app and the Iced app.

## Entries and selection

Bundled TOML files from `presets/` are embedded at build time and loaded in
path-sorted order. Invalid bundled files are skipped. Startup fails if no valid
preset remains, and otherwise selects the first valid preset.

Each entry has a stable session-local identity independent of its authored
`metadata.name`. Duplicate names are accepted. A unique name is displayed
unchanged; duplicate names receive ` (1)`, ` (2)`, and later suffixes in
workspace order. Display suffixes are not written into TOML.

Selecting another entry preserves each entry’s pending draft. Selection changes
which last-applied configuration is rendered; an unapplied draft does not
replace the rendered configuration.

## Applied documents and drafts

An entry is clean when its displayed TOML equals its last-applied document. A
text edit that changes those contents creates a draft and makes the entry
dirty. Editing the draft back to the applied text makes the entry clean.

Applying a draft parses and validates the entire document. A successful apply
replaces the last-applied document and clears the draft. A failed apply keeps
both the draft and the previous last-applied configuration unchanged, so the
rendered scene remains usable.

Reverting discards the draft and restores the displayed TOML from the
last-applied document.

Resetting a bundled entry restores its original embedded document and discards
any pending draft. A custom copy or import has no bundled default and cannot be
reset. Applications expose reset only when it has a visible effect.

Direct configuration controls operate on clean entries. While a raw TOML draft
is pending, controls that would mutate the applied document are disabled until
the draft is applied or reverted.

## Copy, import, rename, and save

Copying creates and selects a custom entry with a fresh identity and no bundled
default. Its name starts with `<current name> copy` and gains the first numeric
suffix that makes it unique. The copy preserves a pending draft, including
invalid draft text; parseable applied and draft documents are renamed to the
copy name.

Importing TOML parses and validates it before modifying the workspace. Success
creates and selects a custom entry with the authored name. Failure leaves the
entry list and selection unchanged. Imported names are permitted to duplicate
existing names.

Renaming changes `metadata.name` in the applied source and in a parseable
pending draft. An unparseable pending draft remains verbatim. Names do not need
to be unique.

Suggested filenames are derived from the applied name. ASCII letters and
digits are lowercased and preserved; every other character becomes `_`; the
requested extension is then appended. Consecutive substitutions are not
collapsed.

**Platform variant:** The primary Leptos app exposes Copy, Rename, Reset, Open,
and Save controls. Open imports one `.toml` file. Save downloads the currently
displayed TOML, including an unapplied draft.

**Platform variant:** The Iced app exposes preset selection, Copy, Reset, and
raw TOML Apply/Revert. It does not expose config-file Open, config-file Save, or
Rename controls.

## Structured controls

Both applications expose direct controls for the effective iteration count,
angle, and color settings. Direct angle controls use the interactive range
`1..=180` degrees even though authored TOML accepts any finite angle. Optional
color controls distinguish an authored override from the effective default.

Changing a line-color mode restores remembered control values for that mode
during the session. This memory is transient and does not change the TOML until
the user commits the corresponding control.

Hue rotation is active only for hue-cycle line color. Its speed is clamped to
`1..=60` degrees per second and its direction is forward or reverse. Hue
rotation state and phase are transient rather than authored configuration.

**Platform variant:** The primary Leptos app has a structured grammar editor.
Its uncommitted grammar draft is separate from the raw TOML draft. A grammar
draft disables raw TOML editing, and a raw TOML draft disables structured
controls. Applying structured grammar replaces the axiom and complete rule
table; reverting restores them from the applied document. It warns about
unreachable rules and prevents selection of 2D while 3D-only symbols remain.

## Interactive iteration limit

The authored iteration domain is `0..=65535`. Each application additionally
computes a smaller interactive maximum for the selected grammar and dimension.

The maximum is the largest prefix-safe iteration count, capped at the workload
policy ceiling of 30. Predicted drawn-segment counts are checked from iteration
zero upward, and the first count exceeding the platform-selected GPU record
capacity ends the selectable range. Capacity uses the depth-bearing record size
whenever the grammar contains stack directives, even if the active color mode
does not use topological depth. Changing only the color mode therefore does not
change this maximum.

Changing the axiom, rules, or dimension recomputes the maximum. The effective
render configuration clamps iterations to it without rewriting a larger
authored TOML value. The direct iteration control range is zero through the
current maximum.

## Errors and continuity

Parse, validation, workspace, rendering, and export failures remain visible at
the relevant application boundary. A failed draft apply or import preserves
the previous valid workspace and rendered configuration. Rendering-specific
recovery is defined in [Rendering and interaction](rendering-and-interaction.md#failures-and-recovery).
