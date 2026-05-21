use glam::{Quat, Vec3};

/// Heading = `orientation * Vec3::X`, left = `* Vec3::Y`, up = `* Vec3::Z`.
pub(crate) struct Segments3D<I: Iterator<Item = char>> {
    chars: I,
    angle_rad: f32,
    step: f32,
    position: Vec3,
    orientation: Quat,
    stack: Vec<(Vec3, Quat)>,
}

impl<I: Iterator<Item = char>> Segments3D<I> {
    pub(crate) fn new(chars: I, angle_deg: f32, step: f32, initial_heading_deg: f32) -> Self {
        Self {
            chars,
            angle_rad: angle_deg.to_radians(),
            step,
            position: Vec3::ZERO,
            orientation: Quat::from_rotation_z(initial_heading_deg.to_radians()),
            stack: Vec::new(),
        }
    }
}

impl<I: Iterator<Item = char>> Iterator for Segments3D<I> {
    type Item = [Vec3; 2];

    fn next(&mut self) -> Option<[Vec3; 2]> {
        loop {
            match self.chars.next()? {
                'F' => {
                    let forward = self.orientation * Vec3::X;
                    let next = self.position + forward * self.step;
                    let seg = [self.position, next];
                    self.position = next;
                    return Some(seg);
                }
                'f' => {
                    let forward = self.orientation * Vec3::X;
                    self.position += forward * self.step;
                }
                '+' => {
                    self.orientation *= Quat::from_rotation_z(self.angle_rad);
                }
                '-' => {
                    self.orientation *= Quat::from_rotation_z(-self.angle_rad);
                }
                '&' => {
                    self.orientation *= Quat::from_rotation_y(self.angle_rad);
                }
                '^' => {
                    self.orientation *= Quat::from_rotation_y(-self.angle_rad);
                }
                '/' => {
                    self.orientation *= Quat::from_rotation_x(self.angle_rad);
                }
                '\\' => {
                    self.orientation *= Quat::from_rotation_x(-self.angle_rad);
                }
                '|' => {
                    self.orientation *= Quat::from_rotation_z(std::f32::consts::PI);
                }
                '[' => self.stack.push((self.position, self.orientation)),
                ']' => {
                    let state = self.stack.pop();
                    debug_assert!(state.is_some(), "unmatched ] in validated program");
                    if let Some((pos, orient)) = state {
                        self.position = pos;
                        self.orientation = orient;
                    }
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Dimensions, GenerationConfig};
    use std::collections::BTreeMap;

    fn make(axiom: &str, angle_deg: f32) -> Vec<[Vec3; 2]> {
        make_with_heading(axiom, angle_deg, 0.0)
    }

    fn make_with_heading(axiom: &str, angle_deg: f32, initial_heading_deg: f32) -> Vec<[Vec3; 2]> {
        Segments3D::new(axiom.chars(), angle_deg, 1.0, initial_heading_deg).collect()
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
        let cfg = GenerationConfig {
            dimensions: Dimensions::ThreeD,
            axiom: "F".to_string(),
            iterations: 0,
            angle: 90.0,
            step: 1.0,
            initial_heading: 90.0,
            rules: BTreeMap::new(),
        };

        let segments: Vec<[Vec3; 2]> = crate::generate_3d(&cfg).collect();
        assert_eq!(segments.len(), 1);
        let [a, b] = segments[0];
        assert!(a.distance(Vec3::ZERO) < 1e-5);
        assert!(b.distance(Vec3::Y) < 1e-5, "end: {b}");
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
}
