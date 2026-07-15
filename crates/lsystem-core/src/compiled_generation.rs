use glam::{Vec2, Vec3};

use crate::{config::GenerationConfig, grammar::CompiledGrammar, turtle};

#[derive(Debug)]
pub enum CompiledGeneration {
    TwoD(CompiledGeneration2D),
    ThreeD(CompiledGeneration3D),
}

#[derive(Debug)]
/// A compiled 2D generation whose grammar and scalar parameters stay paired.
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
pub struct CompiledGeneration2D {
    pub(crate) inner: CompiledGenerationData,
}

#[derive(Debug)]
/// A compiled 3D generation whose grammar and scalar parameters stay paired.
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
pub struct CompiledGeneration3D {
    pub(crate) inner: CompiledGenerationData,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct GenerationParams {
    pub(crate) iterations: u16,
    pub(crate) angle: f32,
    pub(crate) step: f32,
    pub(crate) initial_heading: f32,
}

#[derive(Debug)]
pub(crate) struct CompiledGenerationData {
    pub(crate) grammar: CompiledGrammar,
    pub(crate) params: GenerationParams,
    has_stack_directives: bool,
}

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

impl GenerationConfig {
    pub fn compile(&self) -> CompiledGeneration {
        let inner = CompiledGenerationData {
            grammar: CompiledGrammar::compile(self),
            params: GenerationParams {
                iterations: self.iterations,
                angle: self.angle,
                step: self.step,
                initial_heading: self.initial_heading,
            },
            has_stack_directives: self.has_stack_directives(),
        };
        match self.dimensions {
            crate::Dimensions::TwoD => CompiledGeneration::TwoD(CompiledGeneration2D { inner }),
            crate::Dimensions::ThreeD => CompiledGeneration::ThreeD(CompiledGeneration3D { inner }),
        }
    }
}

impl CompiledGeneration2D {
    pub fn has_stack_directives(&self) -> bool {
        self.inner.has_stack_directives
    }
    /// Number of drawn segments in this generation without expanding it.
    ///
    /// The count is exact while representable and saturates at `u64::MAX`.
    pub fn drawn_segment_count(&self) -> u64 {
        self.inner
            .grammar
            .drawn_segment_count(self.inner.params.iterations)
    }
    pub fn segments(&self) -> impl Iterator<Item = [Vec2; 2]> + '_ {
        let p = self.inner.params;
        turtle::turtle2d::Segments2D::new(
            self.inner.grammar.expand_effects(p.iterations),
            p.angle,
            p.step,
            p.initial_heading,
        )
    }
    pub fn depth_segments(&self) -> impl Iterator<Item = Segment2DWithTopologicalDepth> + '_ {
        let p = self.inner.params;
        turtle::turtle2d::Segments2DWithTopologicalDepth::new(
            self.inner.grammar.expand_effects(p.iterations),
            p.angle,
            p.step,
            p.initial_heading,
        )
    }
}

impl CompiledGeneration3D {
    pub fn has_stack_directives(&self) -> bool {
        self.inner.has_stack_directives
    }
    /// Number of drawn segments in this generation without expanding it.
    ///
    /// The count is exact while representable and saturates at `u64::MAX`.
    pub fn drawn_segment_count(&self) -> u64 {
        self.inner
            .grammar
            .drawn_segment_count(self.inner.params.iterations)
    }
    pub fn segments(&self) -> impl Iterator<Item = [Vec3; 2]> + '_ {
        let p = self.inner.params;
        turtle::turtle3d::Segments3D::new(
            self.inner.grammar.expand_effects(p.iterations),
            p.angle,
            p.step,
            p.initial_heading,
        )
    }
    pub fn depth_segments(&self) -> impl Iterator<Item = Segment3DWithTopologicalDepth> + '_ {
        let p = self.inner.params;
        turtle::turtle3d::Segments3DWithTopologicalDepth::new(
            self.inner.grammar.expand_effects(p.iterations),
            p.angle,
            p.step,
            p.initial_heading,
        )
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
            CompiledGeneration::TwoD(_)
        ));
        assert!(matches!(
            config(Dimensions::ThreeD).compile(),
            CompiledGeneration::ThreeD(_)
        ));
    }

    #[test]
    fn typed_generations_preserve_plain_and_depth_geometry() {
        let CompiledGeneration::TwoD(two_d) = config(Dimensions::TwoD).compile() else {
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

        let CompiledGeneration::ThreeD(three_d) = config(Dimensions::ThreeD).compile() else {
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

        let CompiledGeneration::TwoD(two_d) = config(Dimensions::TwoD).compile() else {
            panic!("expected 2D generation")
        };
        assert_eq!(two_d.drawn_segment_count(), 16);
        assert_eq!(two_d.drawn_segment_count(), two_d.segments().count() as u64);

        let CompiledGeneration::ThreeD(three_d) = config(Dimensions::ThreeD).compile() else {
            panic!("expected 3D generation")
        };
        assert_eq!(three_d.drawn_segment_count(), 16);
        assert_eq!(
            three_d.drawn_segment_count(),
            three_d.segments().count() as u64
        );
    }
}
