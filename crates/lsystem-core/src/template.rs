//! Per-rule geometry templates and placement stamps.
//!
//! A template is the geometry a ruled symbol produces when expanded
//! `template_iterations` times, recorded in the local frame: entry position at
//! the origin, entry heading along +X (2D) or identity orientation (3D). A
//! stamp places one template in world space. Rendering the stamps therefore
//! equals rendering the full `iterations`-deep expansion, but the last
//! `template_iterations` levels are precomputed once per rule instead of being
//! re-interpreted symbol by symbol.
//!
//! Stamps stream in traversal order and `order_base` is the running segment
//! count, so it doubles as the output offset into a flat traversal-ordered
//! segment buffer. [`StampedSegments`] is the high-level CPU generation API;
//! consumers targeting template-aware GPU designs can instead read the
//! templates and stamps directly. Both the compute-explosion and the
//! two-level-instancing designs of issue #120 use this same low-level data.

use crate::compiled_generation::{
    CompiledGeneration, GenerationDimension, GenerationPlan, PreparedGeneration, TemplateBuildError,
};
use crate::turtle::{Turtle, TurtleDimension};
use crate::{D2, D3, Dimension};

/// One template segment in the local frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TemplateSegment<D: Dimension> {
    pub start: D::Point,
    pub end: D::Point,
    /// Topological depth relative to the template entry.
    pub depth_offset: u32,
}

pub type TemplateSegment2D = TemplateSegment<D2>;
pub type TemplateSegment3D = TemplateSegment<D3>;

/// Precomputed geometry of one ruled symbol, plus the turtle-state delta from
/// template entry to exit that the placement walk composes per stamp.
#[derive(Clone, Debug, PartialEq)]
pub struct Template<D: Dimension> {
    pub segments: Vec<TemplateSegment<D>>,
    pub exit_pos: D::Point,
    /// Net rotation from template entry to exit.
    pub exit_rot: D::Rotation,
    pub exit_depth_delta: u32,
    /// Largest `depth_offset` among `segments`; 0 when there are none.
    pub max_depth_offset: u32,
}

pub type Template2D = Template<D2>;
pub type Template3D = Template<D3>;

/// Templates for every ruled symbol of a compiled grammar, at a fixed count
/// of template iterations. Index 0 is the built-in single-`F` template used
/// to stamp bare unruled `F` symbols at the placement boundary.
///
/// The set owns the compiled grammar and the walk parameters it was built
/// from; [`TemplateSet::emit_stamps`] needs no further input.
pub struct TemplateSet<D: Dimension> {
    templates: Vec<Template<D>>,
    symbol_to_template: [Option<u16>; 256],
    template_iterations: u16,
    generation: CompiledGeneration<D>,
}

pub type TemplateSet2D = TemplateSet<D2>;
pub type TemplateSet3D = TemplateSet<D3>;

/// Placement of one template in world space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stamp<D: Dimension> {
    pub template: u16,
    pub pos: D::Point,
    pub rot: D::Rotation,
    pub depth_base: u32,
    /// Number of segments emitted before this stamp; also the stamp's offset
    /// into a flat traversal-ordered segment buffer.
    pub order_base: u64,
}

pub type Stamp2D = Stamp<D2>;
pub type Stamp3D = Stamp<D3>;

impl<D: Dimension> Stamp<D> {
    /// Transforms one template-local segment into world-space endpoints.
    pub fn transform_segment(&self, segment: &TemplateSegment<D>) -> [D::Point; 2] {
        D::transform_points(self.pos, self.rot, [segment.start, segment.end])
    }

    /// Transforms one template-local segment and applies this placement's
    /// topological-depth base.
    pub fn transform_depth_segment(
        &self,
        segment: &TemplateSegment<D>,
    ) -> crate::SegmentWithTopologicalDepth<D> {
        crate::SegmentWithTopologicalDepth {
            points: self.transform_segment(segment),
            topological_depth: self.depth_base.saturating_add(segment.depth_offset),
        }
    }
}

/// Totals of a stamp walk, sufficient to size a flat output buffer and select
/// depth-gradient color parameters without transforming any geometry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StampStats {
    pub total_segments: u64,
    pub max_depth: u32,
}

/// A collected placement walk over a [`TemplateSet`].
///
/// Collection retains only the placement list. Geometry remains
/// streaming: [`Self::segments`] and [`Self::depth_segments`] transform the
/// referenced templates into world space in traversal order without building
/// an intermediate segment list. The iterators are repeatable, which lets a
/// consumer inspect metadata before choosing a record layout or output sink.
pub struct StampedSegments<'a, D: Dimension> {
    set: &'a TemplateSet<D>,
    stamps: Vec<Stamp<D>>,
    stats: StampStats,
}

impl<D: Dimension> StampedSegments<'_, D> {
    pub fn total_segments(&self) -> u64 {
        self.stats.total_segments
    }

    pub fn max_topological_depth(&self) -> u32 {
        self.stats.max_depth
    }

    /// World-space segment endpoints in traversal order.
    pub fn segments(&self) -> impl Iterator<Item = [D::Point; 2]> + '_ {
        let templates = self.set.templates();
        self.stamps.iter().flat_map(move |stamp| {
            templates[stamp.template as usize]
                .segments
                .iter()
                .map(move |segment| stamp.transform_segment(segment))
        })
    }

    /// World-space segments with topological depth, in traversal order.
    pub fn depth_segments(
        &self,
    ) -> impl Iterator<Item = crate::SegmentWithTopologicalDepth<D>> + '_ {
        let templates = self.set.templates();
        self.stamps.iter().flat_map(move |stamp| {
            templates[stamp.template as usize]
                .segments
                .iter()
                .map(move |segment| stamp.transform_depth_segment(segment))
        })
    }
}

/// Recommended template-segment budget for generation planning callers.
/// Bounds precomputed template memory (segments × ~20-32 bytes, ≈1-2 MiB),
/// not output size; large enough that typical systems get a deep template.
pub const DEFAULT_TEMPLATE_SEGMENT_BUDGET: u64 = 65_536;

impl<D: Dimension> TemplateSet<D> {
    /// Built templates; index 0 is the built-in bare-`F` unit template.
    pub fn templates(&self) -> &[Template<D>] {
        &self.templates
    }

    /// The number of expansion levels each template precomputes.
    pub fn template_iterations(&self) -> u16 {
        self.template_iterations
    }
}

impl<D: TemplateDimension> TemplateSet<D> {
    /// Walks the boundary expansion and streams placements that contribute
    /// geometry in traversal order.
    pub fn emit_stamps(&self, sink: impl FnMut(Stamp<D>, &Template<D>)) -> StampStats {
        D::emit_stamps(self, sink)
    }

    /// Streams world-space segment endpoints in traversal order without
    /// retaining stamps or materialized segments.
    pub fn emit_segments(&self, mut sink: impl FnMut([D::Point; 2])) -> StampStats {
        self.emit_stamps(|stamp, template| {
            for segment in &template.segments {
                sink(stamp.transform_segment(segment));
            }
        })
    }

    /// Streams world-space segments with topological depth in traversal order
    /// without retaining stamps or materialized segments.
    pub fn emit_depth_segments(
        &self,
        mut sink: impl FnMut(crate::SegmentWithTopologicalDepth<D>),
    ) -> StampStats {
        self.emit_stamps(|stamp, template| {
            for segment in &template.segments {
                sink(stamp.transform_depth_segment(segment));
            }
        })
    }

    /// Collects this set's placement walk and returns repeatable streaming
    /// iterators over the resulting world-space segments.
    pub fn stamped_segments(&self) -> StampedSegments<'_, D> {
        let mut stamps = Vec::new();
        let mut running = 0u64;
        let stats = self.emit_stamps(|stamp, template| {
            debug_assert_eq!(
                stamp.order_base, running,
                "stamps must stream in traversal order"
            );
            running += template.segments.len() as u64;
            stamps.push(stamp);
        });
        StampedSegments {
            set: self,
            stamps,
            stats,
        }
    }
}

/// Dimension-specific template construction and stamp emission used by
/// generic consumers.
///
/// Implemented only for [`D2`] and [`D3`] via a blanket impl. Code that is
/// generic over dimension normally uses the inherent high-level methods on
/// [`CompiledGeneration`] and [`TemplateSet`]; the associated functions are
/// implementation hooks for those dimension-generic APIs.
pub trait TemplateDimension: GenerationDimension {
    #[doc(hidden)]
    fn build_templates(
        generation: CompiledGeneration<Self>,
        template_iterations: u16,
    ) -> Result<TemplateSet<Self>, TemplateBuildError<Self>>;

    fn emit_stamps(
        set: &TemplateSet<Self>,
        sink: impl FnMut(Stamp<Self>, &Template<Self>),
    ) -> StampStats;
}

/// Builds templates for every ruled symbol by expanding it
/// `template_iterations` times through the turtle in the local frame. The
/// 2D and 3D pipelines are the same walk over different geometry types: the
/// turtle helpers (`heading`, `normalized_heading`, `compose_heading`) hide
/// the heading representation and `TurtleDimension` supplies the rotation
/// application, so the logic exists once and the two instantiations cannot
/// drift apart.
fn build_generic<D: TurtleDimension>(
    generation: CompiledGeneration<D>,
    template_iterations: u16,
) -> Result<TemplateSet<D>, TemplateBuildError<D>> {
    if template_iterations == 0 || template_iterations > generation.params.iterations {
        return Err(TemplateBuildError::new(generation, template_iterations));
    }

    let grammar = &generation.grammar;
    let params = generation.params;

    let unit_end = D::unit_step(params.step);
    let mut templates = vec![Template {
        segments: vec![TemplateSegment {
            start: D::POINT_ZERO,
            end: unit_end,
            depth_offset: 0,
        }],
        exit_pos: unit_end,
        exit_rot: D::ROT_IDENTITY,
        exit_depth_delta: 1,
        max_depth_offset: 0,
    }];
    let mut symbol_to_template = [None; 256];

    for symbol in grammar.ruled_symbols() {
        let mut state = <D::Turtle as Turtle>::new(params.angle, params.step, 0.0);
        let mut segments = Vec::new();
        let mut max_depth_offset = 0;
        grammar
            .expand_rule_effects(symbol, template_iterations)
            .for_each(|byte| {
                if let Some(segment) = state.apply(byte) {
                    let [start, end] = segment.points;
                    max_depth_offset = max_depth_offset.max(segment.topological_depth);
                    segments.push(TemplateSegment {
                        start,
                        end,
                        depth_offset: segment.topological_depth,
                    });
                }
            });
        debug_assert!(state.stack_is_empty(), "balanced RHS leaves stack empty");

        symbol_to_template[symbol as usize] = Some(templates.len() as u16);
        templates.push(Template {
            segments,
            exit_pos: state.position(),
            exit_rot: state.normalized_heading(),
            exit_depth_delta: state.topological_depth(),
            max_depth_offset,
        });
    }

    Ok(TemplateSet {
        templates,
        symbol_to_template,
        template_iterations,
        generation,
    })
}

/// Walks the boundary expansion (`iterations - template_iterations` levels,
/// unfiltered so ruled symbols surface) and streams one stamp per template
/// placement that contributes segments, in traversal order. Placements of
/// geometry-free templates advance the cursor but emit nothing.
fn emit_stamps_generic<D: TurtleDimension>(
    set: &TemplateSet<D>,
    mut sink: impl FnMut(Stamp<D>, &Template<D>),
) -> StampStats {
    let params = set.generation.params;
    let grammar = &set.generation.grammar;
    // The cursor is a plain turtle: effect symbols advance it via
    // the shared apply() transition, so walk semantics cannot
    // drift from the interpreter.
    let mut state = <D::Turtle as Turtle>::new(params.angle, params.step, params.initial_heading);
    let mut order: u64 = 0;
    let mut stats = StampStats {
        total_segments: 0,
        max_depth: 0,
    };
    let mut place = |stamp: Stamp<D>, template: &Template<D>| {
        stats.max_depth = stats
            .max_depth
            .max(stamp.depth_base.saturating_add(template.max_depth_offset));
        stats.total_segments += template.segments.len() as u64;
        sink(stamp, template);
    };

    grammar
        .expand(params.iterations - set.template_iterations)
        .for_each(|byte| {
            let template_index = if let Some(index) = set.symbol_to_template[byte as usize] {
                index
            } else if byte == b'F' {
                // Bare unruled F: stamp the built-in unit
                // template. apply() emits the entry state (as
                // a segment) and advances the cursor.
                let rot = state.heading();
                let segment = state.apply(byte).expect("F always yields a segment");
                place(
                    Stamp {
                        template: 0,
                        pos: segment.points[0],
                        rot,
                        depth_base: segment.topological_depth,
                        order_base: order,
                    },
                    &set.templates[0],
                );
                order += 1;
                return;
            } else {
                state.apply(byte);
                return;
            };

            let template = &set.templates[template_index as usize];
            let rot = state.heading();
            if !template.segments.is_empty() {
                place(
                    Stamp {
                        template: template_index,
                        pos: state.position(),
                        rot,
                        depth_base: state.topological_depth(),
                        order_base: order,
                    },
                    template,
                );
            }
            state.advance(D::rotate(rot, template.exit_pos));
            state.compose_heading(template.exit_rot);
            state.add_topological_depth(template.exit_depth_delta);
            order += template.segments.len() as u64;
        });
    stats
}

impl<D: TurtleDimension> TemplateDimension for D {
    fn build_templates(
        generation: CompiledGeneration<Self>,
        template_iterations: u16,
    ) -> Result<TemplateSet<Self>, TemplateBuildError<Self>> {
        build_generic(generation, template_iterations)
    }

    fn emit_stamps(
        set: &TemplateSet<Self>,
        sink: impl FnMut(Stamp<Self>, &Template<Self>),
    ) -> StampStats {
        emit_stamps_generic(set, sink)
    }
}

impl<D: TemplateDimension> CompiledGeneration<D> {
    /// Builds templates at an explicitly requested expansion depth.
    pub fn build_templates(
        self,
        template_iterations: u16,
    ) -> Result<TemplateSet<D>, TemplateBuildError<D>> {
        D::build_templates(self, template_iterations)
    }
}

impl<D: TemplateDimension> GenerationPlan<D> {
    /// Builds the selected template set, or returns the owned generation for
    /// interpreted output when no template depth fit the planning budget.
    pub fn prepare(self) -> PreparedGeneration<D> {
        match self.selected_template_iterations() {
            Some(template_iterations) => {
                let set = match self.generation.build_templates(template_iterations) {
                    Ok(set) => set,
                    Err(_) => unreachable!("a planned template depth must be valid"),
                };
                PreparedGeneration::Stamped(set)
            }
            None => PreparedGeneration::Interpreted(self.generation),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::test_util::{compile_2d, compile_3d};
    use crate::{
        Dimensions, GenerationConfig, Segment2DWithTopologicalDepth, Segment3DWithTopologicalDepth,
    };

    // Composed rigid transforms round differently from the per-symbol
    // recurrence; real placement bugs produce errors of order one step.
    const TOLERANCE: f32 = 1e-3;

    fn build_2d(
        config: &GenerationConfig,
        template_iterations: u16,
    ) -> Result<TemplateSet2D, crate::TemplateBuildError<D2>> {
        compile_2d(config).build_templates(template_iterations)
    }

    fn build_3d(
        config: &GenerationConfig,
        template_iterations: u16,
    ) -> Result<TemplateSet3D, crate::TemplateBuildError<D3>> {
        compile_3d(config).build_templates(template_iterations)
    }

    fn assert_template_dimension_dispatch<D: TemplateDimension>(generation: CompiledGeneration<D>) {
        let PreparedGeneration::Stamped(set) = generation.plan_templates(u64::MAX).prepare() else {
            panic!("template set builds")
        };
        assert_eq!(set.template_iterations(), 1);

        let segments = set.stamped_segments();
        assert_eq!(segments.total_segments(), 1);
        assert_eq!(
            segments.segments().count() as u64,
            segments.total_segments()
        );
        assert_eq!(
            segments.depth_segments().count() as u64,
            segments.total_segments()
        );
        assert_eq!(segments.max_topological_depth(), 0);
    }

    fn assert_stamped_segments_match_stats<D: TemplateDimension>(set: &TemplateSet<D>) {
        let segments = set.stamped_segments();

        assert!(segments.total_segments() > 0);
        assert_eq!(
            segments.segments().count() as u64,
            segments.total_segments()
        );
        assert_eq!(
            segments.depth_segments().count() as u64,
            segments.total_segments()
        );
        let per_segment_max = segments
            .depth_segments()
            .map(|segment| segment.topological_depth)
            .max()
            .unwrap_or(0);
        assert_eq!(segments.max_topological_depth(), per_segment_max);
    }

    fn stamped_segments_2d(
        config: &GenerationConfig,
        template_iterations: u16,
    ) -> (Vec<Segment2DWithTopologicalDepth>, StampStats) {
        let set = build_2d(config, template_iterations).expect("set builds");
        let mut segments = Vec::new();
        let stats = set.emit_depth_segments(|segment| segments.push(segment));
        (segments, stats)
    }

    fn stamped_segments_3d(
        config: &GenerationConfig,
        template_iterations: u16,
    ) -> (Vec<Segment3DWithTopologicalDepth>, StampStats) {
        let set = build_3d(config, template_iterations).expect("set builds");
        let mut segments = Vec::new();
        let stats = set.emit_depth_segments(|segment| segments.push(segment));
        (segments, stats)
    }

    fn assert_matches_interpreter_2d(config: &GenerationConfig, template_iterations: u16) {
        let interpreted: Vec<_> = compile_2d(config).depth_segments().collect();
        let (stamped, stats) = stamped_segments_2d(config, template_iterations);

        assert_eq!(stamped.len(), interpreted.len(), "segment count");
        assert_eq!(stats.total_segments, interpreted.len() as u64);
        let mut max_depth = 0;
        for (index, (s, i)) in stamped.iter().zip(&interpreted).enumerate() {
            assert_eq!(
                s.topological_depth, i.topological_depth,
                "depth at segment {index}"
            );
            max_depth = max_depth.max(i.topological_depth);
            for point in 0..2 {
                let d = s.points[point].distance(i.points[point]);
                assert!(d < TOLERANCE, "segment {index} point {point}: off by {d}");
            }
        }
        assert_eq!(stats.max_depth, max_depth);
    }

    fn assert_matches_interpreter_3d(config: &GenerationConfig, template_iterations: u16) {
        let interpreted: Vec<_> = compile_3d(config).depth_segments().collect();
        let (stamped, stats) = stamped_segments_3d(config, template_iterations);

        assert_eq!(stamped.len(), interpreted.len(), "segment count");
        assert_eq!(stats.total_segments, interpreted.len() as u64);
        let mut max_depth = 0;
        for (index, (s, i)) in stamped.iter().zip(&interpreted).enumerate() {
            assert_eq!(
                s.topological_depth, i.topological_depth,
                "depth at segment {index}"
            );
            max_depth = max_depth.max(i.topological_depth);
            for point in 0..2 {
                let d = s.points[point].distance(i.points[point]);
                assert!(d < TOLERANCE, "segment {index} point {point}: off by {d}");
            }
        }
        assert_eq!(stats.max_depth, max_depth);
    }

    #[test]
    fn template_dimension_dispatches_for_both_markers() {
        let config = GenerationConfig::new(
            Dimensions::TwoD,
            "F".to_string(),
            1,
            90.0,
            1.0,
            0.0,
            BTreeMap::new(),
        )
        .expect("balanced config");
        assert_template_dimension_dispatch::<D2>(compile_2d(&config));

        let config = GenerationConfig::new(
            Dimensions::ThreeD,
            "F".to_string(),
            1,
            90.0,
            1.0,
            0.0,
            BTreeMap::new(),
        )
        .expect("balanced config");
        assert_template_dimension_dispatch::<D3>(compile_3d(&config));
    }

    fn koch() -> GenerationConfig {
        GenerationConfig::new(
            Dimensions::TwoD,
            "F++F++F".to_string(),
            4,
            60.0,
            1.0,
            0.0,
            BTreeMap::from([('F', "F-F++F-F".to_string())]),
        )
        .expect("balanced config")
    }

    fn dragon() -> GenerationConfig {
        GenerationConfig::new(
            Dimensions::TwoD,
            "FX".to_string(),
            10,
            90.0,
            1.0,
            0.0,
            BTreeMap::from([('X', "X+YF+".to_string()), ('Y', "-FX-Y".to_string())]),
        )
        .expect("balanced config")
    }

    fn plant() -> GenerationConfig {
        GenerationConfig::new(
            Dimensions::TwoD,
            "X".to_string(),
            5,
            23.4,
            1.0,
            90.0,
            BTreeMap::from([
                ('X', "F+[[X]-X]-F[-FX]+X".to_string()),
                ('F', "FF".to_string()),
            ]),
        )
        .expect("balanced config")
    }

    fn hilbert_3d() -> GenerationConfig {
        GenerationConfig::new(
            Dimensions::ThreeD,
            "X".to_string(),
            3,
            90.0,
            1.0,
            0.0,
            BTreeMap::from([('X', r"^\XF^\XFX-F^//XFX&F+//XFX-F/X-/".to_string())]),
        )
        .expect("balanced config")
    }

    fn tree_roll_3d() -> GenerationConfig {
        GenerationConfig::new(
            Dimensions::ThreeD,
            "A".to_string(),
            5,
            40.0,
            1.0,
            90.0,
            BTreeMap::from([
                ('A', r"F[+/A]/[-/A]F[&/A]/[^/A]".to_string()),
                ('F', "FF".to_string()),
            ]),
        )
        .expect("balanced config")
    }

    #[test]
    fn koch_stamped_matches_interpreter() {
        for m in 1..=4 {
            assert_matches_interpreter_2d(&koch(), m);
        }
    }

    #[test]
    fn dragon_stamped_matches_interpreter() {
        // Dragon is the stress case: ruled non-drawing X/Y at the boundary
        // plus bare unruled F stamped via the built-in unit template.
        for m in 1..=3 {
            assert_matches_interpreter_2d(&dragon(), m);
        }
    }

    #[test]
    fn plant_stamped_matches_interpreter_with_depths() {
        for m in 1..=3 {
            assert_matches_interpreter_2d(&plant(), m);
        }
    }

    #[test]
    fn collected_2d_stamped_segments_match_stats() {
        let config = plant();
        let set = build_2d(&config, 2).expect("set builds");

        assert_stamped_segments_match_stats(&set);
    }

    #[test]
    fn uturn_inside_rule_matches_interpreter() {
        let config = GenerationConfig::new(
            Dimensions::TwoD,
            "A".to_string(),
            3,
            90.0,
            1.0,
            0.0,
            BTreeMap::from([('A', "F|AF+".to_string())]),
        )
        .expect("balanced config");
        assert_matches_interpreter_2d(&config, 2);
    }

    #[test]
    fn ruleless_config_uses_only_unit_template() {
        let config = GenerationConfig::new(
            Dimensions::TwoD,
            "F+F".to_string(),
            2,
            90.0,
            1.0,
            0.0,
            BTreeMap::new(),
        )
        .expect("balanced config");
        let set = build_2d(&config, 1).expect("set builds");
        assert_eq!(set.templates().len(), 1);
        assert_matches_interpreter_2d(&config, 1);
    }

    #[test]
    fn hilbert_3d_stamped_matches_interpreter() {
        for m in 1..=2 {
            assert_matches_interpreter_3d(&hilbert_3d(), m);
        }
    }

    #[test]
    fn tree_roll_3d_stamped_matches_interpreter_with_depths() {
        for m in 1..=3 {
            assert_matches_interpreter_3d(&tree_roll_3d(), m);
        }
    }

    #[test]
    fn collected_3d_stamped_segments_match_stats() {
        let config = tree_roll_3d();
        let set = build_3d(&config, 2).expect("set builds");

        assert_stamped_segments_match_stats(&set);
    }

    #[test]
    fn drawn_segment_counts_match_fixtures_across_iteration_depths() {
        for mut config in [koch(), dragon(), plant()] {
            let max_iterations = config.iterations;
            for iterations in 0..=max_iterations {
                config.iterations = iterations;
                let counted = compile_2d(&config)
                    .plan_templates(u64::MAX)
                    .total_segments();
                assert_eq!(
                    counted,
                    compile_2d(&config).segments().count() as u64,
                    "2D count at iteration {iterations}"
                );
                if iterations > 0 {
                    let set = compile_2d(&config)
                        .build_templates(1)
                        .expect("template set builds");
                    assert_eq!(
                        counted,
                        set.emit_stamps(|_, _| {}).total_segments,
                        "2D stamp stats at iteration {iterations}"
                    );
                }
            }
        }

        let mut config = hilbert_3d();
        let max_iterations = config.iterations;
        for iterations in 0..=max_iterations {
            config.iterations = iterations;
            let counted = compile_3d(&config)
                .plan_templates(u64::MAX)
                .total_segments();
            assert_eq!(
                counted,
                compile_3d(&config).segments().count() as u64,
                "3D count at iteration {iterations}"
            );
            if iterations > 0 {
                let set = compile_3d(&config)
                    .build_templates(1)
                    .expect("template set builds");
                assert_eq!(
                    counted,
                    set.emit_stamps(|_, _| {}).total_segments,
                    "3D stamp stats at iteration {iterations}"
                );
            }
        }
    }

    #[test]
    fn template_depth_equal_to_iterations_leaves_axiom_walk() {
        // N = 0: the boundary walk is just the axiom.
        assert_matches_interpreter_2d(&koch(), 4);
        assert_matches_interpreter_3d(&hilbert_3d(), 3);
    }

    #[test]
    fn geometry_free_rule_advances_cursor_without_stamps() {
        // A's template draws nothing; its net rotation must still reach the
        // following F through the exit transform, and no empty stamp may be
        // emitted for it.
        let config = GenerationConfig::new(
            Dimensions::TwoD,
            "FAF".to_string(),
            3,
            30.0,
            1.0,
            0.0,
            BTreeMap::from([('A', "+A+".to_string())]),
        )
        .expect("balanced config");
        let set = build_2d(&config, 1).expect("set builds");
        set.emit_stamps(|_, template| {
            assert!(
                !template.segments.is_empty(),
                "geometry-free templates must not be stamped"
            );
        });
        assert_matches_interpreter_2d(&config, 1);
    }

    #[test]
    fn build_rejects_zero_and_excess_template_iterations() {
        assert!(build_2d(&koch(), 0).is_err());
        assert!(build_2d(&koch(), 5).is_err());
        assert!(build_3d(&hilbert_3d(), 0).is_err());
        assert!(build_3d(&hilbert_3d(), 4).is_err());
    }

    #[test]
    fn non_ascii_symbols_stamped_match_interpreter() {
        // Non-ASCII IDs are assigned by the set's own compilation, so they
        // stay stable between template building and the stamp walk.
        let config = GenerationConfig::new(
            Dimensions::TwoD,
            "Fä".to_string(),
            4,
            60.0,
            1.0,
            0.0,
            BTreeMap::from([('ä', "F+ä-F".to_string())]),
        )
        .expect("balanced config");
        for m in 1..=4 {
            assert_matches_interpreter_2d(&config, m);
        }
    }

    #[test]
    fn unreachable_rule_gets_no_template() {
        // Z never occurs in the axiom or a reachable RHS, so compilation
        // drops it and the set holds only the unit and F templates.
        let config = GenerationConfig::new(
            Dimensions::TwoD,
            "F".to_string(),
            2,
            90.0,
            1.0,
            0.0,
            BTreeMap::from([('F', "F-F".to_string()), ('Z', "FF".to_string())]),
        )
        .expect("balanced config");
        let set = build_2d(&config, 1).expect("set builds");
        assert_eq!(set.templates().len(), 2);
        assert_matches_interpreter_2d(&config, 1);
    }

    #[test]
    fn planning_picks_largest_fitting_depth() {
        // Koch's only ruled symbol is F with 4^m template segments, plus the
        // unit-F template: budget 20 fits m=2 (16 + 1), and budget 3 fits no
        // depth at all (m=1 needs 4 + 1).
        let config = koch();
        let plan = compile_2d(&config).plan_templates(20);
        assert_eq!(plan.selected_template_iterations(), Some(2));
        let PreparedGeneration::Stamped(set) = plan.prepare() else {
            panic!("budget 20 fits depth 2")
        };
        assert_eq!(set.template_iterations(), 2);

        let plan = compile_2d(&config).plan_templates(3);
        assert_eq!(plan.selected_template_iterations(), None);
        assert!(matches!(plan.prepare(), PreparedGeneration::Interpreted(_)));
    }

    #[test]
    fn planning_respects_budget() {
        // Koch's only ruled symbol is F with 4^m template segments.
        let config = koch();
        assert_eq!(
            compile_2d(&config)
                .plan_templates(20)
                .selected_template_iterations(),
            Some(2)
        );
        assert_eq!(
            compile_2d(&config)
                .plan_templates(3)
                .selected_template_iterations(),
            None
        );
        assert_eq!(
            compile_2d(&config)
                .plan_templates(u64::MAX)
                .selected_template_iterations(),
            Some(4)
        );
    }

    #[test]
    fn template_selection_can_exceed_interactive_hard_max() {
        let config = GenerationConfig::new(
            Dimensions::TwoD,
            "F".to_string(),
            31,
            90.0,
            1.0,
            0.0,
            BTreeMap::from([('F', "F".to_string())]),
        )
        .expect("balanced config");
        let PreparedGeneration::Stamped(set) = compile_2d(&config).plan_templates(2).prepare()
        else {
            panic!("fixed-point template fits the budget")
        };

        assert_eq!(set.template_iterations(), 31);
    }

    #[test]
    fn budget_counts_the_built_in_unit_template() {
        // Koch at m=2 stores 16 ruled + 1 unit segments: a budget of exactly
        // 16 no longer fits m=2, while 17 does.
        let config = koch();
        assert_eq!(
            compile_2d(&config)
                .plan_templates(16)
                .selected_template_iterations(),
            Some(1)
        );
        assert_eq!(
            compile_2d(&config)
                .plan_templates(17)
                .selected_template_iterations(),
            Some(2)
        );
        assert_eq!(
            compile_2d(&config)
                .plan_templates(18)
                .selected_template_iterations(),
            Some(2)
        );

        // A ruleless grammar still stores the one-segment unit template, so
        // budget 0 fits no depth and budget 1 fits every depth.
        let config = GenerationConfig::new(
            Dimensions::TwoD,
            "F".to_string(),
            2,
            90.0,
            1.0,
            0.0,
            BTreeMap::new(),
        )
        .expect("balanced config");
        assert_eq!(
            compile_2d(&config)
                .plan_templates(0)
                .selected_template_iterations(),
            None
        );
        assert_eq!(
            compile_2d(&config)
                .plan_templates(1)
                .selected_template_iterations(),
            Some(2)
        );
    }

    #[test]
    fn planning_keeps_searching_after_a_depth_exceeds_budget() {
        // At depth 1, A stores two segments and the unit template makes three.
        // At depth 2, both ruled templates are geometry-free, so only the unit
        // template remains and the later depth fits budget 1.
        let config = GenerationConfig::new(
            Dimensions::TwoD,
            "A".to_string(),
            2,
            90.0,
            1.0,
            0.0,
            BTreeMap::from([('A', "FF".to_string()), ('F', "f".to_string())]),
        )
        .expect("balanced config");

        let plan = compile_2d(&config).plan_templates(1);
        assert_eq!(plan.selected_template_iterations(), Some(2));
        assert_eq!(plan.total_segments(), 0);
    }

    #[test]
    fn failed_build_returns_generation_with_geometry_and_metadata() {
        let config = GenerationConfig::new(
            Dimensions::TwoD,
            "F[+F]F".to_string(),
            1,
            90.0,
            1.0,
            0.0,
            BTreeMap::new(),
        )
        .expect("balanced config");
        let error = match build_2d(&config, 0) {
            Ok(_) => panic!("depth zero must be invalid"),
            Err(error) => error,
        };
        assert_eq!(error.requested_iterations(), 0);
        assert_eq!(error.available_iterations(), 1);
        assert_eq!(
            error.to_string(),
            "template iteration count 0 is outside 1..=1 for this generation"
        );
        let generation = error.into_generation();
        assert!(generation.has_stack_directives());
        assert_eq!(generation.segments().count(), 3);

        let config = GenerationConfig::new(
            Dimensions::ThreeD,
            "F[+F]F".to_string(),
            1,
            90.0,
            1.0,
            0.0,
            BTreeMap::new(),
        )
        .expect("balanced config");
        let error = match compile_3d(&config).build_templates(2) {
            Ok(_) => panic!("depth above the generation must be invalid"),
            Err(error) => error,
        };
        assert_eq!(error.requested_iterations(), 2);
        assert_eq!(error.available_iterations(), 1);
        let generation = error.into_generation();
        assert!(generation.has_stack_directives());
        assert_eq!(generation.depth_segments().count(), 3);
    }
}
