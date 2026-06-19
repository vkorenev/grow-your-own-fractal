#[cfg(feature = "svg")]
pub mod svg_export;

pub(crate) mod alphabet;
pub mod config;
pub mod grammar;
pub(crate) mod turtle;

pub use alphabet::{contains_3d_symbols, validate_bracket_balance, validate_symbols};
pub use config::{
    ColorConfig, Config, ConfigError, Dimensions, GenerationConfig, LineColorConfig, Rgb, RgbError,
};
pub use grammar::{max_safe_iterations, unused_rules};

use glam::{Vec2, Vec3};

/// 2D line segment with turtle topological depth metadata.
///
/// Topological depth is the number of drawn `F` segments from the initial
/// segment along the current branch path. The first emitted `F` has depth `0`;
/// each emitted `F` increments depth after emission; `f` moves without changing
/// depth; `[` and `]` save and restore depth together with turtle state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Segment2DWithTopologicalDepth {
    pub points: [Vec2; 2],
    pub topological_depth: u32,
}

/// 3D line segment with turtle topological depth metadata.
///
/// Semantics match [`Segment2DWithTopologicalDepth`], with the 3D turtle's
/// position and orientation saved/restored by branch stack operations.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Segment3DWithTopologicalDepth {
    pub points: [Vec3; 2],
    pub topological_depth: u32,
}

/// Expand the grammar and run the 2D turtle, returning a lazy iterator of line segments.
pub fn generate(config: &GenerationConfig) -> impl Iterator<Item = [Vec2; 2]> + '_ {
    let chars = grammar::expand(&config.axiom, &config.rules, config.iterations);
    turtle::turtle2d::Segments2D::new(chars, config.angle, config.step, config.initial_heading)
}

/// Expand the grammar and run the 2D turtle with per-segment topological depth.
pub fn generate_with_topological_depth(
    config: &GenerationConfig,
) -> impl Iterator<Item = Segment2DWithTopologicalDepth> + '_ {
    let chars = grammar::expand(&config.axiom, &config.rules, config.iterations);
    turtle::turtle2d::Segments2DWithTopologicalDepth::new(
        chars,
        config.angle,
        config.step,
        config.initial_heading,
    )
}

/// Like `generate` but runs the 3D turtle.
pub fn generate_3d(config: &GenerationConfig) -> impl Iterator<Item = [Vec3; 2]> + '_ {
    let chars = grammar::expand(&config.axiom, &config.rules, config.iterations);
    turtle::turtle3d::Segments3D::new(chars, config.angle, config.step, config.initial_heading)
}

/// Like `generate_with_topological_depth` but runs the 3D turtle.
pub fn generate_3d_with_topological_depth(
    config: &GenerationConfig,
) -> impl Iterator<Item = Segment3DWithTopologicalDepth> + '_ {
    let chars = grammar::expand(&config.axiom, &config.rules, config.iterations);
    turtle::turtle3d::Segments3DWithTopologicalDepth::new(
        chars,
        config.angle,
        config.step,
        config.initial_heading,
    )
}
