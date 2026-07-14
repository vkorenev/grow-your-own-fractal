#[cfg(feature = "svg")]
pub mod svg_export;

pub(crate) mod alphabet;
mod compiled_generation;
pub mod config;
pub mod grammar;
pub mod template;
#[cfg(test)]
pub(crate) mod test_util;
pub(crate) mod turtle;

pub use alphabet::{contains_3d_symbols, validate_bracket_balance, validate_symbols};
pub use compiled_generation::{
    CompiledGeneration, CompiledGeneration2D, CompiledGeneration3D, Segment2DWithTopologicalDepth,
    Segment3DWithTopologicalDepth,
};
pub use config::{
    ColorConfig, Config, ConfigError, Dimensions, GenerationConfig, LineColorConfig, Rgb, RgbError,
};
pub use grammar::{max_safe_iterations, unused_rules};
pub use template::{
    DEFAULT_TEMPLATE_SEGMENT_BUDGET, Stamp2D, Stamp3D, StampStats, Template2D, Template3D,
    TemplateSegment2D, TemplateSegment3D, TemplateSet2D, TemplateSet3D,
};
