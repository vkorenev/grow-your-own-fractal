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
//! [`PreparedGeneration::segments`] and
//! [`PreparedGeneration::depth_segments`] hide whether geometry comes from
//! template placements or direct interpretation while preserving lazy,
//! resumable iteration. Template sets continue to expose their local geometry
//! for inspection and explicit template-aware designs. The supported
//! consumer-facing placement convenience API is the prepared facade; the
//! public marker trait retains a doc-hidden placement hook for generic
//! implementation dispatch.

use crate::compiled_generation::{
    CompiledGeneration, GenerationDimension, GenerationPlan, PreparedGeneration, TemplateBuildError,
};
use crate::grammar::ExpandIter;
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
}

pub type Template2D = Template<D2>;
pub type Template3D = Template<D3>;

/// Templates for every ruled symbol of a compiled grammar, at a fixed count
/// of template iterations. Index 0 is the built-in single-`F` template used
/// to stamp bare unruled `F` symbols at the placement boundary.
///
/// The set owns the compiled grammar and the walk parameters it was built
/// from. Generic production consumers should obtain world-space geometry
/// through [`GenerationPlan::prepare`] and [`PreparedGeneration`].
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
    /// Lazily yields world-space segment endpoints in traversal order.
    ///
    /// Each call starts a fresh boundary walk. The returned iterator is
    /// resumable, but repeated calls repeat placement expansion.
    pub(crate) fn segments(&self) -> impl Iterator<Item = [D::Point; 2]> + '_ {
        D::stamp_placements(self).flat_map(|(stamp, template)| {
            template
                .segments
                .iter()
                .map(move |segment| stamp.transform_segment(segment))
        })
    }

    /// Lazily yields world-space segments with topological depth in traversal
    /// order.
    ///
    /// Each call starts a fresh boundary walk. The returned iterator is
    /// resumable, but repeated calls repeat placement expansion.
    pub(crate) fn depth_segments(
        &self,
    ) -> impl Iterator<Item = crate::SegmentWithTopologicalDepth<D>> + '_ {
        D::stamp_placements(self).flat_map(|(stamp, template)| {
            template
                .segments
                .iter()
                .map(move |segment| stamp.transform_depth_segment(segment))
        })
    }
}

/// Dimension-specific template construction and placement iteration.
///
/// Implemented only for [`D2`] and [`D3`] via a blanket impl. Code that is
/// generic over dimension normally uses the inherent high-level methods on
/// [`CompiledGeneration`] and [`PreparedGeneration`]; the associated functions
/// are implementation hooks for those dimension-generic APIs.
pub trait TemplateDimension: GenerationDimension {
    #[doc(hidden)]
    fn build_templates(
        generation: CompiledGeneration<Self>,
        template_iterations: u16,
    ) -> Result<TemplateSet<Self>, TemplateBuildError<Self>>;

    #[doc(hidden)]
    fn stamp_placements<'a>(
        set: &'a TemplateSet<Self>,
    ) -> impl Iterator<Item = (Stamp<Self>, &'a Template<Self>)> + 'a;
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
    }];
    let mut symbol_to_template = [None; 256];

    for symbol in grammar.ruled_symbols() {
        let mut state = <D::Turtle as Turtle>::new(params.angle, params.step, 0.0);
        let mut segments = Vec::new();
        grammar
            .expand_rule_effects(symbol, template_iterations)
            .for_each(|byte| {
                if let Some(segment) = state.apply(byte) {
                    let [start, end] = segment.points;
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
/// unfiltered so ruled symbols surface) and yields one stamp per template
/// placement that contributes segments, in traversal order. Placements of
/// geometry-free templates advance the cursor but yield nothing.
struct StampPlacements<'a, D: TurtleDimension> {
    set: &'a TemplateSet<D>,
    expansion: ExpandIter<'a>,
    state: D::Turtle,
    order: u64,
}

impl<'a, D: TurtleDimension> StampPlacements<'a, D> {
    fn new(set: &'a TemplateSet<D>) -> Self {
        let params = set.generation.params;
        Self {
            set,
            expansion: set
                .generation
                .grammar
                .expand(params.iterations - set.template_iterations),
            state: <D::Turtle as Turtle>::new(params.angle, params.step, params.initial_heading),
            order: 0,
        }
    }
}

fn apply_placement_symbol<'a, D: TurtleDimension>(
    set: &'a TemplateSet<D>,
    state: &mut D::Turtle,
    order: &mut u64,
    byte: u8,
) -> Option<(Stamp<D>, &'a Template<D>)> {
    let template_index = if let Some(index) = set.symbol_to_template[byte as usize] {
        index
    } else if byte == b'F' {
        // Bare unruled F: stamp the built-in unit template. apply() emits the
        // entry state (as a segment) and advances the cursor.
        let rot = state.heading();
        let segment = state.apply(byte).expect("F always yields a segment");
        let stamp = Stamp {
            template: 0,
            pos: segment.points[0],
            rot,
            depth_base: segment.topological_depth,
            order_base: *order,
        };
        *order += 1;
        return Some((stamp, &set.templates[0]));
    } else {
        state.apply(byte);
        return None;
    };

    let template = &set.templates[template_index as usize];
    let rot = state.heading();
    let placement = (!template.segments.is_empty()).then(|| {
        (
            Stamp {
                template: template_index,
                pos: state.position(),
                rot,
                depth_base: state.topological_depth(),
                order_base: *order,
            },
            template,
        )
    });
    state.advance(D::rotate(rot, template.exit_pos));
    state.compose_heading(template.exit_rot);
    state.add_topological_depth(template.exit_depth_delta);
    *order += template.segments.len() as u64;
    placement
}

impl<'a, D: TurtleDimension> Iterator for StampPlacements<'a, D> {
    type Item = (Stamp<D>, &'a Template<D>);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let byte = self.expansion.next()?;
            if let Some(placement) =
                apply_placement_symbol(self.set, &mut self.state, &mut self.order, byte)
            {
                return Some(placement);
            }
        }
    }

    fn fold<B, F>(self, init: B, mut f: F) -> B
    where
        Self: Sized,
        F: FnMut(B, Self::Item) -> B,
    {
        let set = self.set;
        let mut state = self.state;
        let mut order = self.order;
        self.expansion.fold(init, |acc, byte| {
            match apply_placement_symbol(set, &mut state, &mut order, byte) {
                Some(placement) => f(acc, placement),
                None => acc,
            }
        })
    }
}

impl<D: TurtleDimension> TemplateDimension for D {
    fn build_templates(
        generation: CompiledGeneration<Self>,
        template_iterations: u16,
    ) -> Result<TemplateSet<Self>, TemplateBuildError<Self>> {
        build_generic(generation, template_iterations)
    }

    fn stamp_placements<'a>(
        set: &'a TemplateSet<Self>,
    ) -> impl Iterator<Item = (Stamp<Self>, &'a Template<Self>)> + 'a {
        StampPlacements::new(set)
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

/// Private two-branch adapter used by [`PreparedGeneration`] to keep strategy
/// selection allocation-free while exposing one opaque iterator type.
enum PreparedIterator<S, I> {
    Stamped(S),
    Interpreted(I),
}

impl<T, S, I> Iterator for PreparedIterator<S, I>
where
    S: Iterator<Item = T>,
    I: Iterator<Item = T>,
{
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Stamped(iter) => iter.next(),
            Self::Interpreted(iter) => iter.next(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            Self::Stamped(iter) => iter.size_hint(),
            Self::Interpreted(iter) => iter.size_hint(),
        }
    }

    fn fold<B, F>(self, init: B, f: F) -> B
    where
        Self: Sized,
        F: FnMut(B, Self::Item) -> B,
    {
        match self {
            Self::Stamped(iter) => iter.fold(init, f),
            Self::Interpreted(iter) => iter.fold(init, f),
        }
    }
}

impl<D: TemplateDimension> PreparedGeneration<D> {
    /// Lazily yields world-space segment endpoints using the strategy selected
    /// by the generation plan.
    ///
    /// The returned iterator is allocation-free and resumable. Each call
    /// starts a fresh generation walk.
    pub fn segments(&self) -> impl Iterator<Item = [D::Point; 2]> + '_ {
        match self {
            Self::Stamped(set) => PreparedIterator::Stamped(set.segments()),
            Self::Interpreted(generation) => PreparedIterator::Interpreted(generation.segments()),
        }
    }

    /// Lazily yields world-space segments with topological depth using the
    /// strategy selected by the generation plan.
    ///
    /// The returned iterator is allocation-free and resumable. Each call
    /// starts a fresh generation walk.
    pub fn depth_segments(
        &self,
    ) -> impl Iterator<Item = crate::SegmentWithTopologicalDepth<D>> + '_ {
        match self {
            Self::Stamped(set) => PreparedIterator::Stamped(set.depth_segments()),
            Self::Interpreted(generation) => {
                PreparedIterator::Interpreted(generation.depth_segments())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::test_util::{FoldOnly, compile_2d, compile_3d};
    use crate::{Dimensions, GenerationConfig};

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
        let prepared = generation.plan_templates(u64::MAX).prepare();
        let PreparedGeneration::Stamped(set) = &prepared else {
            panic!("template set builds")
        };
        assert_eq!(set.template_iterations(), 1);
        assert_eq!(prepared.segments().count(), 1);
        let depths: Vec<_> = prepared.depth_segments().collect();
        assert_eq!(depths.len(), 1);
        assert_eq!(depths[0].topological_depth, 0);
    }

    fn assert_matches_interpreter_2d(config: &GenerationConfig, template_iterations: u16) {
        let stamped_generation =
            PreparedGeneration::Stamped(build_2d(config, template_iterations).expect("set builds"));
        let interpreted_generation = PreparedGeneration::Interpreted(compile_2d(config));
        let plain_interpreted: Vec<_> = interpreted_generation.segments().collect();
        let plain_stamped: Vec<_> = stamped_generation.segments().collect();
        let interpreted: Vec<_> = interpreted_generation.depth_segments().collect();
        let stamped: Vec<_> = stamped_generation.depth_segments().collect();

        assert_eq!(plain_stamped.len(), plain_interpreted.len(), "plain count");
        for (index, (stamped, interpreted)) in
            plain_stamped.iter().zip(&plain_interpreted).enumerate()
        {
            for point in 0..2 {
                let distance = stamped[point].distance(interpreted[point]);
                assert!(
                    distance < TOLERANCE,
                    "plain segment {index} point {point}: off by {distance}"
                );
            }
        }
        assert_eq!(stamped.len(), interpreted.len(), "segment count");
        for (index, (s, i)) in stamped.iter().zip(&interpreted).enumerate() {
            assert_eq!(
                s.topological_depth, i.topological_depth,
                "depth at segment {index}"
            );
            for point in 0..2 {
                let d = s.points[point].distance(i.points[point]);
                assert!(d < TOLERANCE, "segment {index} point {point}: off by {d}");
            }
        }
    }

    fn assert_matches_interpreter_3d(config: &GenerationConfig, template_iterations: u16) {
        let stamped_generation =
            PreparedGeneration::Stamped(build_3d(config, template_iterations).expect("set builds"));
        let interpreted_generation = PreparedGeneration::Interpreted(compile_3d(config));
        let plain_interpreted: Vec<_> = interpreted_generation.segments().collect();
        let plain_stamped: Vec<_> = stamped_generation.segments().collect();
        let interpreted: Vec<_> = interpreted_generation.depth_segments().collect();
        let stamped: Vec<_> = stamped_generation.depth_segments().collect();

        assert_eq!(plain_stamped.len(), plain_interpreted.len(), "plain count");
        for (index, (stamped, interpreted)) in
            plain_stamped.iter().zip(&plain_interpreted).enumerate()
        {
            for point in 0..2 {
                let distance = stamped[point].distance(interpreted[point]);
                assert!(
                    distance < TOLERANCE,
                    "plain segment {index} point {point}: off by {distance}"
                );
            }
        }
        assert_eq!(stamped.len(), interpreted.len(), "segment count");
        for (index, (s, i)) in stamped.iter().zip(&interpreted).enumerate() {
            assert_eq!(
                s.topological_depth, i.topological_depth,
                "depth at segment {index}"
            );
            for point in 0..2 {
                let d = s.points[point].distance(i.points[point]);
                assert!(d < TOLERANCE, "segment {index} point {point}: off by {d}");
            }
        }
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
    fn placement_next_and_fold_produce_the_same_output() {
        let config = plant();
        let set = build_2d(&config, 2).expect("set builds");
        let mut next_placements = StampPlacements::new(&set);
        let mut next = Vec::new();
        for (stamp, _) in &mut next_placements {
            next.push(stamp);
        }
        let folded = StampPlacements::new(&set).fold(Vec::new(), |mut stamps, (stamp, _)| {
            stamps.push(stamp);
            stamps
        });

        assert_eq!(next, folded);
    }

    #[test]
    fn placement_fold_after_partial_next_produces_the_exact_suffix() {
        let config = plant();
        let set = build_2d(&config, 2).expect("set builds");
        let all = StampPlacements::new(&set).fold(Vec::new(), |mut stamps, (stamp, _)| {
            stamps.push(stamp);
            stamps
        });
        assert!(all.len() > 2);

        let mut placements = StampPlacements::new(&set);
        assert_eq!(placements.next().map(|(stamp, _)| stamp), Some(all[0]));
        assert_eq!(placements.next().map(|(stamp, _)| stamp), Some(all[1]));
        let suffix = placements.fold(Vec::new(), |mut stamps, (stamp, _)| {
            stamps.push(stamp);
            stamps
        });

        assert_eq!(suffix, all[2..]);
    }

    #[test]
    fn prepared_iterator_delegates_size_hint_and_specialized_fold() {
        let stamped_hint =
            PreparedIterator::<_, std::iter::Empty<i32>>::Stamped([1, 2, 3].into_iter());
        assert_eq!(stamped_hint.size_hint(), (3, Some(3)));
        let stamped = PreparedIterator::<_, std::iter::Empty<i32>>::Stamped(FoldOnly::new(
            [1, 2, 3].into_iter(),
        ));
        let stamped_values = stamped.fold(Vec::new(), |mut values, value| {
            values.push(value);
            values
        });
        assert_eq!(stamped_values, [1, 2, 3]);

        let interpreted_hint =
            PreparedIterator::<std::iter::Empty<i32>, _>::Interpreted([4, 5].into_iter());
        assert_eq!(interpreted_hint.size_hint(), (2, Some(2)));
        let interpreted = PreparedIterator::<std::iter::Empty<i32>, _>::Interpreted(FoldOnly::new(
            [4, 5].into_iter(),
        ));
        let interpreted_values = interpreted.fold(Vec::new(), |mut values, value| {
            values.push(value);
            values
        });
        assert_eq!(interpreted_values, [4, 5]);
    }

    #[test]
    fn placement_order_is_exact_above_u32_max() {
        let config = GenerationConfig::new(
            Dimensions::TwoD,
            "FFF".to_string(),
            1,
            90.0,
            1.0,
            0.0,
            BTreeMap::new(),
        )
        .expect("balanced config");
        let set = build_2d(&config, 1).expect("set builds");
        let mut placements = StampPlacements::new(&set);
        placements.order = u64::from(u32::MAX);

        let orders: Vec<_> = placements.map(|(stamp, _)| stamp.order_base).collect();
        assert_eq!(
            orders,
            [
                u64::from(u32::MAX),
                u64::from(u32::MAX) + 1,
                u64::from(u32::MAX) + 2,
            ]
        );
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
                    let stamped = PreparedGeneration::Stamped(
                        compile_2d(&config)
                            .build_templates(1)
                            .expect("template set builds"),
                    );
                    assert_eq!(
                        counted,
                        stamped.segments().count() as u64,
                        "2D stamped count at iteration {iterations}"
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
                let stamped = PreparedGeneration::Stamped(
                    compile_3d(&config)
                        .build_templates(1)
                        .expect("template set builds"),
                );
                assert_eq!(
                    counted,
                    stamped.segments().count() as u64,
                    "3D stamped count at iteration {iterations}"
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
        D2::stamp_placements(&set).for_each(|(_, template)| {
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
    fn three_d_zero_budget_prepares_interpreted_generation() {
        let config = GenerationConfig::new(
            Dimensions::ThreeD,
            "F[&F]F".to_string(),
            1,
            90.0,
            1.0,
            0.0,
            BTreeMap::new(),
        )
        .expect("balanced config");

        let plan = compile_3d(&config).plan_templates(0);
        assert_eq!(plan.total_segments(), 3);
        assert_eq!(plan.selected_template_iterations(), None);
        let PreparedGeneration::Interpreted(generation) = plan.prepare() else {
            panic!("zero budget cannot fit the built-in unit template")
        };
        assert!(generation.has_stack_directives());
        assert_eq!(generation.depth_segments().count(), 3);
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
