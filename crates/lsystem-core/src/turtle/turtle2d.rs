use glam::Vec2;

use crate::Segment2DWithTopologicalDepth;

pub(crate) struct Segments2D<I: Iterator<Item = u8>> {
    inner: Segments2DWithTopologicalDepth<I>,
}

pub(crate) struct Segments2DWithTopologicalDepth<I: Iterator<Item = u8>> {
    symbols: I,
    rot_plus: Vec2,
    rot_minus: Vec2,
    delta: Vec2,
    position: Vec2,
    topological_depth: u32,
    stack: Vec<(Vec2, Vec2, u32)>,
}

impl<I: Iterator<Item = u8>> Segments2DWithTopologicalDepth<I> {
    pub(crate) fn new(symbols: I, angle_deg: f32, step: f32, initial_heading_deg: f32) -> Self {
        let angle_rad = angle_deg.to_radians();
        Self {
            symbols,
            rot_plus: Vec2::from_angle(angle_rad),
            rot_minus: Vec2::from_angle(-angle_rad),
            delta: Vec2::from_angle(initial_heading_deg.to_radians()) * step,
            position: Vec2::ZERO,
            topological_depth: 0,
            stack: Vec::new(),
        }
    }
}

impl<I: Iterator<Item = u8>> Iterator for Segments2DWithTopologicalDepth<I> {
    type Item = Segment2DWithTopologicalDepth;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.symbols.next()? {
                b'F' => {
                    let next = self.position + self.delta;
                    let segment = Segment2DWithTopologicalDepth {
                        points: [self.position, next],
                        topological_depth: self.topological_depth,
                    };
                    self.position = next;
                    self.topological_depth = self.topological_depth.saturating_add(1);
                    return Some(segment);
                }
                b'f' => {
                    self.position += self.delta;
                }
                b'+' => self.delta = self.delta.rotate(self.rot_plus),
                b'-' => self.delta = self.delta.rotate(self.rot_minus),
                b'|' => self.delta = -self.delta,
                b'[' => self
                    .stack
                    .push((self.position, self.delta, self.topological_depth)),
                b']' => {
                    let state = self.stack.pop();
                    debug_assert!(state.is_some(), "unmatched ] in validated program");
                    if let Some((pos, delta, topological_depth)) = state {
                        self.position = pos;
                        self.delta = delta;
                        self.topological_depth = topological_depth;
                    }
                }
                _ => {}
            }
        }
    }
}

impl<I: Iterator<Item = u8>> Segments2D<I> {
    pub(crate) fn new(symbols: I, angle_deg: f32, step: f32, initial_heading_deg: f32) -> Self {
        Self {
            inner: Segments2DWithTopologicalDepth::new(
                symbols,
                angle_deg,
                step,
                initial_heading_deg,
            ),
        }
    }
}

impl<I: Iterator<Item = u8>> Iterator for Segments2D<I> {
    type Item = [Vec2; 2];

    fn next(&mut self) -> Option<[Vec2; 2]> {
        self.inner.next().map(|segment| segment.points)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Dimensions, GenerationConfig};
    use std::collections::BTreeMap;

    fn gen_config(axiom: &str) -> GenerationConfig {
        GenerationConfig {
            dimensions: Dimensions::TwoD,
            axiom: axiom.to_string(),
            iterations: 0,
            angle: 90.0,
            step: 1.0,
            initial_heading: 0.0,
            rules: BTreeMap::new(),
        }
    }

    #[test]
    fn single_f_draws_one_segment() {
        let cfg = gen_config("F");
        let segments: Vec<[Vec2; 2]> = crate::generate(&cfg).collect();
        assert_eq!(segments.len(), 1);
        let [a, b] = segments[0];
        assert!((a - Vec2::ZERO).length() < 1e-5);
        assert!((b - Vec2::new(1.0, 0.0)).length() < 1e-5);
    }

    #[test]
    fn plus_turns_left() {
        let cfg = gen_config("F+F");
        let segments: Vec<[Vec2; 2]> = crate::generate(&cfg).collect();
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
        let segments: Vec<[Vec2; 2]> = crate::generate(&cfg).collect();
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
        for (iters, expected) in [(0u32, 3usize), (1, 12), (2, 48), (3, 192), (4, 768)] {
            let cfg = GenerationConfig {
                dimensions: Dimensions::TwoD,
                axiom: "F++F++F".to_string(),
                iterations: iters,
                angle: 60.0,
                step: 1.0,
                initial_heading: 0.0,
                rules: BTreeMap::from([('F', "F-F++F-F".to_string())]),
            };
            let segments: Vec<[Vec2; 2]> = crate::generate(&cfg).collect();
            assert_eq!(segments.len(), expected, "iter {iters}");
        }
    }

    #[test]
    fn topological_depth_starts_at_zero_and_increments_per_drawn_segment() {
        let cfg = gen_config("FF");
        let depths: Vec<u32> = crate::generate_with_topological_depth(&cfg)
            .map(|segment| segment.topological_depth)
            .collect();

        assert_eq!(depths, [0, 1]);
    }

    #[test]
    fn topological_depth_restores_across_branches() {
        let cfg = gen_config("F[+F]F");
        let depths: Vec<u32> = crate::generate_with_topological_depth(&cfg)
            .map(|segment| segment.topological_depth)
            .collect();

        assert_eq!(depths, [0, 1, 1]);
    }

    #[test]
    fn nested_branches_restore_topological_depth() {
        let cfg = gen_config("F[+F[+F]F]F");
        let depths: Vec<u32> = crate::generate_with_topological_depth(&cfg)
            .map(|segment| segment.topological_depth)
            .collect();

        assert_eq!(depths, [0, 1, 2, 2, 1]);
    }

    #[test]
    fn non_drawing_forward_does_not_increment_topological_depth() {
        let cfg = gen_config("FfF");
        let depths: Vec<u32> = crate::generate_with_topological_depth(&cfg)
            .map(|segment| segment.topological_depth)
            .collect();

        assert_eq!(depths, [0, 1]);
    }

    #[test]
    fn pipe_u_turn_reverses_direction() {
        let cfg = gen_config("F|F");
        let segments: Vec<[Vec2; 2]> = crate::generate(&cfg).collect();
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
        for iters in 4u32..=6 {
            let config = GenerationConfig {
                dimensions: Dimensions::TwoD,
                axiom: "F++F++F".to_string(),
                iterations: iters,
                angle: 60.0,
                step: 1.0,
                initial_heading: 0.0,
                rules: rules.clone(),
            };
            let segments: Vec<[Vec2; 2]> = crate::generate(&config).collect();
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
        let segments: Vec<[Vec2; 2]> = crate::generate(&cfg).collect();
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
        let config = GenerationConfig {
            dimensions: Dimensions::TwoD,
            axiom: "F".to_string(),
            iterations: 0,
            angle: 90.0,
            step: 2.0,
            initial_heading: 45.0,
            rules: BTreeMap::new(),
        };
        let segments: Vec<[Vec2; 2]> = crate::generate(&config).collect();
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
}
