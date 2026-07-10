#[cfg(feature = "svg")]
pub mod svg_export;

pub(crate) mod alphabet;
pub mod config;
pub mod grammar;
#[cfg(test)]
pub(crate) mod test_util;
pub(crate) mod turtle;

pub use alphabet::{contains_3d_symbols, validate_bracket_balance, validate_symbols};
pub use config::{
    ColorConfig, Config, ConfigError, Dimensions, GenerationConfig, GenerationParams,
    LineColorConfig, Rgb, RgbError,
};
pub use grammar::{CompiledGrammar, max_safe_iterations, unused_rules};

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

/// Compiles `config`'s grammar and projects its scalar parameters together,
/// so callers get a correctly paired `(CompiledGrammar, GenerationParams)`
/// from a single config in one call instead of deriving each independently.
pub fn compile_generation(config: &GenerationConfig) -> (CompiledGrammar, GenerationParams) {
    (
        CompiledGrammar::compile(config),
        GenerationParams::from(config),
    )
}

/// Expand the grammar and run the 2D turtle, returning a lazy iterator of line segments.
///
/// `grammar` and `params` should be derived from the same [`GenerationConfig`]
/// — prefer [`compile_generation`] over deriving them independently.
/// Nothing enforces the pairing: passing mismatched sources compiles and
/// runs but produces geometry that doesn't correspond to any single config.
pub fn generate<'g>(
    grammar: &'g CompiledGrammar,
    params: &GenerationParams,
) -> impl Iterator<Item = [Vec2; 2]> + 'g {
    let symbols = grammar.expand_effects(params.iterations);
    turtle::turtle2d::Segments2D::new(symbols, params.angle, params.step, params.initial_heading)
}

/// Expand the grammar and run the 2D turtle with per-segment topological depth.
///
/// See [`generate`] for the `grammar`/`params` pairing convention.
pub fn generate_with_topological_depth<'g>(
    grammar: &'g CompiledGrammar,
    params: &GenerationParams,
) -> impl Iterator<Item = Segment2DWithTopologicalDepth> + 'g {
    let symbols = grammar.expand_effects(params.iterations);
    turtle::turtle2d::Segments2DWithTopologicalDepth::new(
        symbols,
        params.angle,
        params.step,
        params.initial_heading,
    )
}

/// Like `generate` but runs the 3D turtle.
///
/// See [`generate`] for the `grammar`/`params` pairing convention.
pub fn generate_3d<'g>(
    grammar: &'g CompiledGrammar,
    params: &GenerationParams,
) -> impl Iterator<Item = [Vec3; 2]> + 'g {
    let symbols = grammar.expand_effects(params.iterations);
    turtle::turtle3d::Segments3D::new(symbols, params.angle, params.step, params.initial_heading)
}

/// Like `generate_with_topological_depth` but runs the 3D turtle.
///
/// See [`generate`] for the `grammar`/`params` pairing convention.
pub fn generate_3d_with_topological_depth<'g>(
    grammar: &'g CompiledGrammar,
    params: &GenerationParams,
) -> impl Iterator<Item = Segment3DWithTopologicalDepth> + 'g {
    let symbols = grammar.expand_effects(params.iterations);
    turtle::turtle3d::Segments3DWithTopologicalDepth::new(
        symbols,
        params.angle,
        params.step,
        params.initial_heading,
    )
}
