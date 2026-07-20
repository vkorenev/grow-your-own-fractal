use std::marker::PhantomData;

use crate::{
    D2, D3, Dimension,
    config::GenerationConfig,
    grammar::CompiledGrammar,
    turtle::{DepthSegments, Turtle, TurtleDimension},
};

/// Dimension-erased result of `GenerationConfig::compile`; match once at a
/// runtime boundary to obtain the typed `CompiledGeneration<D>`.
#[derive(Debug)]
pub enum AnyCompiledGeneration {
    TwoD(CompiledGeneration2D),
    ThreeD(CompiledGeneration3D),
}

#[derive(Debug)]
/// A dimension-typed compiled generation whose grammar and scalar parameters
/// stay paired.
pub struct CompiledGeneration<D: Dimension> {
    pub(crate) grammar: CompiledGrammar,
    pub(crate) params: GenerationParams,
    has_stack_directives: bool,
    // `fn() -> D` keeps auto-traits independent of the marker type.
    dimension: PhantomData<fn() -> D>,
}

/// A compiled 2D generation.
///
/// A 3D value cannot be passed to a 2D template set:
///
/// ```compile_fail
/// use lsystem_core::{CompiledGeneration3D, TemplateSet2D};
///
/// fn build_wrong_dimension(generation: CompiledGeneration3D) {
///     let _ = TemplateSet2D::build(generation, 1);
/// }
/// ```
pub type CompiledGeneration2D = CompiledGeneration<D2>;

/// A compiled 3D generation.
///
/// A 2D value cannot be passed to a 3D template set:
///
/// ```compile_fail
/// use lsystem_core::{CompiledGeneration2D, TemplateSet3D};
///
/// fn build_wrong_dimension(generation: CompiledGeneration2D) {
///     let _ = TemplateSet3D::build(generation, 1);
/// }
/// ```
pub type CompiledGeneration3D = CompiledGeneration<D3>;

#[derive(Debug, Clone, Copy)]
pub(crate) struct GenerationParams {
    pub(crate) iterations: u16,
    pub(crate) angle: f32,
    pub(crate) step: f32,
    pub(crate) initial_heading: f32,
}

/// Line segment with turtle topological depth metadata.
///
/// Topological depth is the number of drawn `F` segments from the initial
/// segment along the current branch path. The first emitted `F` has depth `0`;
/// each emitted `F` increments depth after emission; `f` moves without changing
/// depth; `[` and `]` save and restore depth together with turtle state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SegmentWithTopologicalDepth<D: Dimension> {
    pub points: [D::Point; 2],
    pub topological_depth: u32,
}

/// A 2D line segment with turtle topological depth metadata.
pub type Segment2DWithTopologicalDepth = SegmentWithTopologicalDepth<D2>;

/// A 3D line segment with turtle topological depth metadata.
///
/// Its semantics match [`Segment2DWithTopologicalDepth`], with the 3D turtle's
/// position and orientation saved/restored by branch stack operations.
pub type Segment3DWithTopologicalDepth = SegmentWithTopologicalDepth<D3>;

/// Dimension-specific turtle construction used by generic geometry iteration.
///
/// Implemented only for [`D2`] and [`D3`] via a blanket impl; add it as a
/// bound on your own code that is generic over dimension, then call
/// [`CompiledGeneration::segments`] or [`CompiledGeneration::depth_segments`]
/// rather than this trait's method directly.
pub trait GenerationDimension: Dimension {
    fn depth_segments(
        generation: &CompiledGeneration<Self>,
    ) -> impl Iterator<Item = SegmentWithTopologicalDepth<Self>> + '_;
}

impl<D: TurtleDimension> GenerationDimension for D {
    fn depth_segments(
        generation: &CompiledGeneration<Self>,
    ) -> impl Iterator<Item = SegmentWithTopologicalDepth<Self>> + '_ {
        let p = generation.params;
        DepthSegments::new(
            generation.grammar.expand_effects(p.iterations),
            <D::Turtle as Turtle>::new(p.angle, p.step, p.initial_heading),
        )
    }
}

impl GenerationConfig {
    pub fn compile(&self) -> AnyCompiledGeneration {
        match self.dimensions {
            crate::Dimensions::TwoD => AnyCompiledGeneration::TwoD(self.compile_for::<D2>()),
            crate::Dimensions::ThreeD => AnyCompiledGeneration::ThreeD(self.compile_for::<D3>()),
        }
    }

    fn compile_for<D: Dimension>(&self) -> CompiledGeneration<D> {
        CompiledGeneration {
            grammar: CompiledGrammar::compile(self),
            params: GenerationParams {
                iterations: self.iterations,
                angle: self.angle,
                step: self.step,
                initial_heading: self.initial_heading,
            },
            has_stack_directives: self.has_stack_directives(),
            dimension: PhantomData,
        }
    }
}

impl<D: Dimension> CompiledGeneration<D> {
    pub fn has_stack_directives(&self) -> bool {
        self.has_stack_directives
    }

    /// Number of drawn segments in this generation without expanding it.
    ///
    /// The count is exact while representable and saturates at `u64::MAX`.
    pub fn drawn_segment_count(&self) -> u64 {
        self.grammar.drawn_segment_count(self.params.iterations)
    }
}

impl<D: GenerationDimension> CompiledGeneration<D> {
    pub fn segments(&self) -> impl Iterator<Item = [D::Point; 2]> + '_ {
        D::depth_segments(self).map(|segment| segment.points)
    }

    pub fn depth_segments(&self) -> impl Iterator<Item = SegmentWithTopologicalDepth<D>> + '_ {
        D::depth_segments(self)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::Dimensions;

    fn config(dimensions: Dimensions) -> GenerationConfig {
        GenerationConfig::new(
            dimensions,
            "F[+F]F".to_string(),
            0,
            90.0,
            1.0,
            0.0,
            BTreeMap::new(),
        )
        .expect("balanced config")
    }

    #[test]
    fn compile_returns_dimension_matching_variant() {
        assert!(matches!(
            config(Dimensions::TwoD).compile(),
            AnyCompiledGeneration::TwoD(_)
        ));
        assert!(matches!(
            config(Dimensions::ThreeD).compile(),
            AnyCompiledGeneration::ThreeD(_)
        ));
    }

    #[test]
    fn typed_generations_preserve_plain_and_depth_geometry() {
        let AnyCompiledGeneration::TwoD(two_d) = config(Dimensions::TwoD).compile() else {
            panic!("expected 2D generation")
        };
        assert_eq!(two_d.segments().count(), 3);
        assert_eq!(
            two_d
                .depth_segments()
                .map(|segment| segment.topological_depth)
                .collect::<Vec<_>>(),
            [0, 1, 1]
        );

        let AnyCompiledGeneration::ThreeD(three_d) = config(Dimensions::ThreeD).compile() else {
            panic!("expected 3D generation")
        };
        assert_eq!(three_d.segments().count(), 3);
        assert_eq!(
            three_d
                .depth_segments()
                .map(|segment| segment.topological_depth)
                .collect::<Vec<_>>(),
            [0, 1, 1]
        );
    }

    #[test]
    fn typed_generations_count_their_paired_iteration_depth() {
        let config = |dimensions| {
            GenerationConfig::new(
                dimensions,
                "F".to_string(),
                4,
                90.0,
                1.0,
                0.0,
                [('F', "FF".to_string())].into(),
            )
            .expect("valid config")
        };

        let AnyCompiledGeneration::TwoD(two_d) = config(Dimensions::TwoD).compile() else {
            panic!("expected 2D generation")
        };
        assert_eq!(two_d.drawn_segment_count(), 16);
        assert_eq!(two_d.drawn_segment_count(), two_d.segments().count() as u64);

        let AnyCompiledGeneration::ThreeD(three_d) = config(Dimensions::ThreeD).compile() else {
            panic!("expected 3D generation")
        };
        assert_eq!(three_d.drawn_segment_count(), 16);
        assert_eq!(
            three_d.drawn_segment_count(),
            three_d.segments().count() as u64
        );
    }
}
