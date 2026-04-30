use glam::Vec2;

use super::Turtle;
use crate::{Config, Geometry};

#[derive(Default)]
pub struct Turtle2D {
    position: Vec2,
    heading: f32,
    stack: Vec<(Vec2, f32)>,
}

impl Turtle2D {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Turtle for Turtle2D {
    fn interpret(&mut self, program: &mut dyn Iterator<Item = char>, cfg: &Config) -> Geometry {
        let angle_rad = cfg.angle.to_radians();

        self.position = Vec2::ZERO;
        self.heading = cfg.initial_heading.to_radians();
        self.stack.clear();

        let mut segments = Vec::new();

        for ch in program {
            match ch {
                'F' => {
                    let next = self.position
                        + Vec2::new(self.heading.cos(), self.heading.sin()) * cfg.step;
                    segments.push([self.position, next]);
                    self.position = next;
                }
                'f' => {
                    self.position += Vec2::new(self.heading.cos(), self.heading.sin()) * cfg.step;
                }
                '+' => self.heading += angle_rad,
                '-' => self.heading -= angle_rad,
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
                _ => {} // Non-terminal variable; no drawing command.
            }
        }

        Geometry::D2 { segments }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Config;

    fn parse(toml: &str) -> Config {
        Config::parse(toml).expect("valid test config")
    }

    #[test]
    fn single_f_draws_one_segment() {
        let cfg = parse("name=\"t\"\naxiom=\"F\"\niterations=0\nangle=90.0\nstep=1.0");
        let mut iter = crate::grammar::expand(&cfg.axiom, &cfg.rules, cfg.iterations);
        let geom = Turtle2D::new().interpret(&mut iter, &cfg);
        let Geometry::D2 { segments } = geom;
        assert_eq!(segments.len(), 1);
        let [a, b] = segments[0];
        assert!((a - Vec2::ZERO).length() < 1e-5);
        assert!((b - Vec2::new(1.0, 0.0)).length() < 1e-5);
    }

    #[test]
    fn plus_turns_left() {
        // F+F at 90° → one segment east, turn left 90°, one segment north.
        let cfg = parse("name=\"t\"\naxiom=\"F+F\"\niterations=0\nangle=90.0\nstep=1.0");
        let mut iter = crate::grammar::expand(&cfg.axiom, &cfg.rules, cfg.iterations);
        let geom = Turtle2D::new().interpret(&mut iter, &cfg);
        let Geometry::D2 { segments } = geom;
        assert_eq!(segments.len(), 2);
        // Second segment should start at (1, 0) and go to (1, 1).
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
        let cfg = parse("name=\"t\"\naxiom=\"F[+F]-F\"\niterations=0\nangle=90.0\nstep=1.0");
        let mut iter = crate::grammar::expand(&cfg.axiom, &cfg.rules, cfg.iterations);
        let geom = Turtle2D::new().interpret(&mut iter, &cfg);
        let Geometry::D2 { segments } = geom;
        assert_eq!(segments.len(), 3);
        // Third segment starts back at the saved position (y=0), goes south.
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
        let base = r#"
name = "Koch Snowflake"
dimensions = 2
axiom = "F++F++F"
angle = 60.0
step = 1.0

[rules]
F = "F-F++F-F"
"#;
        for (iters, expected) in [(0u32, 3usize), (1, 12), (2, 48), (3, 192), (4, 768)] {
            let toml = format!("iterations = {iters}\n{base}");
            let cfg = parse(&toml);
            let geom = crate::generate(&cfg);
            let Geometry::D2 { segments } = geom;
            assert_eq!(segments.len(), expected, "iter {iters}");
        }
    }
}
