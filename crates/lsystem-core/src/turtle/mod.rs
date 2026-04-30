pub(crate) mod turtle2d;

use crate::{Config, Geometry};

pub trait Turtle {
    fn interpret(&mut self, program: &mut dyn Iterator<Item = char>, cfg: &Config) -> Geometry;
}

// The cfg parameter is reserved for future dispatch between 2D and 3D turtles.
pub fn build(_cfg: &Config) -> Box<dyn Turtle> {
    Box::new(turtle2d::Turtle2D::new())
}
