use glam::Vec2;

pub(crate) struct Segments2D<I: Iterator<Item = char>> {
    chars: I,
    angle_rad: f32,
    step: f32,
    position: Vec2,
    heading: f32,
    stack: Vec<(Vec2, f32)>,
}

impl<I: Iterator<Item = char>> Segments2D<I> {
    pub(crate) fn new(chars: I, angle_deg: f32, step: f32, initial_heading_deg: f32) -> Self {
        Self {
            chars,
            angle_rad: angle_deg.to_radians(),
            step,
            position: Vec2::ZERO,
            heading: initial_heading_deg.to_radians(),
            stack: Vec::new(),
        }
    }
}

impl<I: Iterator<Item = char>> Iterator for Segments2D<I> {
    type Item = [Vec2; 2];

    fn next(&mut self) -> Option<[Vec2; 2]> {
        loop {
            match self.chars.next()? {
                'F' => {
                    let next = self.position
                        + Vec2::new(self.heading.cos(), self.heading.sin()) * self.step;
                    let seg = [self.position, next];
                    self.position = next;
                    return Some(seg);
                }
                'f' => {
                    self.position += Vec2::new(self.heading.cos(), self.heading.sin()) * self.step;
                }
                '+' => self.heading += self.angle_rad,
                '-' => self.heading -= self.angle_rad,
                '|' => self.heading += std::f32::consts::PI,
                '[' => self.stack.push((self.position, self.heading)),
                ']' => {
                    let state = self.stack.pop();
                    debug_assert!(state.is_some(), "unmatched ] in validated program");
                    if let Some((pos, head)) = state {
                        self.position = pos;
                        self.heading = head;
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
}
