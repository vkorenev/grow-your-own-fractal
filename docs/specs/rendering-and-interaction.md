# Rendering and Interaction

The applications render generated segments on a configured background and
provide dimension-specific camera controls. Export rendering is covered in
[Exports](exports.md).

## Color modes

The background is the resolved `colors.background` value.

Solid mode assigns the configured line color to every segment.

Traversal-gradient mode linearly interpolates RGB from `start` to `end` by
zero-based segment position in traversal order. For two or more segments, the
first segment uses `start` and the last uses `end`. A single segment uses
`start`.

Hue-cycle mode converts `initial` to HSV and preserves its saturation and
value. Hue advances one full 360-degree cycle over traversal order. A single
segment uses the initial color.

A gradient with `topological_depth = true` interpolates by the segment depth
defined in [L-system semantics](l-system.md#segments-and-traversal-order). The
interpolation fraction is the segment depth divided by the greater of the
maximum emitted depth and one. The deepest emitted segment therefore uses
`end` when the maximum depth is nonzero. When every emitted segment has depth
zero, every segment uses `start`. When the grammar contains no stack
directives, this mode falls back to the traversal gradient.

Changing only colors updates rendering without regenerating geometry. Live hue
rotation changes only the hue-cycle color phase. It does not mutate the
configuration or geometry.

## Scene construction and limits

The renderer checks the exact output segment count against the active GPU
record-layout capacity before generating or uploading the scene. A grammar
without stack directives uses position-only records. A grammar with stack
directives uses records that also carry depth, regardless of active color mode;
that metadata affects shading only for a requested topological-depth gradient.

A scene exceeding its applicable capacity fails instead of partially
rendering. Generation strategy, template depth, streaming, staging, buffer
growth, and shader organization are implementation choices and do not change
the required segment order, geometry, or color result.

An empty successful scene contains no line segments and still presents the
configured background. It uses a dimension-appropriate fallback volume so
camera operations remain defined.

## Framing

Initial display and explicit fit/reset center and fit the current scene with a
margin while preserving its aspect ratio.

Two-dimensional framing uses the exact minimum and maximum generated
endpoints. Three-dimensional framing uses a conservative world-Y bounding
cylinder and fits it against the perspective viewport. Three-dimensional line
occlusion is depth-tested by camera distance in the live applications.

The camera has bounded positive zoom. Three-dimensional elevation is clamped
away from the exact poles to `-89..=89` degrees.

## Shared controls

| Input | 2D behavior | 3D behavior |
|---|---|---|
| Primary drag | Pan. | Orbit in direct-manipulation direction. |
| Wheel or scroll gesture | Zoom toward the pointer. | Change camera distance. |
| `F` | Fit and reset the 2D view. | Fit and reset the 3D camera. |
| Left/Right arrows | No camera action. | Change azimuth by 5 degrees. |
| Up/Down arrows | No camera action. | Change elevation by 5 degrees. |
| `Q` / `E` | No camera action. | Roll clockwise / counter-clockwise by 5 degrees. |

Keyboard controls apply while the fractal viewport has focus.

**Platform variant:** The primary Leptos app accepts pointer input and supports
two-pointer pinch zoom around the pointer midpoint. It interprets pointer deltas
in CSS pixels, keeping orbit sensitivity independent of display pixel density.
On narrow layouts, its control sheet remains anchored to the visual viewport as
page zoom or the on-screen keyboard changes the visible area.

**Platform variant:** The Iced app uses mouse input for viewport drag and wheel
navigation.

## Animation

Camera auto-rotation affects only 3D scenes. It advances azimuth around the
world Y axis at the configured speed.

During a pointer orbit, camera auto-rotation is suspended without clearing the
user’s enabled setting. It resumes when the orbit ends or is cancelled. Hue
rotation continues independently during the pointer interaction.

Hue rotation advances its phase only when it is enabled and hue-cycle color is
active. Transient animation state is not written into TOML.

## Failures and recovery

Before the first successful upload, the primary web renderer has no active
scene and clears the canvas to its current background.

A web scene-upload failure replaces the active scene with the no-upload state,
presents only the background, and exposes an actionable viewport error. A later
successful upload, including a successful zero-segment scene, restores the
scene and clears that error.

**Platform variant:** In the primary Leptos app, surface loss or an outdated
surface triggers surface reconfiguration and a subsequent render attempt. GPU
state rebuilding preserves the current camera and colors.

**Platform variant:** The Iced app generates scenes asynchronously and ignores
stale generation results after newer configuration changes. While generation
is pending it retains its application state and reports the pending state in
the controls.
