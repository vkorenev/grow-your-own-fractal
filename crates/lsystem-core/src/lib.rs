#[cfg(feature = "svg")]
pub mod svg_export;

pub(crate) mod alphabet;
mod compiled_generation;
pub mod config;
mod dimension;
pub mod grammar;
pub mod template;
#[cfg(test)]
pub(crate) mod test_util;
pub(crate) mod turtle;

pub use alphabet::{contains_3d_symbols, validate_bracket_balance, validate_symbols};
pub use compiled_generation::{
    AnyCompiledGeneration, CompiledGeneration, CompiledGeneration2D, CompiledGeneration3D,
    GenerationDimension, Segment2DWithTopologicalDepth, Segment3DWithTopologicalDepth,
    SegmentWithTopologicalDepth,
};
pub use config::{
    ColorConfig, Config, ConfigError, Dimensions, GenerationConfig, LineColorConfig, Rgb, RgbError,
};
pub use dimension::{D2, D3, Dimension};
pub use grammar::{max_safe_iterations, unused_rules};
pub use template::{
    DEFAULT_TEMPLATE_SEGMENT_BUDGET, Stamp, Stamp2D, Stamp3D, StampStats, StampedSegments,
    Template, Template2D, Template3D, TemplateDimension, TemplateSegment, TemplateSegment2D,
    TemplateSegment3D, TemplateSet, TemplateSet2D, TemplateSet3D,
};
