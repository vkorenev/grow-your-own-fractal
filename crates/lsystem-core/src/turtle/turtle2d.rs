use glam::Vec2;

use super::Turtle;
use crate::{D2, Segment2DWithTopologicalDepth};

/// Turtle state plus the single symbol transition consumed by `DepthSegments`.
/// Template building also drives it directly to capture the exit state after a
/// rule expansion.
pub(crate) struct TurtleState2D {
    rot_plus: Vec2,
    rot_minus: Vec2,
    inv_step: f32,
    pub(crate) delta: Vec2,
    pub(crate) position: Vec2,
    pub(crate) topological_depth: u32,
    pub(crate) stack: Vec<(Vec2, Vec2, u32)>,
}

impl TurtleState2D {
    pub(crate) fn new(angle_deg: f32, step: f32, initial_heading_deg: f32) -> Self {
        let angle_rad = angle_deg.to_radians();
        Self {
            rot_plus: Vec2::from_angle(angle_rad),
            rot_minus: Vec2::from_angle(-angle_rad),
            inv_step: step.recip(),
            delta: Vec2::from_angle(initial_heading_deg.to_radians()) * step,
            position: Vec2::ZERO,
            topological_depth: 0,
            stack: Vec::new(),
        }
    }

    /// Heading as a rotation (unit complex), relying on `delta` keeping its
    /// initial `step` length (rotations preserve it up to f32 rounding, the
    /// same drift the interpreter accumulates). Falls back to +X when the
    /// heading is degenerate (`step == 0` or non-finite).
    pub(crate) fn heading(&self) -> Vec2 {
        let heading = self.delta * self.inv_step;
        if heading.is_finite() {
            heading
        } else {
            Vec2::X
        }
    }

    /// Exactly normalized [`Self::heading`], for one-time captures whose
    /// result is reused many times (template exit rotations).
    pub(crate) fn normalized_heading(&self) -> Vec2 {
        (self.delta * self.inv_step).normalize_or(Vec2::X)
    }

    /// Composes a rotation into the heading, preserving `delta`'s length.
    pub(crate) fn compose_heading(&mut self, rot: Vec2) {
        self.delta = self.delta.rotate(rot);
    }

    #[inline]
    pub(crate) fn apply(&mut self, symbol: u8) -> Option<Segment2DWithTopologicalDepth> {
        match symbol {
            b'F' => {
                let next = self.position + self.delta;
                let segment = Segment2DWithTopologicalDepth {
                    points: [self.position, next],
                    topological_depth: self.topological_depth,
                };
                self.position = next;
                self.topological_depth = self.topological_depth.saturating_add(1);
                Some(segment)
            }
            b'f' => {
                self.position += self.delta;
                None
            }
            b'+' => {
                self.delta = self.delta.rotate(self.rot_plus);
                None
            }
            b'-' => {
                self.delta = self.delta.rotate(self.rot_minus);
                None
            }
            b'|' => {
                self.delta = -self.delta;
                None
            }
            b'[' => {
                self.stack
                    .push((self.position, self.delta, self.topological_depth));
                None
            }
            b']' => {
                let state = self.stack.pop();
                debug_assert!(state.is_some(), "unmatched ] in validated program");
                if let Some((position, delta, topological_depth)) = state {
                    self.position = position;
                    self.delta = delta;
                    self.topological_depth = topological_depth;
                }
                None
            }
            _ => None,
        }
    }
}

impl Turtle for TurtleState2D {
    type Dimension = D2;

    #[inline]
    fn apply(&mut self, symbol: u8) -> Option<Segment2DWithTopologicalDepth> {
        TurtleState2D::apply(self, symbol)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{FoldOnly, collect_with_next, compile_2d};
    use crate::turtle::DepthSegments;
    use crate::{Dimensions, GenerationConfig};
    use std::collections::BTreeMap;

    fn gen_config(axiom: &str) -> GenerationConfig {
        GenerationConfig::new(
            Dimensions::TwoD,
            axiom.to_string(),
            0,
            90.0,
            1.0,
            0.0,
            BTreeMap::new(),
        )
        .expect("balanced config")
    }

    #[test]
    fn single_f_draws_one_segment() {
        let cfg = gen_config("F");
        let generation = compile_2d(&cfg);
        let segments: Vec<[Vec2; 2]> = generation.segments().collect();
        assert_eq!(segments.len(), 1);
        let [a, b] = segments[0];
        assert!((a - Vec2::ZERO).length() < 1e-5);
        assert!((b - Vec2::new(1.0, 0.0)).length() < 1e-5);
    }

    #[test]
    fn plus_turns_left() {
        let cfg = gen_config("F+F");
        let generation = compile_2d(&cfg);
        let segments: Vec<[Vec2; 2]> = generation.segments().collect();
        assert_eq!(segments.len(), 2);
        let [a, b] = segments[1];
        assert!((a - Vec2::new(1.0, 0.0)).length() < 1e-5);
        assert!((b - Vec2::new(1.0, 1.0)).length() < 1e-5);
    }

    #[test]
    fn bracket_saves_and_restores_state() {
        // F[+F]-F with 90° angle:
        //   F   → (0,0)→(1,0); position=(1,0), heading=0
        //   [   → push (1,0), 0
        //   +F  → turn north, draw (1,0)→(1,1)
        //   ]   → restore position=(1,0), heading=0
        //   -F  → turn south (-90°), draw (1,0)→(1,-1)
        let cfg = gen_config("F[+F]-F");
        let generation = compile_2d(&cfg);
        let segments: Vec<[Vec2; 2]> = generation.segments().collect();
        assert_eq!(segments.len(), 3);
        let [a3, b3] = segments[2];
        assert!(
            (a3 - Vec2::new(1.0, 0.0)).length() < 1e-5,
            "bracket should restore to (1,0)"
        );
        assert!(
            (b3 - Vec2::new(1.0, -1.0)).length() < 1e-5,
            "south step ends at (1,-1)"
        );
    }

    #[test]
    fn koch_segment_count() {
        for (iters, expected) in [(0u16, 3usize), (1, 12), (2, 48), (3, 192), (4, 768)] {
            let cfg = GenerationConfig::new(
                Dimensions::TwoD,
                "F++F++F".to_string(),
                iters,
                60.0,
                1.0,
                0.0,
                BTreeMap::from([('F', "F-F++F-F".to_string())]),
            )
            .expect("balanced config");
            let generation = compile_2d(&cfg);
            let segments: Vec<[Vec2; 2]> = generation.segments().collect();
            assert_eq!(segments.len(), expected, "iter {iters}");
        }
    }

    #[test]
    fn topological_depth_starts_at_zero_and_increments_per_drawn_segment() {
        let cfg = gen_config("FF");
        let generation = compile_2d(&cfg);
        let depths: Vec<u32> = generation
            .depth_segments()
            .map(|segment| segment.topological_depth)
            .collect();

        assert_eq!(depths, [0, 1]);
    }

    #[test]
    fn topological_depth_restores_across_branches() {
        let cfg = gen_config("F[+F]F");
        let generation = compile_2d(&cfg);
        let depths: Vec<u32> = generation
            .depth_segments()
            .map(|segment| segment.topological_depth)
            .collect();

        assert_eq!(depths, [0, 1, 1]);
    }

    #[test]
    fn nested_branches_restore_topological_depth() {
        let cfg = gen_config("F[+F[+F]F]F");
        let generation = compile_2d(&cfg);
        let depths: Vec<u32> = generation
            .depth_segments()
            .map(|segment| segment.topological_depth)
            .collect();

        assert_eq!(depths, [0, 1, 2, 2, 1]);
    }

    #[test]
    fn non_drawing_forward_does_not_increment_topological_depth() {
        let cfg = gen_config("FfF");
        let generation = compile_2d(&cfg);
        let depths: Vec<u32> = generation
            .depth_segments()
            .map(|segment| segment.topological_depth)
            .collect();

        assert_eq!(depths, [0, 1]);
    }

    #[test]
    fn pipe_u_turn_reverses_direction() {
        let cfg = gen_config("F|F");
        let generation = compile_2d(&cfg);
        let segments: Vec<[Vec2; 2]> = generation.segments().collect();
        assert_eq!(segments.len(), 2);
        let [_, b0] = segments[0];
        let [a1, b1] = segments[1];
        assert!(
            (b0 - Vec2::new(1.0, 0.0)).length() < 1e-5,
            "first segment ends at (1,0): {b0}"
        );
        assert!(
            (a1 - Vec2::new(1.0, 0.0)).length() < 1e-5,
            "second segment starts at (1,0): {a1}"
        );
        assert!(
            (b1 - Vec2::ZERO).length() < 1e-5,
            "U-turn returns to origin: {b1}"
        );
    }

    #[test]
    fn koch_snowflake_path_closes() {
        // The Koch snowflake forms a closed curve at every iteration depth.
        // Any significant drift in direction or step size breaks closure.
        //
        // Measured endpoint errors across implementations (iter 4..=6):
        //   current (sin/cos per F): 6e-6 .. 4e-5   — well within 1e-3
        //   proposed (Vec2::rotate): 1e-4 .. 2e-4   — well within 1e-3
        // Iter 4 is the worst case for the proposed recurrence; errors decrease
        // at finer scales due to Koch symmetry cancellation. The range 4..=6
        // guards against regressions from unrelated future changes.
        //
        // Segment-length check: each drawn step must be within 1e-4 of 1.0,
        // catching delta length drift independently of endpoint closure.
        let rules = BTreeMap::from([('F', "F-F++F-F".to_string())]);
        for iters in 4u16..=6 {
            let config = GenerationConfig::new(
                Dimensions::TwoD,
                "F++F++F".to_string(),
                iters,
                60.0,
                1.0,
                0.0,
                rules.clone(),
            )
            .expect("balanced config");
            let generation = compile_2d(&config);
            let segments: Vec<[Vec2; 2]> = generation.segments().collect();
            let end = segments.last().expect("non-empty")[1];
            assert!(
                end.length() < 1e-3,
                "Koch snowflake iter {iters} should close; end at {end}"
            );
            for [a, b] in &segments {
                let len = (*b - *a).length();
                assert!(
                    (len - 1.0).abs() < 1e-4,
                    "iter {iters}: segment length {len:.6} deviates from 1.0"
                );
            }
        }
    }

    #[test]
    fn long_turn_sequence_preserves_delta() {
        // Four 90° left turns complete a full circle. The drawn segment should
        // end near Vec2::X, confirming delta survives repeated rotation without
        // significant direction or length drift.
        let cfg = gen_config("++++F");
        let generation = compile_2d(&cfg);
        let segments: Vec<[Vec2; 2]> = generation.segments().collect();
        assert_eq!(segments.len(), 1);
        let [a, b] = segments[0];
        assert!((a - Vec2::ZERO).length() < 1e-5, "starts at origin: {a}");
        assert!(
            (b - Vec2::X).length() < 1e-4,
            "full circle should draw along X: {b}"
        );
    }

    #[test]
    fn non_default_heading_and_step() {
        // Validates that delta is correctly initialized from a non-default
        // initial heading and step, not just the 0°/1.0 default.
        let config = GenerationConfig::new(
            Dimensions::TwoD,
            "F".to_string(),
            0,
            90.0,
            2.0,
            45.0,
            BTreeMap::new(),
        )
        .expect("balanced config");
        let generation = compile_2d(&config);
        let segments: Vec<[Vec2; 2]> = generation.segments().collect();
        assert_eq!(segments.len(), 1);
        let [a, b] = segments[0];
        let expected = Vec2::new(
            2.0 * 45_f32.to_radians().cos(),
            2.0 * 45_f32.to_radians().sin(),
        );
        assert!((a - Vec2::ZERO).length() < 1e-5, "starts at origin: {a}");
        assert!(
            (b - expected).length() < 1e-5,
            "step=2 at 45° should end near {expected}: {b}"
        );
    }

    #[test]
    fn fold_uses_symbol_fold_for_plain_and_depth_segments() {
        let plain = DepthSegments::new(
            FoldOnly::new(b"F[+F]f-F".iter().copied()),
            TurtleState2D::new(90.0, 1.0, 0.0),
        )
        .map(|segment| segment.points)
        .fold(Vec::new(), |mut segments, segment| {
            segments.push(segment);
            segments
        });
        let with_depth = DepthSegments::new(
            FoldOnly::new(b"F[+F]f-F".iter().copied()),
            TurtleState2D::new(90.0, 1.0, 0.0),
        )
        .fold(Vec::new(), |mut segments, segment| {
            segments.push(segment);
            segments
        });

        assert_eq!(plain.len(), 3);
        assert_eq!(with_depth.len(), 3);
    }

    #[test]
    fn plant_fold_matches_repeated_next_with_depths() {
        let config = GenerationConfig::new(
            Dimensions::TwoD,
            "X".to_string(),
            4,
            23.4,
            1.0,
            90.0,
            BTreeMap::from([
                ('X', "F+[[X]-X]-F[-FX]+X".to_string()),
                ('F', "FF".to_string()),
            ]),
        )
        .expect("balanced config");
        let generation = compile_2d(&config);
        let folded = generation
            .depth_segments()
            .fold(Vec::new(), |mut segments, segment| {
                segments.push(segment);
                segments
            });
        let stepped = collect_with_next(generation.depth_segments());

        assert_eq!(folded, stepped);
    }
}
