# L-System Semantics

This specification defines the authored L-system language and the geometry it
produces. Configuration syntax and validation errors are specified separately
in [Configuration](configuration.md).

## Grammar expansion

An L-system consists of an axiom, zero or more production rules, and an
iteration count.

One iteration rewrites every symbol in the current sequence simultaneously.
A symbol with a production rule is replaced by that rule’s right-hand side. A
symbol without a rule remains unchanged in the expanded sequence. An empty
rule right-hand side is valid.

Iteration zero produces the axiom unchanged. Iteration `n + 1` rewrites the
complete result of iteration `n`. Rewriting preserves left-to-right sequence
order.

Only rules reachable from the axiom affect expansion. An authored but
unreachable rule is valid and has no effect on expansion.

**Non-normative:** Implementations can expand lazily and can use templates or
other optimizations instead of materializing the expanded string.

## Alphabet

Every non-whitespace character in the axiom and rule right-hand sides is an
ASCII letter or a reserved turtle symbol.

The following symbols are valid in both dimensions:

| Symbol | Meaning |
|---|---|
| `F` | Move forward one step and emit a line segment. |
| `f` | Move forward one step without emitting a segment. |
| `+` | Turn left by `angle`. |
| `-` | Turn right by `angle`. |
| `\|` | Turn 180 degrees around the current up axis. |
| `[` | Push the current turtle state. |
| `]` | Restore the most recently pushed turtle state. |
| `A`–`Z`, `a`–`z` | A letter available as a production-rule symbol or non-terminal. |

The following symbols are valid only in 3D:

| Symbol | Meaning |
|---|---|
| `&` | Pitch down by `angle`. |
| `^` | Pitch up by `angle`. |
| `/` | Roll right by `angle`. |
| `\` | Roll left by `angle`. |

A 2D axiom or rule containing `&`, `^`, `/`, or `\` is invalid. Any character
outside the dimension’s alphabet is invalid.

Rule keys are single ASCII letters. Reserved turtle punctuation cannot be a
rule key, so rewriting cannot remove or introduce imbalance by replacing a
bracket itself.

Whitespace in the axiom and rule right-hand sides is removed before alphabet
and bracket validation. Whitespace therefore has no expansion or turtle
effect.

Each of the axiom and every rule right-hand side is independently bracket
balanced. A closing bracket never precedes its matching opening bracket. These
conditions keep every expanded sequence balanced.

## Turtle state

The turtle starts at the origin with a heading along positive X. The
`initial_heading` rotates that heading in degrees around positive Z; positive
angles are counter-clockwise in the XY plane. Each forward move has length
`step`.

In 2D the state contains position, heading, and topological depth. In 3D it
contains position, full orientation, and topological depth. The 3D local axes
are heading, left, and up; yaw, pitch, and roll compose around those local
axes.

`[` saves the complete current state and `]` restores it. Restoring a state
does not emit a segment.

Letters that have no reserved turtle action are silently skipped during turtle
interpretation. This differs from grammar expansion: an unruled letter remains
in the expanded sequence but has no geometric effect when interpreted.

## Segments and traversal order

Every interpreted `F` emits one segment from the position before the move to
the position after the move. `f` changes position without emitting a segment.
All other symbols change or restore state without emitting a segment.

Segments are ordered by left-to-right turtle traversal of the fully expanded
sequence. This order defines traversal gradients and hue cycles.

The topological depth of the first emitted segment on a path is zero. Emitting
an `F` records the current depth and then increments it. An `f` does not change
depth. Push and pop save and restore depth together with position and heading.

## Strategy equivalence

Direct interpretation and optimized generation produce the same segment
count, traversal order, branch-depth values, and dimension. Corresponding
coordinates agree within the floating-point tolerance appropriate to repeated
`f32` rotations and transformations.
