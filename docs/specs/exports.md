# Exports

Exports use the resolved configuration and the same geometry and
[color semantics](rendering-and-interaction.md#color-modes) as live rendering.
Suggested filenames follow the rules in
[Application workspace](application-workspace.md#copy-import-rename-and-save).

## Format availability

SVG export accepts only 2D configurations. Passing a 3D configuration to the
SVG boundary returns an error, and applications do not offer SVG while 3D is
selected.

PNG export accepts both 2D and 3D configurations. APNG export accepts both
dimensions at the shared renderer boundary.

**Platform variant:** The primary Leptos app offers SVG, PNG, and APNG for 2D,
and PNG and APNG for 3D.

**Platform variant:** The Iced app offers SVG and PNG for 2D and PNG for 3D. It
does not expose APNG export.

**Platform variant:** Native Iced export opens a save-file dialog and writes the
chosen path. Browser exports initiate a download using a sanitized suggested
filename.

## SVG

SVG output fits generated 2D endpoints into the document view box, includes a
small margin, uses round line caps, and fills the full view box with the
configured background. Turtle Y-up coordinates are displayed with the same
orientation as the live view.

An empty SVG has a `1 × 1` view box and contains only the configured background.
Degenerate horizontal or vertical bounds are padded using half the configured
step before the normal margin is added.

Solid output combines all segments in one path. Other color modes encode each
segment separately.

Live hue-animation phase is not part of a still SVG; the export starts from the
resolved configured color.

## PNG

PNG width and height are each integers in `1..=8192`. Values outside that range
are rejected by the export boundary. Application controls constrain or
normalize user input to the same range.

PNG output is 8-bit RGBA. It uses the configured background, line colors, and
the same depth-tested 3D rendering as the live viewport.

A 2D PNG always fits the generated geometry to the requested output dimensions
and ignores live pan and zoom. A 3D PNG starts from the current camera
orientation and zoom supplied by the application, then renders at the requested
dimensions.

Live hue-animation phase is not part of a still PNG; the export starts from the
resolved configured color.

## APNG

An APNG is an infinitely looping sequence of 8-bit RGBA frames. Width and
height use the PNG range `1..=8192`.

Frame rate is greater than zero. Frame count is in `1..=3600`. Hue phase, hue
rotation speed, and camera auto-rotation speed are finite values.

Frame time is computed analytically as `frame_index / fps`. Frame zero exactly
matches the supplied camera and initial hue phase. Later frames apply hue and
camera rotation from that same baseline rather than accumulating prior-frame
rounding error. Geometry is constant across the animation.

Hue animation changes only hue-cycle line color. Camera animation changes only
the 3D camera; its value has no visual effect on 2D geometry. The application
passes zero speed for an animation option that is disabled or inapplicable.

Progress is reported after each encoded frame as completed frames out of total
frames. An invalid frame count, frame rate, non-finite animation value,
unavailable GPU, scene-capacity failure, readback failure, or encoding failure
terminates export with an error rather than producing a claimed successful
partial result.

**Platform variant:** The primary Leptos app derives frame count by rounding
`duration_seconds × fps`, offers 12, 24, 30, and 60 FPS, and disables saving
when the derived count exceeds 3600. It can set duration to one full enabled
hue or 3D orbit cycle.
