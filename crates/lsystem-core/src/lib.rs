#[cfg(feature = "svg")]
pub mod svg_export;

pub(crate) mod alphabet;
pub mod color_util;
pub mod config;
pub mod config_workspace;
pub mod grammar;
pub(crate) mod turtle;

pub use config::{
    ColorConfig, Config, ConfigDocument, ConfigError, ConfigSource, Dimensions, GenerationConfig,
    LineColorConfig,
};
pub use config_workspace::{
    ConfigEntry, ConfigEntryMutationError, ConfigWorkspace, ConfigWorkspaceError,
};
pub use grammar::max_safe_iterations;

use glam::{Vec2, Vec3};

/// Expand the grammar and run the 2D turtle, returning a lazy iterator of line segments.
pub fn generate(config: &GenerationConfig) -> impl Iterator<Item = [Vec2; 2]> + '_ {
    let chars = grammar::expand(&config.axiom, &config.rules, config.iterations);
    turtle::turtle2d::Segments2D::new(chars, config.angle, config.step, config.initial_heading)
}

/// Like `generate` but runs the 3D turtle.
pub fn generate_3d(config: &GenerationConfig) -> impl Iterator<Item = [Vec3; 2]> + '_ {
    let chars = grammar::expand(&config.axiom, &config.rules, config.iterations);
    turtle::turtle3d::Segments3D::new(chars, config.angle, config.step, config.initial_heading)
}
