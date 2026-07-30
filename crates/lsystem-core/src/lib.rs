#[cfg(feature = "svg")]
pub mod svg_export;

pub(crate) mod alphabet;
mod bounds;
mod compiled_generation;
pub mod config;
mod dimension;
mod geometry_stream;
pub mod grammar;
pub mod template;
#[cfg(test)]
pub(crate) mod test_util;
pub(crate) mod turtle;

pub use alphabet::{contains_3d_symbols, validate_bracket_balance, validate_symbols};
pub use bounds::{
    BoundingCylinder3D, Bounds2D, BoundsAccumulator, BoundsAccumulator2D, BoundsAccumulator3D,
};
pub use compiled_generation::{
    AnyCompiledGeneration, CompiledGeneration, CompiledGeneration2D, CompiledGeneration3D,
    GenerationDimension, GenerationPlan, PreparedGeneration, Segment2DWithTopologicalDepth,
    Segment3DWithTopologicalDepth, SegmentWithTopologicalDepth, TemplateBuildError,
};
pub use config::{
    ColorConfig, Config, ConfigError, Dimensions, GenerationConfig, LineColorConfig, Rgb, RgbError,
};
pub use dimension::{D2, D3, Dimension};
pub use geometry_stream::{DepthSegmentStream, DepthStreamSummary, SegmentStream};
pub use grammar::{max_safe_iterations, unused_rules};
pub use template::{
    DEFAULT_TEMPLATE_SEGMENT_BUDGET, Stamp, Stamp2D, Stamp3D, Template, Template2D, Template3D,
    TemplateDimension, TemplateSegment, TemplateSegment2D, TemplateSegment3D, TemplateSet,
    TemplateSet2D, TemplateSet3D,
};
#[doc(hidden)]
pub use turtle::{Turtle, TurtleDimension, turtle2d::TurtleState2D, turtle3d::TurtleState3D};
