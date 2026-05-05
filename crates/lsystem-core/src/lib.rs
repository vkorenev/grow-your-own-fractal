pub(crate) mod alphabet;
pub mod config;
pub mod grammar;
pub(crate) mod turtle;

pub use config::{ColorConfig, Config, ConfigError, LineColorConfig};
pub use grammar::max_safe_iterations;

use glam::Vec2;

/// Expand the grammar and run the 2D turtle, returning a lazy iterator of
/// line segments. The iterator owns all its state and does not borrow from `config`.
pub fn generate(config: &Config) -> impl Iterator<Item = [Vec2; 2]> {
    let chars = grammar::expand_owned(
        config.axiom.clone(),
        config.rules.clone(),
        config.iterations,
    );
    turtle::turtle2d::Segments2D::new(chars, config.angle, config.step, config.initial_heading)
}
