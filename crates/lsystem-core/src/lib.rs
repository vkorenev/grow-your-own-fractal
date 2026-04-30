pub(crate) mod alphabet;
pub mod config;
pub mod geometry;
pub mod grammar;
pub mod turtle;

pub use config::{Config, ConfigError};
pub use geometry::Geometry;
pub use turtle::Turtle;

/// Convenience function: expand the grammar and run the turtle in one call.
pub fn generate(config: &Config) -> Geometry {
    let mut iter = grammar::expand(&config.axiom, &config.rules, config.iterations);
    let mut t = turtle::build(config);
    t.interpret(&mut iter, config)
}
