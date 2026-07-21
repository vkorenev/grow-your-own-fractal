use std::fmt::{Debug, Display, Formatter};
use std::marker::PhantomData;

use crate::{
    D2, D3, Dimension,
    config::GenerationConfig,
    grammar::CompiledGrammar,
    template::{TemplateDimension, TemplateSet},
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
/// use lsystem_core::CompiledGeneration3D;
///
/// fn build_wrong_dimension(generation: CompiledGeneration3D) {
///     let set: lsystem_core::TemplateSet2D = generation
///         .build_templates(1)
///         .unwrap();
/// }
/// ```
pub type CompiledGeneration2D = CompiledGeneration<D2>;

/// A compiled 3D generation.
///
/// A 2D value cannot be passed to a 3D template set:
///
/// ```compile_fail
/// use lsystem_core::CompiledGeneration2D;
///
/// fn build_wrong_dimension(generation: CompiledGeneration2D) {
///     let set: lsystem_core::TemplateSet3D = generation
///         .build_templates(1)
///         .unwrap();
/// }
/// ```
pub type CompiledGeneration3D = CompiledGeneration<D3>;

/// An allocation-free generation plan that owns the compiled generation until
/// the caller is ready to prepare its selected strategy.
pub struct GenerationPlan<D: Dimension> {
    pub(crate) generation: CompiledGeneration<D>,
    total_segments: u64,
    selected_template_iterations: Option<u16>,
}

/// A generation prepared using the strategy selected by [`GenerationPlan`].
// Keep the public strategy payloads direct: preparation already owns either
// value, and adding a box would impose a second allocation on template users.
#[allow(clippy::large_enum_variant)]
pub enum PreparedGeneration<D: Dimension> {
    Stamped(TemplateSet<D>),
    Interpreted(CompiledGeneration<D>),
}

/// Failure to build a template set at an explicitly requested depth.
pub struct TemplateBuildError<D: Dimension> {
    generation: CompiledGeneration<D>,
    requested_iterations: u16,
    available_iterations: u16,
}

impl<D: Dimension> TemplateBuildError<D> {
    pub(crate) fn new(generation: CompiledGeneration<D>, requested_iterations: u16) -> Self {
        let available_iterations = generation.params.iterations;
        Self {
            generation,
            requested_iterations,
            available_iterations,
        }
    }

    /// The invalid template depth supplied by the caller.
    pub fn requested_iterations(&self) -> u16 {
        self.requested_iterations
    }

    /// The generation's maximum valid template depth.
    pub fn available_iterations(&self) -> u16 {
        self.available_iterations
    }

    /// Recovers the owned generation after a failed fixed-depth build.
    pub fn into_generation(self) -> CompiledGeneration<D> {
        self.generation
    }
}

impl<D: Dimension> Debug for TemplateBuildError<D> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TemplateBuildError")
            .field("requested_iterations", &self.requested_iterations)
            .field("available_iterations", &self.available_iterations)
            .finish_non_exhaustive()
    }
}

impl<D: Dimension> Display for TemplateBuildError<D> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "template iteration count {} is outside 1..={} for this generation",
            self.requested_iterations, self.available_iterations
        )
    }
}

impl<D: Dimension> std::error::Error for TemplateBuildError<D> {}

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
}

impl<D: TemplateDimension> CompiledGeneration<D> {
    /// Plans output counting and bounded template selection in one
    /// allocation-free recurrence pass.
    pub fn plan_templates(self, max_template_segments: u64) -> GenerationPlan<D> {
        let mut yields = [0u64; 256];
        yields[b'F' as usize] = 1;
        let mut selected_template_iterations = None;

        // Always run every round. Accepted grammars may shrink, remain fixed,
        // or oscillate, so a failed depth does not imply later depths fail.
        for template_iterations in 1..=self.params.iterations {
            yields = self.grammar.advance_drawn_segment_yields(&yields);
            if self.grammar.template_segment_count(&yields) <= max_template_segments {
                selected_template_iterations = Some(template_iterations);
            }
        }

        GenerationPlan {
            total_segments: self.grammar.axiom_drawn_segment_count(&yields),
            generation: self,
            selected_template_iterations,
        }
    }
}

impl<D: Dimension> GenerationPlan<D> {
    /// Exact output segment count while representable, saturating at
    /// `u64::MAX`.
    pub fn total_segments(&self) -> u64 {
        self.total_segments
    }

    pub fn has_stack_directives(&self) -> bool {
        self.generation.has_stack_directives
    }

    /// Selected stamped template depth, or `None` when no depth fits.
    pub fn selected_template_iterations(&self) -> Option<u16> {
        self.selected_template_iterations
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
        let two_d_total = two_d.plan_templates(u64::MAX).total_segments();
        assert_eq!(two_d_total, 16);
        assert_eq!(
            two_d_total,
            config(Dimensions::TwoD)
                .compile_for::<D2>()
                .segments()
                .count() as u64
        );

        let AnyCompiledGeneration::ThreeD(three_d) = config(Dimensions::ThreeD).compile() else {
            panic!("expected 3D generation")
        };
        let three_d_total = three_d.plan_templates(u64::MAX).total_segments();
        assert_eq!(three_d_total, 16);
        assert_eq!(
            three_d_total,
            config(Dimensions::ThreeD)
                .compile_for::<D3>()
                .segments()
                .count() as u64
        );
    }

    fn compile_2d_case(
        axiom: &str,
        iterations: u16,
        rules: BTreeMap<char, String>,
    ) -> CompiledGeneration2D {
        GenerationConfig::new(
            Dimensions::TwoD,
            axiom.to_string(),
            iterations,
            90.0,
            1.0,
            0.0,
            rules,
        )
        .expect("valid generation")
        .compile_for::<D2>()
    }

    #[test]
    fn planning_totals_cover_supported_grammar_shapes() {
        let cases = [
            (
                "growing",
                compile_2d_case("F", 8, [('F', "FF".to_string())].into()),
            ),
            (
                "shrinking",
                compile_2d_case("F", 8, [('F', "f".to_string())].into()),
            ),
            (
                "fixed",
                compile_2d_case("F", 8, [('F', "F".to_string())].into()),
            ),
            (
                "oscillating",
                compile_2d_case(
                    "F",
                    8,
                    [('F', "G".to_string()), ('G', "F".to_string())].into(),
                ),
            ),
            ("empty", compile_2d_case("", 8, BTreeMap::new())),
            (
                "non-ASCII",
                compile_2d_case("ä", 4, [('ä', "Fä".to_string())].into()),
            ),
            (
                "multi-rule",
                compile_2d_case(
                    "A",
                    3,
                    [('A', "FB".to_string()), ('B', "FF".to_string())].into(),
                ),
            ),
        ];

        for (name, generation) in cases {
            let expected = generation.segments().count() as u64;
            assert_eq!(
                generation.plan_templates(u64::MAX).total_segments(),
                expected,
                "{name}"
            );
        }
    }

    #[test]
    fn planning_runs_to_the_u16_rewrite_bound_without_expansion() {
        let shrinking = compile_2d_case("F", u16::MAX, [('F', "f".to_string())].into());
        assert_eq!(shrinking.plan_templates(0).total_segments(), 0);

        let fixed = compile_2d_case("F", u16::MAX, [('F', "F".to_string())].into());
        assert_eq!(fixed.plan_templates(2).total_segments(), 1);

        let oscillating = compile_2d_case(
            "F",
            u16::MAX,
            [('F', "G".to_string()), ('G', "F".to_string())].into(),
        );
        assert_eq!(oscillating.plan_templates(u64::MAX).total_segments(), 0);
    }

    #[test]
    fn planning_counts_saturating_growth() {
        let generation = compile_2d_case("F", 64, [('F', "FF".to_string())].into());
        assert_eq!(generation.plan_templates(1).total_segments(), u64::MAX);
    }
}
