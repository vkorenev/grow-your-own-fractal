use glam::{Quat, Vec3};

use crate::Segment3DWithTopologicalDepth;

/// Heading = `orientation * Vec3::X`, left = `* Vec3::Y`, up = `* Vec3::Z`.
pub(crate) struct Segments3D<I: Iterator<Item = u8>> {
    inner: Segments3DWithTopologicalDepth<I>,
}

pub(crate) struct Segments3DWithTopologicalDepth<I: Iterator<Item = u8>> {
    symbols: I,
    state: TurtleState3D,
}

/// Turtle state plus the single symbol transition shared by `next` and `fold`,
/// so the two iteration paths cannot drift apart.
struct TurtleState3D {
    rot_yaw_plus: Quat,
    rot_yaw_minus: Quat,
    rot_pitch_down: Quat,
    rot_pitch_up: Quat,
    rot_roll_right: Quat,
    rot_roll_left: Quat,
    rot_uturn: Quat,
    step: f32,
    position: Vec3,
    orientation: Quat,
    topological_depth: u32,
    stack: Vec<(Vec3, Quat, u32)>,
}

impl TurtleState3D {
    #[inline]
    fn apply(&mut self, symbol: u8) -> Option<Segment3DWithTopologicalDepth> {
        match symbol {
            b'F' => {
                let forward = self.orientation * Vec3::X;
                let next = self.position + forward * self.step;
                let segment = Segment3DWithTopologicalDepth {
                    points: [self.position, next],
                    topological_depth: self.topological_depth,
                };
                self.position = next;
                self.topological_depth = self.topological_depth.saturating_add(1);
                Some(segment)
            }
            b'f' => {
                let forward = self.orientation * Vec3::X;
                self.position += forward * self.step;
                None
            }
            b'+' => {
                self.orientation *= self.rot_yaw_plus;
                None
            }
            b'-' => {
                self.orientation *= self.rot_yaw_minus;
                None
            }
            b'&' => {
                self.orientation *= self.rot_pitch_down;
                None
            }
            b'^' => {
                self.orientation *= self.rot_pitch_up;
                None
            }
            b'/' => {
                self.orientation *= self.rot_roll_right;
                None
            }
            b'\\' => {
                self.orientation *= self.rot_roll_left;
                None
            }
            b'|' => {
                self.orientation *= self.rot_uturn;
                None
            }
            b'[' => {
                self.stack
                    .push((self.position, self.orientation, self.topological_depth));
                None
            }
            b']' => {
                let state = self.stack.pop();
                debug_assert!(state.is_some(), "unmatched ] in validated program");
                if let Some((position, orientation, topological_depth)) = state {
                    self.position = position;
                    self.orientation = orientation;
                    self.topological_depth = topological_depth;
                }
                None
            }
            _ => None,
        }
    }
}

impl<I: Iterator<Item = u8>> Segments3DWithTopologicalDepth<I> {
    pub(crate) fn new(symbols: I, angle_deg: f32, step: f32, initial_heading_deg: f32) -> Self {
        let angle_rad = angle_deg.to_radians();
        Self {
            symbols,
            state: TurtleState3D {
                rot_yaw_plus: Quat::from_rotation_z(angle_rad),
                rot_yaw_minus: Quat::from_rotation_z(-angle_rad),
                rot_pitch_down: Quat::from_rotation_y(angle_rad),
                rot_pitch_up: Quat::from_rotation_y(-angle_rad),
                rot_roll_right: Quat::from_rotation_x(angle_rad),
                rot_roll_left: Quat::from_rotation_x(-angle_rad),
                rot_uturn: Quat::from_rotation_z(std::f32::consts::PI),
                step,
                position: Vec3::ZERO,
                orientation: Quat::from_rotation_z(initial_heading_deg.to_radians()),
                topological_depth: 0,
                stack: Vec::new(),
            },
        }
    }
}

impl<I: Iterator<Item = u8>> Iterator for Segments3DWithTopologicalDepth<I> {
    type Item = Segment3DWithTopologicalDepth;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let symbol = self.symbols.next()?;
            if let Some(segment) = self.state.apply(symbol) {
                return Some(segment);
            }
        }
    }

    fn fold<B, F>(self, init: B, mut f: F) -> B
    where
        Self: Sized,
        F: FnMut(B, Self::Item) -> B,
    {
        let mut state = self.state;
        self.symbols
            .fold(init, |acc, symbol| match state.apply(symbol) {
                Some(segment) => f(acc, segment),
                None => acc,
            })
    }
}

impl<I: Iterator<Item = u8>> Segments3D<I> {
    pub(crate) fn new(symbols: I, angle_deg: f32, step: f32, initial_heading_deg: f32) -> Self {
        Self {
            inner: Segments3DWithTopologicalDepth::new(
                symbols,
                angle_deg,
                step,
                initial_heading_deg,
            ),
        }
    }
}

impl<I: Iterator<Item = u8>> Iterator for Segments3D<I> {
    type Item = [Vec3; 2];

    fn next(&mut self) -> Option<[Vec3; 2]> {
        self.inner.next().map(|segment| segment.points)
    }

    fn fold<B, F>(self, init: B, mut f: F) -> B
    where
        Self: Sized,
        F: FnMut(B, Self::Item) -> B,
    {
        self.inner.fold(init, |acc, segment| f(acc, segment.points))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{FoldOnly, collect_with_next};
    use crate::{Dimensions, GenerationConfig, compile_generation};
    use std::collections::BTreeMap;

    fn make(axiom: &str, angle_deg: f32) -> Vec<[Vec3; 2]> {
        make_with_heading(axiom, angle_deg, 0.0)
    }

    fn make_with_heading(axiom: &str, angle_deg: f32, initial_heading_deg: f32) -> Vec<[Vec3; 2]> {
        Segments3D::new(axiom.bytes(), angle_deg, 1.0, initial_heading_deg).collect()
    }

    #[test]
    fn single_f_draws_along_x() {
        let segs = make("F", 90.0);
        assert_eq!(segs.len(), 1);
        let [a, b] = segs[0];
        assert!(a.distance(Vec3::ZERO) < 1e-5);
        assert!(b.distance(Vec3::X) < 1e-5);
    }

    #[test]
    fn initial_heading_turns_initial_direction() {
        let segs = make_with_heading("F", 90.0, 90.0);
        assert_eq!(segs.len(), 1);
        let [a, b] = segs[0];
        assert!(a.distance(Vec3::ZERO) < 1e-5);
        assert!(b.distance(Vec3::Y) < 1e-5, "end: {b}");
    }

    #[test]
    fn generate_3d_uses_config_initial_heading() {
        let cfg = GenerationConfig::new(
            Dimensions::ThreeD,
            "F".to_string(),
            0,
            90.0,
            1.0,
            90.0,
            BTreeMap::new(),
        )
        .expect("balanced config");

        let (grammar, params) = compile_generation(&cfg);
        let segments: Vec<[Vec3; 2]> = crate::generate_3d(&grammar, &params).collect();
        assert_eq!(segments.len(), 1);
        let [a, b] = segments[0];
        assert!(a.distance(Vec3::ZERO) < 1e-5);
        assert!(b.distance(Vec3::Y) < 1e-5, "end: {b}");
    }

    #[test]
    fn fold_uses_symbol_fold_for_plain_and_depth_segments() {
        let plain = Segments3D::new(FoldOnly::new(br"F[+/F]f-F".iter().copied()), 90.0, 1.0, 0.0)
            .fold(Vec::new(), |mut segments, segment| {
                segments.push(segment);
                segments
            });
        let with_depth = Segments3DWithTopologicalDepth::new(
            FoldOnly::new(br"F[+/F]f-F".iter().copied()),
            90.0,
            1.0,
            0.0,
        )
        .fold(Vec::new(), |mut segments, segment| {
            segments.push(segment);
            segments
        });

        assert_eq!(plain.len(), 3);
        assert_eq!(with_depth.len(), 3);
    }

    #[test]
    fn hilbert_fold_matches_repeated_next_with_depths() {
        let config = GenerationConfig::new(
            Dimensions::ThreeD,
            "X".to_string(),
            3,
            90.0,
            1.0,
            0.0,
            BTreeMap::from([('X', r"^\XF^\XFX-F^//XFX&F+//XFX-F/X-/".to_string())]),
        )
        .expect("balanced config");
        let (grammar, params) = compile_generation(&config);
        let folded = crate::generate_3d_with_topological_depth(&grammar, &params).fold(
            Vec::new(),
            |mut segments, segment| {
                segments.push(segment);
                segments
            },
        );
        let stepped =
            collect_with_next(crate::generate_3d_with_topological_depth(&grammar, &params));

        assert_eq!(folded, stepped);
    }

    #[test]
    fn plus_yaws_left_into_y() {
        let segs = make("F+F", 90.0);
        assert_eq!(segs.len(), 2);
        let [a, b] = segs[1];
        assert!(a.distance(Vec3::X) < 1e-5, "start at X: {a}");
        assert!(b.distance(Vec3::X + Vec3::Y) < 1e-5, "end at X+Y: {b}");
    }

    #[test]
    fn ampersand_pitches_down_into_neg_z() {
        let segs = make("F&F", 90.0);
        assert_eq!(segs.len(), 2);
        let [_, b] = segs[1];
        // After & (90° pitch down): heading was +X, now -Z
        assert!(b.distance(Vec3::X - Vec3::Z) < 1e-5, "end at X-Z: {b}");
    }

    #[test]
    fn bracket_saves_and_restores() {
        // [+F] saves (1,0,0) facing +X, turns left, draws to (1,1,0), restores.
        // After ], heading is back to +X. - turns right to -Y. F draws to (1,-1,0).
        let segs = make("F[+F]-F", 90.0);
        assert_eq!(segs.len(), 3);
        let [a, b] = segs[2];
        assert!(a.distance(Vec3::X) < 1e-5, "restored position: {a}");
        assert!(b.distance(Vec3::new(1.0, -1.0, 0.0)) < 1e-5, "end: {b}");
    }

    #[test]
    fn topological_depth_starts_at_zero_and_increments_per_drawn_segment() {
        let cfg = GenerationConfig::new(
            Dimensions::ThreeD,
            "FF".to_string(),
            0,
            90.0,
            1.0,
            0.0,
            BTreeMap::new(),
        )
        .expect("balanced config");
        let (grammar, params) = compile_generation(&cfg);
        let depths: Vec<u32> = crate::generate_3d_with_topological_depth(&grammar, &params)
            .map(|segment| segment.topological_depth)
            .collect();

        assert_eq!(depths, [0, 1]);
    }

    #[test]
    fn topological_depth_restores_across_branches() {
        let cfg = GenerationConfig::new(
            Dimensions::ThreeD,
            "F[+F]F".to_string(),
            0,
            90.0,
            1.0,
            0.0,
            BTreeMap::new(),
        )
        .expect("balanced config");
        let (grammar, params) = compile_generation(&cfg);
        let depths: Vec<u32> = crate::generate_3d_with_topological_depth(&grammar, &params)
            .map(|segment| segment.topological_depth)
            .collect();

        assert_eq!(depths, [0, 1, 1]);
    }

    #[test]
    fn nested_branches_restore_topological_depth() {
        let cfg = GenerationConfig::new(
            Dimensions::ThreeD,
            "F[+F[+F]F]F".to_string(),
            0,
            90.0,
            1.0,
            0.0,
            BTreeMap::new(),
        )
        .expect("balanced config");
        let (grammar, params) = compile_generation(&cfg);
        let depths: Vec<u32> = crate::generate_3d_with_topological_depth(&grammar, &params)
            .map(|segment| segment.topological_depth)
            .collect();

        assert_eq!(depths, [0, 1, 2, 2, 1]);
    }

    #[test]
    fn moves_and_rotations_do_not_increment_topological_depth() {
        let cfg = GenerationConfig::new(
            Dimensions::ThreeD,
            r"Ff&^/\F".to_string(),
            0,
            90.0,
            1.0,
            0.0,
            BTreeMap::new(),
        )
        .expect("balanced config");
        let (grammar, params) = compile_generation(&cfg);
        let depths: Vec<u32> = crate::generate_3d_with_topological_depth(&grammar, &params)
            .map(|segment| segment.topological_depth)
            .collect();

        assert_eq!(depths, [0, 1]);
    }

    #[test]
    fn pipe_u_turn_reverses_direction() {
        let segs = make("F|F", 90.0);
        assert_eq!(segs.len(), 2);
        let [_, b0] = segs[0];
        let [a1, b1] = segs[1];
        assert!(b0.distance(Vec3::X) < 1e-5, "first segment ends at X: {b0}");
        assert!(
            a1.distance(Vec3::X) < 1e-5,
            "second segment starts at X: {a1}"
        );
        assert!(
            b1.distance(Vec3::ZERO) < 1e-5,
            "U-turn returns to origin: {b1}"
        );
    }

    #[test]
    fn roll_does_not_change_forward_direction() {
        // / and \ rotate around the local forward axis (Vec3::X), so orientation * Vec3::X is unchanged.
        for axiom in [r"/F", r"\F"] {
            let segs = make(axiom, 90.0);
            assert_eq!(segs.len(), 1, "axiom {axiom:?}");
            let [a, b] = segs[0];
            assert!(
                a.distance(Vec3::ZERO) < 1e-5,
                "starts at origin ({axiom:?})"
            );
            assert!(
                b.distance(Vec3::X) < 1e-5,
                "roll should not change forward ({axiom:?}): {b}"
            );
        }
    }

    #[test]
    fn roll_affects_subsequent_yaw_plane() {
        // Without prior roll, +F draws along Y (see plus_yaws_left_into_y).
        // After rolling 90° right, local up shifts from Z to -Y, so a subsequent
        // yaw left sweeps around the new local up and draws along Z instead.
        let segs = make(r"/+F", 90.0);
        assert_eq!(segs.len(), 1);
        let [a, b] = segs[0];
        assert!(a.distance(Vec3::ZERO) < 1e-5);
        assert!(
            b.distance(Vec3::Z) < 1e-5,
            "after /+, F should move along Z: {b}"
        );
    }

    #[test]
    fn branch_restore_after_roll() {
        // [/+F] draws along Z (see roll_affects_subsequent_yaw_plane).
        // After ], position and orientation are fully restored so the following
        // F draws along the original X axis.
        let segs = make(r"[/+F]F", 90.0);
        assert_eq!(segs.len(), 2);
        let [a0, b0] = segs[0];
        let [a1, b1] = segs[1];
        assert!(
            a0.distance(Vec3::ZERO) < 1e-5,
            "branch starts at origin: {a0}"
        );
        assert!(b0.distance(Vec3::Z) < 1e-5, "branch F ends at Z: {b0}");
        assert!(
            a1.distance(Vec3::ZERO) < 1e-5,
            "position restored to origin: {a1}"
        );
        assert!(
            b1.distance(Vec3::X) < 1e-5,
            "forward restored to X after branch: {b1}"
        );
    }

    #[test]
    fn roll_left_affects_subsequent_yaw_plane() {
        // Without prior roll, +F draws along Y (see plus_yaws_left_into_y).
        // After rolling 90° left, local up shifts from Z to Y, so a subsequent
        // yaw left sweeps around the new local up and draws along -Z instead.
        let segs = make(r"\+F", 90.0);
        assert_eq!(segs.len(), 1);
        let [_, b] = segs[0];
        assert!(
            b.distance(-Vec3::Z) < 1e-5,
            r"after \+, F should move along -Z: {b}"
        );
    }

    #[test]
    fn minus_yaws_right() {
        let segs = make("-F", 90.0);
        assert_eq!(segs.len(), 1);
        let [_, b] = segs[0];
        assert!(b.distance(-Vec3::Y) < 1e-5, "-F should move along -Y: {b}");
    }

    #[test]
    fn caret_pitches_up() {
        let segs = make("^F", 90.0);
        assert_eq!(segs.len(), 1);
        let [_, b] = segs[0];
        assert!(b.distance(Vec3::Z) < 1e-5, "^F should move along Z: {b}");
    }

    #[test]
    fn non_drawing_forward_uses_cached_direction() {
        // After +, f moves without drawing in the yawed direction.
        // The subsequent F draws from that new position along the same direction.
        // Confirms that f correctly uses (and does not skip) a dirty forward cache.
        let segs = make("+fF", 90.0);
        assert_eq!(segs.len(), 1);
        let [a, b] = segs[0];
        assert!(a.distance(Vec3::Y) < 1e-5, "f should move to Y: {a}");
        assert!(b.distance(2.0 * Vec3::Y) < 1e-5, "F should draw to 2Y: {b}");
    }

    #[test]
    fn branch_dirty_save_and_restore() {
        // + yaws left, making the forward cache dirty. [ saves that dirty state.
        // Inside the branch, F recomputes forward (Y) and draws from origin to Y.
        // ] restores position to origin and the saved dirty state. The outer F
        // also recomputes forward (Y) and draws from origin to Y.
        let segs = make("+[F]F", 90.0);
        assert_eq!(segs.len(), 2);
        let [a0, b0] = segs[0];
        let [a1, b1] = segs[1];
        assert!(
            a0.distance(Vec3::ZERO) < 1e-5,
            "branch F starts at origin: {a0}"
        );
        assert!(b0.distance(Vec3::Y) < 1e-5, "branch F draws to Y: {b0}");
        assert!(
            a1.distance(Vec3::ZERO) < 1e-5,
            "position restored to origin: {a1}"
        );
        assert!(
            b1.distance(Vec3::Y) < 1e-5,
            "outer F draws to Y after dirty restore: {b1}"
        );
    }
}
