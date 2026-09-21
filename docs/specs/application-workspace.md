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
any pending draft, without asking for confirmation. A custom copy or import has
no bundled default and cannot be reset. Bundled entries display Reset, enabled
if and only if resetting would change the applied document or discard a pending
draft; custom entries do not display it. A pending raw TOML draft therefore
keeps Reset enabled. Reset is a workspace action, not one of the direct
configuration controls described next.

Direct configuration controls update specific values in the selected entry's
applied document without using the raw TOML editor. They operate on clean
entries. While a raw TOML draft is pending, those controls remain available;
using one discards the pending draft and then applies the control's change. If
that change fails validation, the discarded draft is restored unchanged and
the control's error is reported, so a failed change never loses the pending
draft. Each panel exposing such controls displays a persistent warning while a
draft is pending, noting that using its controls discards the draft.

**Non-normative:** This discard has no undo. The persistent warning is the
only safeguard until a general undo capability exists; adding one is expected
to cover recovery of a discarded draft.

## Copy, import, rename, remove, and save

Copying creates a custom entry with a fresh identity and no bundled default,
appends it to the end of the workspace order, and selects it. Its name starts
with `<current name> copy` and gains the first numeric suffix that makes it
unique. The copy preserves a pending draft, including invalid draft text;
parseable applied and draft documents are renamed to the copy name.

Importing TOML parses and validates it before modifying the workspace. Success
creates a custom entry with the authored name, appends it to the end of the
workspace order, and selects it. Failure leaves the entry list and selection
unchanged. Imported names are permitted to duplicate existing names.

Renaming changes `metadata.name` in the applied source and in a parseable
pending draft. An unparseable pending draft remains verbatim. Names do not need
to be unique.

Removing deletes the selected entry from the session workspace. Custom entries
display Remove; bundled entries do not display it and cannot be removed, so the
workspace always retains at least one entry. Remove is a workspace action, not
one of the direct configuration controls described above, and it is available
while a raw TOML draft is pending.

Removal requires explicit confirmation. The confirmation identifies the entry
and states that any unapplied changes will be discarded. It applies only while
the identified entry remains selected: changing the selection dismisses it and
removes nothing, and cancelling it leaves the workspace unchanged. Confirming
removes the identified entry, never a different one, together with its pending
drafts: the raw TOML draft and, where the application has one, the structured
grammar draft. Removal is an in-session action only: an imported file is never
modified or deleted.

**Non-normative:** Removal provides no undo, like the discards described
earlier; a general undo capability is expected to cover it as well.

After removal, the entry that followed the removed entry in workspace order
becomes selected. If the removed entry was last, the entry that preceded it
becomes selected instead. The newly selected entry is displayed and rendered as
for any other selection change.

Suggested filenames are derived from the applied name. ASCII letters and
digits are lowercased and preserved; every other character becomes `_`; the
requested extension is then appended. Consecutive substitutions are not
collapsed.

Both applications expose preset selection, Copy, Rename, Reset, Remove, and raw
TOML Apply/Revert controls.

**Platform variant:** The primary Leptos app additionally exposes Open and Save
controls. Open imports one `.toml` file. Save downloads the currently displayed
TOML, including an unapplied draft. The Iced app does not expose config-file
Open or config-file Save.

## Persistence

**Platform variant:** The primary Leptos app persists workspace state to local
browser storage across page reloads. The Iced app does not persist workspace
state.

### What persists

Persisted state covers every entry whose applied configuration differs from
its bundled default, plus every custom entry, together with which entry is
selected. Unapplied drafts and non-configuration session state — such as
camera position and rotation and animation or auto-rotation toggles — are
never persisted.

Persistence happens automatically as the workspace changes; no explicit save
action is required, and it is independent of the Save control above, which
downloads a file rather than persisting workspace state. Removing a custom
entry also removes its persisted copy in browser storage, so it does not
return on the next restore; resetting a bundled entry likewise removes its
persisted edit. This concerns only browser storage — as described above,
removal never modifies an imported file.

### Restoring at startup

On startup, the app defers its first render until any persisted state has
been loaded and applied, so the initial render already reflects restored
content rather than momentarily showing bundled defaults first — but only up
to a bounded wait: if loading has not finished within that time, the app
proceeds as if nothing had been persisted, the same as if no persisted state
existed or it failed to load outright. A load that finishes after that point
is discarded rather than retroactively applied, so it can never overwrite
whatever the user has already started doing in the meantime.

A persisted edit to a bundled preset is restored onto that same preset
entry, matched by a stable identity independent of display name, so the
entry's Reset control remains available after restore. A persisted custom
entry keeps a stable identity of its own from creation onward: restoring it
and then editing it further updates that same persisted entry rather than
creating another. This identity is unique to that entry alone — two custom
entries created independently, including in different tabs or windows with
no coordination between them, never end up sharing one identity. Restored
custom entries are independently selectable like any other custom entry and
appear after the bundled presets in the order they were created.

A persisted preset entry whose path no longer matches any currently bundled
preset — for example, the preset was renamed or removed since the edit was
saved — is not discarded: its saved edit is restored as a new custom entry
instead, under the same name, placed after the other restored custom
entries, so a change outside the user's control does not silently lose their
work. Like any custom entry, it no longer has a Reset control, since the
preset it once matched no longer exists to reset to. A persisted entry whose
content fails to parse or validate, by contrast, has nothing salvageable to
preserve and is skipped. Neither case prevents the rest of the persisted
state from restoring or the application from starting.

A persisted selection can fail to resolve only if it named a custom entry
that no longer exists — for example, because it was removed from another
window — or whose own content failed to parse or validate; that entry is
never created, so there is nothing to select. When that happens, restoring
leaves the selection as whatever the rest of the restore has already
produced, falling back as far as the first bundled preset if nothing else
was restored either; the workspace is never left without a selected entry. A
persisted selection that named a preset always resolves — either onto that
preset directly, or, if the preset no longer exists, onto the custom entry
it was converted into as described above — since matching a preset by path
never depends on whether its saved edit itself applied successfully.

### Multiple tabs and windows

The app can be open in several tabs or windows at once. Each entry is
persisted independently, and windows share persisted state by entry identity
rather than replacing one another's:

1. **Saving.** A save adds or updates only the entries that window has
   changed, so it never discards entries saved from another window. A
   restore in any window reflects entries from all of them, including custom
   entries that window never created.
2. **Refreshing.** When a tab or window that was in the background becomes
   active again, it re-reads persisted state and picks up changes saved
   elsewhere in the meantime. An entry is updated only if this window has
   nothing of its own pending in it — no unapplied raw TOML or grammar draft
   and no change not yet saved. Any other entry is left as it is, and its own
   changes persist through the normal save. An updated entry takes the
   persisted content, reverting to its bundled default if it was reset
   elsewhere or disappearing if it was removed elsewhere; entries created
   elsewhere are added to the workspace. A refresh that fails leaves the
   window unchanged.
3. **Selection.** A refresh never changes the selection: the persisted
   selection is used only at startup, and is otherwise whichever window
   changed its selection last. The one exception is a selected entry that is
   removed elsewhere and updated here, in which case the selection moves as
   it would after removing that entry in this window. The displayed and
   rendered configuration follow the selected entry whenever its content is
   updated.
4. **Conflicts.** When two windows change the same entry, whichever saves
   last wins for that entry, and nothing else is affected. A reset or removal
   stands until the entry is changed again: a later change to that entry in
   any window brings it back, so a window that is mid-edit on an entry
   removed elsewhere keeps its work. No prompt or indicator resolves a
   conflict.

### Failures

**Non-normative:** persistence is a best-effort enhancement. The most recent
change is always eventually persisted while the tab remains open, though a
rapid sequence of changes is not guaranteed to persist every intermediate
state, and a best-effort attempt is made to persist the latest change
immediately if the tab is closed or navigated away from. Failures to load,
save, or refresh persisted state do not block startup, editing, or any other
workspace operation — including copying, importing, renaming, removing, and
resetting.

If browser storage fails to open at startup, or any save or refresh attempt
fails, the app shows a small, non-blocking indicator for the remainder of the
session, rather than staying silent about it. The indicator reports only that
persistence is not fully working, never which specific change failed to save,
and it does not reappear or clear itself if persistence starts working again
later in the same session.

## Direct configuration controls

Both applications expose direct controls for the effective iteration count,
angle, and color settings. Angle controls use the interactive range
`1..=180` degrees even though authored TOML accepts any finite angle. Optional
color controls distinguish an authored override from the effective default.

Changing a line-color mode restores remembered control values for that mode
during the session. This memory is transient and does not change the TOML until
the user commits the corresponding control.

**Platform variant:** The primary Leptos app has a structured grammar editor.
Its uncommitted grammar draft is separate from the raw TOML draft, and each
may be pending independently of the other. Applying structured grammar
discards a pending raw TOML draft the same way other direct controls do, then
replaces the axiom and complete rule table; reverting the grammar draft
restores them from the applied document. A successful raw TOML apply
discards a pending grammar draft the same way, resetting the grammar editor
to the newly-applied document; a failed raw TOML apply leaves the pending
grammar draft untouched, same as it leaves the raw draft and previous applied
document untouched. The raw TOML panel displays a persistent warning while a
grammar draft is pending, noting that a successful apply discards it.
Reverting the raw TOML draft leaves a pending grammar draft intact, since
reverting does not change the applied document the grammar draft is based
on. The grammar editor warns about unreachable rules and prevents
selection of 2D while 3D-only symbols remain.

**Non-normative:** Discarding a pending grammar draft this way has no undo,
same as the discards described earlier under "Applied documents and
drafts" — the persistent warning is the only safeguard until a general undo
capability exists.

## Transient controls

Hue rotation is active only for hue-cycle line color. Its speed is clamped to
`1..=60` degrees per second and its direction is forward or reverse. Hue
rotation state and phase are transient rather than authored configuration.

Both applications expose transient on/off and speed controls for camera
auto-rotation in 3D. Camera auto-rotation speed is constrained to `5..=360`
degrees per second in steps of 5.

## Camera pane

Both applications expose a camera pane with a Reset view button and, in 3D,
orbit and roll buttons. Each button performs the same camera action, on the
same target, as its keyboard equivalent defined in
[Rendering and interaction](rendering-and-interaction.md#shared-controls):
Reset view fits and resets the view like `F`; the orbit buttons match the
Left/Right/Up/Down arrow actions; the roll buttons match `Q`/`E`. A pane
button dispatches through the same mechanism as its keyboard equivalent and
introduces no separate targeting behavior of its own. These are discrete
actions, not a pointer drag, so pane buttons do not suspend camera
auto-rotation, the same as their keyboard equivalents.

Orbit and roll buttons are 3D-only and are not shown in 2D, where those
actions have no camera effect. Reset view is available in both dimensions.

Every camera pane button is disabled, rather than staying clickable with no
visible response, in states where its action is known to have no effect.

**Platform variant:** The Iced app's Reset view button is never disabled —
resetting the camera always succeeds, the same as its keyboard equivalent
`F`. Its orbit and roll buttons are disabled specifically while the selected
document's dimension and the currently rendered scene's dimension disagree
(the state a pending dimension-changing regeneration can produce), not for
the duration of every pending regeneration — most regenerations (an
iteration or angle change, a non-dimension grammar edit) never disagree on
dimension in the first place, and gating on all of them would make the
buttons more restrictive than their keyboard equivalents, which have never
depended on regeneration state.

**Platform variant:** The primary Leptos app disables every camera pane
button while no renderer is currently installed (including while briefly
recovering a lost GPU surface), or while the current scene is unavailable
because its most recent rebuild failed. That unavailability is a persistent
state, not a one-render check: a later render that only updates colors, not
geometry, does not clear it, since it does not attempt to rebuild the
missing scene. Only a render that successfully rebuilds the scene clears it.

**Non-normative:** Neither app's coverage is exhaustive. The Iced app has no
recoverable render-failure state to gate on, so a failed generation isn't
covered there. The web app has no signal analogous to Iced's document/scene
dimension check, so there is an unverified, narrow window between a
configuration change and the scene catching up where a button can be enabled
but not yet effective. Each app covers the states it already tracks for
other purposes, not every theoretically possible one.

Camera pane actions are transient viewport actions, independent of the
selected entry's applied document and draft state: they remain available
while a raw TOML or grammar draft is pending, and never apply, revert, or
discard a draft. This is distinct from the workspace Reset control described
above, which restores a bundled entry's original document and discards its
pending draft; the camera pane's Reset view button only affects the camera
and never touches the document.

## Interactive iteration limit

The authored iteration domain is `0..=65535`. Each application additionally
computes a smaller interactive maximum for the selected grammar and dimension.

When iteration zero fits, the maximum is the largest prefix-safe iteration
count, capped at the workload policy ceiling of 30. Predicted drawn-segment
counts are checked from iteration zero upward, and the first count exceeding
the platform-selected GPU record capacity ends the selectable range. Capacity
uses the depth-bearing record size whenever the grammar contains stack
directives, even if the active color mode does not use topological depth.
Changing only the color mode therefore does not change this maximum.

If iteration zero already exceeds capacity, no prefix-safe iteration exists.
The document remains valid, and the application represents the interactive
maximum as zero because the iteration domain has no lower value. The effective
configuration therefore uses iteration zero and the iteration control range is
`0..=0`, but scene construction reports a segment-capacity failure. It does not
truncate the geometry or substitute a fallback scene.

Changing the axiom, rules, or dimension recomputes the maximum. The effective
render configuration clamps iterations to it without rewriting a larger
authored TOML value. The iteration control range is zero through the current
maximum.

## Errors and continuity

Parse, validation, workspace, rendering, and export failures remain visible at
the relevant application boundary. A failed draft apply or import preserves
the previous valid workspace and rendered configuration. Rendering-specific
recovery is defined in [Rendering and interaction](rendering-and-interaction.md#failures-and-recovery).
Persistence failures are the one exception: as described above, they never
block or interrupt anything, and are surfaced — if at all — only through the
coarse, non-blocking indicator described above, never at the point of the
specific action that failed to persist.
