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
//! segment buffer.
//!
//! [`PreparedGeneration::segments`] and [`PreparedGeneration::depth_segments`]
//! (defined in the `geometry_stream` module) own the placement walk, hide
//! whether geometry comes from template placements or direct interpretation,
//! and fold conservative bounds while yielding. Template sets continue to
//! expose their local geometry for inspection and explicit template-aware
//! designs. The public marker trait retains a doc-hidden placement hook for
//! generic implementation dispatch.

use std::sync::LazyLock;

use crate::bounds::{NUMERICAL_MARGIN_ULPS, generate_directions, round_down_f32, round_up_f32};
use crate::compiled_generation::{
    CompiledGeneration, GenerationDimension, GenerationPlan, PreparedGeneration, TemplateBuildError,
};
use crate::grammar::ExpandIter;
use crate::turtle::{Turtle, TurtleDimension};
use crate::{BoundsAccumulator, BoundsAccumulator2D, D2, D3, Dimension};
use glam::{Vec2, Vec3};

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
    // Conservative local-frame bounds summary. Small templates retain their
    // endpoints, which avoids adding candidates relative to direct endpoint
    // accumulation; larger templates store a dimension-specific conservative
    // shape (see [`Summary2D`] for 2D, local AABB corners for 3D).
    summary: D::Summary,
    pub exit_pos: D::Point,
    /// Net rotation from template entry to exit.
    pub exit_rot: D::Rotation,
    pub exit_depth_delta: u32,
    /// Largest `depth_offset` among `segments`; 0 when there are none.
    pub(crate) max_depth_offset: u32,
}

pub type Template2D = Template<D2>;
pub type Template3D = Template<D3>;

/// Builds and folds the local bounds summary stored with a template.
///
/// This stays with template construction rather than turtle semantics: it is
/// a storage tradeoff for stamped bounds, not a geometric transition.
trait TemplateSummaryDimension: Dimension {
    fn build_summary(segments: &[TemplateSegment<Self>]) -> Self::Summary;
    fn fold_summary(
        summary: &Self::Summary,
        stamp: Stamp<Self>,
        accumulator: &mut Self::BoundsAccumulator,
    );
}

/// Number of evenly spaced local directions sampled by a 2D template's
/// support polygon.
///
/// Divisible by four, so the four world-axis directions pulled back through
/// any stamp rotation land exactly a quarter of the table apart and one sector
/// search serves all four.
pub const SUPPORT_DIRECTION_COUNT_2D: usize = 32;

/// The `SUPPORT_DIRECTION_COUNT_2D` evenly spaced sampling directions, shared
/// by every 2D template.
static SUPPORT_DIRECTIONS_2D: LazyLock<[Vec2; SUPPORT_DIRECTION_COUNT_2D]> =
    LazyLock::new(generate_directions::<SUPPORT_DIRECTION_COUNT_2D>);

/// Conservative local-frame bounds summary stored with a 2D template.
///
/// Templates with fewer than four segments keep their (at most six) exact
/// endpoints, which is both tighter and cheaper than a 32-entry table. Larger
/// templates store a direction-sampled support polygon.
#[derive(Clone, Debug, PartialEq)]
pub enum Summary2D {
    /// Exact local endpoints of a template with fewer than four segments.
    Points(Vec<Vec2>),
    /// Sector vertices of the template's `SUPPORT_DIRECTION_COUNT_2D`
    /// direction support polygon.
    ///
    /// Let `h_i = max over local endpoints p of dot(p, d_i)` be the template's
    /// true local support along sampling direction `d_i`. Entry `i` is the
    /// intersection of the support lines of `d_i` and `d_{i + 1 mod M}`,
    /// rounded outward so that the **stored** value satisfies the containment
    /// invariant
    ///
    /// ```text
    /// dot(v_i, d_i) >= h_i   and   dot(v_i, d_{i + 1 mod M}) >= h_{i + 1 mod M}
    /// ```
    ///
    /// Every direction `u` inside sector `i` is a non-negative combination
    /// `u = a * d_i + b * d_{i+1}`, so for any local endpoint `p`
    /// `dot(p, u) = a * dot(p, d_i) + b * dot(p, d_{i+1}) <= a * h_i + b * h_{i+1}
    /// <= dot(v_i, u)`. Vertex `i` alone therefore bounds the template's
    /// support in *every* direction of sector `i`, with no search over the
    /// other vertices — unlike the free-form 3D active-set solve, the active
    /// constraint pair is fixed by construction here.
    ///
    /// Boxed because the array is an order of magnitude larger than the
    /// `Points` variant and only large templates need it.
    Table(Box<[Vec2; SUPPORT_DIRECTION_COUNT_2D]>),
}

/// True local support of a template along every sampling direction.
fn local_supports(
    segments: &[TemplateSegment<D2>],
    directions: &[Vec2; SUPPORT_DIRECTION_COUNT_2D],
) -> [f64; SUPPORT_DIRECTION_COUNT_2D] {
    let mut supports = [f64::NEG_INFINITY; SUPPORT_DIRECTION_COUNT_2D];
    for point in segments
        .iter()
        .flat_map(|segment| [segment.start, segment.end])
    {
        let (px, py) = (f64::from(point.x), f64::from(point.y));
        for (support, direction) in supports.iter_mut().zip(directions.iter().copied()) {
            *support = support.max(px * f64::from(direction.x) + py * f64::from(direction.y));
        }
    }
    supports
}

/// Solves the sector vertex bracketed by `first`/`second` and rounds the
/// stored `f32` outward until it satisfies the containment invariant
/// documented on [`Summary2D::Table`].
fn sector_vertex(first: Vec2, second: Vec2, first_support: f64, second_support: f64) -> Vec2 {
    let (ax, ay) = (f64::from(first.x), f64::from(first.y));
    let (bx, by) = (f64::from(second.x), f64::from(second.y));
    // Adjacent generated directions are 11.25 degrees apart, so they are never
    // coincident or anti-parallel and the determinant stays bounded away from
    // zero: no near-singular branch is needed here.
    let determinant = ax * by - ay * bx;
    let x = (first_support * by - ay * second_support) / determinant;
    let y = (ax * second_support - first_support * bx) / determinant;

    let magnitude = [first_support, second_support, x, y]
        .into_iter()
        .map(f64::abs)
        .fold(1.0_f64, f64::max);
    let margin = NUMERICAL_MARGIN_ULPS * f64::from(f32::EPSILON) * magnitude;

    // Push the f32-rounded vertex outward along the bisector `first + second`,
    // which has a strictly positive dot product with both bracketing
    // directions, so a single push moves both supports the same way. Rounding
    // the components independently (the way `finish_cylinder` rounds a scalar
    // radius) would instead *reduce* the support of a direction with a
    // negative component and silently break containment.
    let stored = Vec2::new(x as f32, y as f32);
    let (mx, my) = (ax + bx, ay + by);
    let (px, py) = (f64::from(stored.x), f64::from(stored.y));
    let push = ((first_support + margin - (px * ax + py * ay)) / (mx * ax + my * ay))
        .max((second_support + margin - (px * bx + py * by)) / (mx * bx + my * by))
        .max(0.0);
    let vertex = Vec2::new((px + push * mx) as f32, (py + push * my) as f32);

    debug_assert!(
        {
            let (vx, vy) = (f64::from(vertex.x), f64::from(vertex.y));
            vx * ax + vy * ay >= first_support + 0.5 * margin
                && vx * bx + vy * by >= second_support + 0.5 * margin
        },
        "stored sector vertex must clear both bracketing supports by most of the outward margin"
    );
    vertex
}

/// Index `i` of the sector whose bracketing directions `d_i` and
/// `d_{i + 1 mod M}` span `direction`.
///
/// Trigonometry-free: the component signs select the quadrant, and within one
/// quadrant the perp-dot sign against each generator is monotone, so counting
/// the generators `direction` has already passed locates the sector. A
/// finite-precision misclassification can only pick an adjacent sector, which
/// [`sector_support`] absorbs by evaluating both neighbours.
fn sector_index(directions: &[Vec2; SUPPORT_DIRECTION_COUNT_2D], direction: Vec2) -> usize {
    let quadrant = match (direction.x >= 0.0, direction.y >= 0.0) {
        (true, true) => 0,
        (false, true) => 1,
        (false, false) => 2,
        (true, false) => 3,
    };
    let quarter = SUPPORT_DIRECTION_COUNT_2D / 4;
    let base = quadrant * quarter;
    base + (1..quarter)
        .take_while(|&step| directions[base + step].perp_dot(direction) >= 0.0)
        .count()
}

/// Support of the stored polygon along `direction`, whose bracketing sector is
/// `sector` (modulo the table size).
///
/// Both neighbouring sectors are evaluated as well, so a finite-precision
/// misclassification at a sector boundary cannot under-bound.
fn sector_support(
    vertices: &[Vec2; SUPPORT_DIRECTION_COUNT_2D],
    sector: usize,
    direction: Vec2,
) -> f64 {
    let (dx, dy) = (f64::from(direction.x), f64::from(direction.y));
    [sector + SUPPORT_DIRECTION_COUNT_2D - 1, sector, sector + 1]
        .into_iter()
        .map(|index| vertices[index % SUPPORT_DIRECTION_COUNT_2D])
        .map(|vertex| f64::from(vertex.x) * dx + f64::from(vertex.y) * dy)
        .fold(f64::NEG_INFINITY, f64::max)
}

/// Folds a table-backed summary into the accumulator for one placement.
fn fold_support_table(
    vertices: &[Vec2; SUPPORT_DIRECTION_COUNT_2D],
    stamp: Stamp<D2>,
    accumulator: &mut BoundsAccumulator2D,
) {
    // World `+X` pulled back through the stamp rotation is the rotor
    // conjugate: `rotation.rotate(p).x == dot(p, (rot.x, -rot.y))`. The other
    // three world axes are quarter turns of it, so no per-stamp trigonometry
    // is needed. A rotor that is not exactly unit length stays correct: that
    // identity holds for any length, and only perp-dot *signs* drive the
    // sector search.
    let max_x_direction = Vec2::new(stamp.rot.x, -stamp.rot.y);
    let max_y_direction = max_x_direction.perp();
    let directions = LazyLock::force(&SUPPORT_DIRECTIONS_2D);
    let quarter = SUPPORT_DIRECTION_COUNT_2D / 4;
    // One search; the table is angle-sorted and its size is divisible by four,
    // so the other three axes sit exactly a quarter table apart.
    let sector = sector_index(directions, max_x_direction);
    let max_x = sector_support(vertices, sector, max_x_direction);
    let max_y = sector_support(vertices, sector + quarter, max_y_direction);
    let min_x = sector_support(vertices, sector + 2 * quarter, -max_x_direction);
    let min_y = sector_support(vertices, sector + 3 * quarter, -max_y_direction);

    let (x, y) = (f64::from(stamp.pos.x), f64::from(stamp.pos.y));
    let magnitude = [x, y, max_x, max_y, min_x, min_y]
        .into_iter()
        .map(f64::abs)
        .fold(1.0_f64, f64::max);
    // The world endpoints this bound must contain are themselves `f32`
    // roundings of `pos + rot.rotate(point)`, whose error scales with the
    // world position rather than with the template's local extent. Rounding
    // the fold outward by a world-scale margin keeps placements far from the
    // origin conservative.
    let margin = NUMERICAL_MARGIN_ULPS * f64::from(f32::EPSILON) * magnitude;
    // `BoundsAccumulator2D::include` packs `[x, y, -x, -y]` and takes a
    // component-wise max, so including exactly these two corners reproduces
    // the packed state of folding the four support values directly.
    accumulator.include(Vec2::new(
        round_up_f32(x + max_x + margin),
        round_up_f32(y + max_y + margin),
    ));
    accumulator.include(Vec2::new(
        round_down_f32(x - min_x - margin),
        round_down_f32(y - min_y - margin),
    ));
}

impl TemplateSummaryDimension for D2 {
    fn build_summary(segments: &[TemplateSegment<Self>]) -> Summary2D {
        // Three segments have six endpoints, which stays exact and remains
        // cheaper than a 32-entry support table.
        if segments.len() < 4 {
            return Summary2D::Points(
                segments
                    .iter()
                    .flat_map(|segment| [segment.start, segment.end])
                    .collect(),
            );
        }

        let directions = LazyLock::force(&SUPPORT_DIRECTIONS_2D);
        let supports = local_supports(segments, directions);
        Summary2D::Table(Box::new(std::array::from_fn(|index| {
            let next = (index + 1) % SUPPORT_DIRECTION_COUNT_2D;
            sector_vertex(
                directions[index],
                directions[next],
                supports[index],
                supports[next],
            )
        })))
    }

    fn fold_summary(
        summary: &Self::Summary,
        stamp: Stamp<Self>,
        accumulator: &mut Self::BoundsAccumulator,
    ) {
        match summary {
            Summary2D::Points(points) => {
                for candidate in points.iter().copied() {
                    accumulator.include(Self::transform_point(stamp.pos, stamp.rot, candidate));
                }
            }
            Summary2D::Table(vertices) => fold_support_table(vertices, stamp, accumulator),
        }
    }
}

impl TemplateSummaryDimension for D3 {
    fn build_summary(segments: &[TemplateSegment<Self>]) -> Vec<Vec3> {
        // Three segments have six endpoints, which remains tighter than the
        // eight AABB corners used by larger templates in three dimensions.
        if segments.len() < 4 {
            return segments
                .iter()
                .flat_map(|segment| [segment.start, segment.end])
                .collect();
        }

        let (min, max) = segments
            .iter()
            .flat_map(|segment| [segment.start, segment.end])
            .fold(
                (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
                |(min, max), point| (min.min(point), max.max(point)),
            );
        vec![
            Vec3::new(min.x, min.y, min.z),
            Vec3::new(min.x, min.y, max.z),
            Vec3::new(min.x, max.y, min.z),
            Vec3::new(min.x, max.y, max.z),
            Vec3::new(max.x, min.y, min.z),
            Vec3::new(max.x, min.y, max.z),
            Vec3::new(max.x, max.y, min.z),
            Vec3::new(max.x, max.y, max.z),
        ]
    }

    fn fold_summary(
        summary: &Self::Summary,
        stamp: Stamp<Self>,
        accumulator: &mut Self::BoundsAccumulator,
    ) {
        for candidate in summary.iter().copied() {
            accumulator.include(Self::transform_point(stamp.pos, stamp.rot, candidate));
        }
    }
}

impl<D: TemplateDimension> Template<D> {
    /// Includes this template's conservative world-space summary for one
    /// placement. Stamped consumers call this once before emitting the
    /// placement's individual records.
    pub(crate) fn include_world_bounds(
        &self,
        stamp: Stamp<D>,
        accumulator: &mut D::BoundsAccumulator,
    ) {
        D::fold_summary(&self.summary, stamp, accumulator);
    }
}

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
    /// Lazily yields geometry-producing template placements in traversal order.
    ///
    /// Each call starts a fresh boundary walk. Geometry-free templates are
    /// omitted, while their exit transforms still affect later placements.
    #[cfg(test)]
    pub(crate) fn stamps(&self) -> impl Iterator<Item = (Stamp<D>, &Template<D>)> + '_ {
        D::stamp_placements(self)
    }

    /// Lazily yields world-space segment endpoints in traversal order.
    ///
    /// Each call starts a fresh boundary walk. The returned iterator is
    /// resumable, but repeated calls repeat placement expansion.
    #[cfg(test)]
    pub(crate) fn segments(&self) -> impl Iterator<Item = [D::Point; 2]> + '_ {
        self.stamps().flat_map(|(stamp, template)| {
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
    #[cfg(test)]
    pub(crate) fn depth_segments(
        &self,
    ) -> impl Iterator<Item = crate::SegmentWithTopologicalDepth<D>> + '_ {
        self.stamps().flat_map(|(stamp, template)| {
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
    type StampPlacements<'a>: Iterator<Item = (Stamp<Self>, &'a Template<Self>)>
    where
        Self: 'a;

    #[doc(hidden)]
    fn stamp_placements(set: &TemplateSet<Self>) -> Self::StampPlacements<'_>;

    /// Forwards to [`TemplateSummaryDimension::fold_summary`] so callers
    /// generic over [`TemplateDimension`] (a public, sealed trait) can fold a
    /// template's summary without naming the private trait themselves.
    #[doc(hidden)]
    fn fold_summary(
        summary: &Self::Summary,
        stamp: Stamp<Self>,
        accumulator: &mut Self::BoundsAccumulator,
    );
}

/// Builds templates for every ruled symbol by expanding it
/// `template_iterations` times through the turtle in the local frame. The
/// 2D and 3D pipelines are the same walk over different geometry types: the
/// turtle helpers (`heading`, `normalized_heading`, `compose_heading`) hide
/// the heading representation and `TurtleDimension` supplies the rotation
/// application, so the logic exists once and the two instantiations cannot
/// drift apart.
fn build_generic<D: TurtleDimension + TemplateSummaryDimension>(
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
        summary: D::build_summary(&[TemplateSegment {
            start: D::POINT_ZERO,
            end: unit_end,
            depth_offset: 0,
        }]),
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
            summary: D::build_summary(&segments),
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
/// unfiltered so ruled symbols surface) and yields one stamp per template
/// placement that contributes segments, in traversal order. Placements of
/// geometry-free templates advance the cursor but yield nothing.
#[doc(hidden)]
pub struct StampPlacements<'a, D: TurtleDimension> {
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

impl<D: TurtleDimension + TemplateSummaryDimension> TemplateDimension for D {
    type StampPlacements<'a>
        = StampPlacements<'a, D>
    where
        D: 'a;

    fn build_templates(
        generation: CompiledGeneration<Self>,
        template_iterations: u16,
    ) -> Result<TemplateSet<Self>, TemplateBuildError<Self>> {
        build_generic(generation, template_iterations)
    }

    fn stamp_placements(set: &TemplateSet<Self>) -> Self::StampPlacements<'_> {
        StampPlacements::new(set)
    }

    fn fold_summary(
        summary: &Self::Summary,
        stamp: Stamp<Self>,
        accumulator: &mut Self::BoundsAccumulator,
    ) {
        <D as TemplateSummaryDimension>::fold_summary(summary, stamp, accumulator);
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

    use glam::{Quat, Vec2, Vec3, Vec3Swizzles};

    use super::*;
    use crate::test_util::{compile_2d, compile_3d};
    use crate::{Bounds2D, BoundsAccumulator, Dimensions, GenerationConfig};

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

        let (total_segments, max_depth) = stamp_stats(set);
        assert_eq!(total_segments, 1);
        assert_eq!(set.segments().count() as u64, total_segments);
        assert_eq!(set.depth_segments().count() as u64, total_segments);
        assert_eq!(max_depth, 0);
    }

    fn stamp_stats<D: TemplateDimension>(set: &TemplateSet<D>) -> (u64, u32) {
        set.stamps()
            .fold((0, 0), |(total, max), (stamp, template)| {
                (
                    total + template.segments.len() as u64,
                    max.max(stamp.depth_base.saturating_add(template.max_depth_offset)),
                )
            })
    }

    fn assert_lazy_segments_match_stats<D: TemplateDimension>(set: &TemplateSet<D>) {
        let (total_segments, max_depth) = stamp_stats(set);

        assert!(total_segments > 0);
        assert_eq!(set.segments().count() as u64, total_segments);
        assert_eq!(set.depth_segments().count() as u64, total_segments);
        let per_segment_max = set
            .depth_segments()
            .map(|segment| segment.topological_depth)
            .max()
            .unwrap_or(0);
        assert_eq!(max_depth, per_segment_max);
    }

    fn assert_matches_interpreter_2d(config: &GenerationConfig, template_iterations: u16) {
        let set = build_2d(config, template_iterations).expect("set builds");
        let plain_interpreted: Vec<_> = compile_2d(config).segments().collect();
        let plain_stamped: Vec<_> = set.segments().collect();
        let interpreted: Vec<_> = compile_2d(config).depth_segments().collect();
        let stamped: Vec<_> = set.depth_segments().collect();
        let (total_segments, expected_max_depth) = stamp_stats(&set);

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
        assert_eq!(total_segments, interpreted.len() as u64);
        let mut max_depth = 0;
        for (index, (s, i)) in stamped.iter().zip(&interpreted).enumerate() {
            assert_eq!(
                s.topological_depth, i.topological_depth,
                "depth at segment {index}"
            );
            max_depth = max_depth.max(s.topological_depth);
            for point in 0..2 {
                let d = s.points[point].distance(i.points[point]);
                assert!(d < TOLERANCE, "segment {index} point {point}: off by {d}");
            }
        }
        assert_eq!(expected_max_depth, max_depth);
    }

    fn assert_matches_interpreter_3d(config: &GenerationConfig, template_iterations: u16) {
        let set = build_3d(config, template_iterations).expect("set builds");
        let plain_interpreted: Vec<_> = compile_3d(config).segments().collect();
        let plain_stamped: Vec<_> = set.segments().collect();
        let interpreted: Vec<_> = compile_3d(config).depth_segments().collect();
        let stamped: Vec<_> = set.depth_segments().collect();
        let (total_segments, expected_max_depth) = stamp_stats(&set);

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
        assert_eq!(total_segments, interpreted.len() as u64);
        let mut max_depth = 0;
        for (index, (s, i)) in stamped.iter().zip(&interpreted).enumerate() {
            assert_eq!(
                s.topological_depth, i.topological_depth,
                "depth at segment {index}"
            );
            max_depth = max_depth.max(s.topological_depth);
            for point in 0..2 {
                let d = s.points[point].distance(i.points[point]);
                assert!(d < TOLERANCE, "segment {index} point {point}: off by {d}");
            }
        }
        assert_eq!(expected_max_depth, max_depth);
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
        assert_eq!(
            set.stamps().map(|(stamp, _)| stamp).collect::<Vec<_>>(),
            folded
        );
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
    fn lazy_2d_stamped_segments_match_stats() {
        let config = plant();
        let set = build_2d(&config, 2).expect("set builds");

        assert_lazy_segments_match_stats(&set);
    }

    #[test]
    fn template_summaries_use_2d_endpoints_for_small_templates_and_a_support_table_otherwise() {
        let config = GenerationConfig::new(
            Dimensions::TwoD,
            "A".to_string(),
            1,
            90.0,
            1.0,
            0.0,
            BTreeMap::from([('A', "F+F+F+F".to_string())]),
        )
        .expect("balanced config");
        let set = build_2d(&config, 1).expect("set builds");

        // The built-in one-segment unit template keeps its exact endpoints.
        let Summary2D::Points(points) = &set.templates()[0].summary else {
            panic!("a one-segment template keeps its endpoints")
        };
        assert_eq!(points.len(), 2);

        let template = &set.templates()[1];
        assert_eq!(template.segments.len(), 4);
        let Summary2D::Table(vertices) = &template.summary else {
            panic!("a four-segment template builds a support table")
        };
        assert_support_table_contains_endpoints(vertices, &template.segments);
    }

    /// Local-frame directions to probe a support table with. Deliberately
    /// misaligned with the 32 generators so most probes land strictly inside a
    /// sector rather than on a generator, where the table is exact anyway.
    fn probe_directions() -> impl Iterator<Item = Vec2> {
        (0..512).map(|index| Vec2::from_angle(std::f32::consts::TAU * index as f32 / 512.0 + 0.017))
    }

    /// Asserts the guarantee the per-stamp query relies on: for every probe
    /// direction, the bracketing sector's support (with neighbours, exactly as
    /// [`fold_support_table`] evaluates it) is at least the true support of
    /// every local endpoint.
    fn assert_support_table_contains_endpoints(
        vertices: &[Vec2; SUPPORT_DIRECTION_COUNT_2D],
        segments: &[TemplateSegment2D],
    ) {
        let directions = LazyLock::force(&SUPPORT_DIRECTIONS_2D);
        for probe in probe_directions() {
            let support = sector_support(vertices, sector_index(directions, probe), probe);
            for point in segments
                .iter()
                .flat_map(|segment| [segment.start, segment.end])
            {
                let exact = f64::from(point.x) * f64::from(probe.x)
                    + f64::from(point.y) * f64::from(probe.y);
                assert!(
                    support >= exact,
                    "support {support} under-bounds endpoint projection {exact} \
                     along {probe:?}"
                );
            }
        }
    }

    fn local_segments(points: &[[f32; 4]]) -> Vec<TemplateSegment2D> {
        points
            .iter()
            .map(|&[sx, sy, ex, ey]| TemplateSegment {
                start: Vec2::new(sx, sy),
                end: Vec2::new(ex, ey),
                depth_offset: 0,
            })
            .collect()
    }

    fn build_2d_summary(segments: &[TemplateSegment2D]) -> Summary2D {
        <D2 as TemplateSummaryDimension>::build_summary(segments)
    }

    /// Segment clouds chosen to stress the build: singleton and repeated
    /// points, collinear geometry (whose polygon is degenerate in one
    /// direction), a thin diagonal, and a generic scatter.
    fn adversarial_clouds() -> Vec<(&'static str, Vec<TemplateSegment2D>)> {
        vec![
            ("singleton", local_segments(&[[2.0, -3.0, 2.0, -3.0]])),
            ("repeated", local_segments(&[[1.0, 1.0, 1.0, 1.0]; 5])),
            (
                "collinear",
                local_segments(&[
                    [-4.0, -4.0, -1.0, -1.0],
                    [-1.0, -1.0, 2.0, 2.0],
                    [2.0, 2.0, 5.0, 5.0],
                    [5.0, 5.0, 8.0, 8.0],
                ]),
            ),
            (
                "thin-diagonal",
                local_segments(&[
                    [0.0, 0.0, 1.0, 1.0],
                    [1.0, 1.0, 2.0, 2.0],
                    [2.0, 2.0, 3.0, 3.001],
                    [3.0, 3.001, 4.0, 4.0],
                ]),
            ),
            (
                "scatter",
                local_segments(&[
                    [-9.0, 4.0, 11.0, -6.0],
                    [2.0, 8.0, -1.0, -4.0],
                    [6.0, 1.0, -3.0, 7.0],
                    [0.0, -11.0, 4.0, 3.0],
                    [-7.0, -2.0, 9.0, 5.0],
                ]),
            ),
        ]
    }

    #[test]
    fn support_tables_contain_their_own_local_endpoints() {
        for (label, segments) in adversarial_clouds() {
            match build_2d_summary(&segments) {
                // Fewer than four segments keeps the exact endpoints, so the
                // containment check is the identity.
                Summary2D::Points(points) => {
                    assert!(segments.len() < 4, "{label}: unexpected fallback");
                    let expected: Vec<_> = segments
                        .iter()
                        .flat_map(|segment| [segment.start, segment.end])
                        .collect();
                    assert_eq!(points, expected, "{label}");
                }
                Summary2D::Table(vertices) => {
                    assert!(segments.len() >= 4, "{label}: unexpected table");
                    assert_support_table_contains_endpoints(&vertices, &segments);
                }
            }
        }
    }

    #[test]
    fn support_tables_stay_conservative_across_scales_and_translation() {
        for (label, segments) in adversarial_clouds() {
            for scale in [1.0e-3_f32, 1.0, 1.0e3, 1.0e6] {
                for offset in [Vec2::ZERO, Vec2::new(1.0e3, -1.0e3)] {
                    let scaled: Vec<_> = segments
                        .iter()
                        .map(|segment| TemplateSegment {
                            start: segment.start * scale + offset,
                            end: segment.end * scale + offset,
                            depth_offset: segment.depth_offset,
                        })
                        .collect();
                    if let Summary2D::Table(vertices) = build_2d_summary(&scaled) {
                        assert_support_table_contains_endpoints(&vertices, &scaled);
                    } else {
                        assert!(scaled.len() < 4, "{label}: unexpected fallback");
                    }
                }
            }
        }
    }

    #[test]
    fn support_table_build_is_order_independent() {
        for (label, segments) in adversarial_clouds() {
            let forward = build_2d_summary(&segments);
            let mut reversed = segments.clone();
            reversed.reverse();
            let rotated: Vec<_> = segments[1..]
                .iter()
                .chain(&segments[..1])
                .copied()
                .collect();
            if segments.len() >= 4 {
                assert_eq!(forward, build_2d_summary(&reversed), "{label}: reversed");
                assert_eq!(forward, build_2d_summary(&rotated), "{label}: rotated");
            }
        }
    }

    #[test]
    fn generated_2d_support_directions_are_unit_length_and_antipodal() {
        let directions = LazyLock::force(&SUPPORT_DIRECTIONS_2D);
        let half = SUPPORT_DIRECTION_COUNT_2D / 2;
        for index in 0..half {
            let direction = directions[index];
            assert!((direction.length_squared() - 1.0).abs() <= 2.0e-7);
            assert_eq!(directions[index + half], -direction);
        }
        assert_eq!(directions[0], Vec2::X);
        // Angle-sorted: every step turns the same way by less than a half turn.
        for index in 0..SUPPORT_DIRECTION_COUNT_2D {
            let next = (index + 1) % SUPPORT_DIRECTION_COUNT_2D;
            assert!(directions[index].perp_dot(directions[next]) > 0.0);
        }
    }

    #[test]
    fn sector_lookup_brackets_every_probe_direction() {
        let directions = LazyLock::force(&SUPPORT_DIRECTIONS_2D);
        for probe in probe_directions() {
            let sector = sector_index(directions, probe);
            let next = (sector + 1) % SUPPORT_DIRECTION_COUNT_2D;
            assert!(
                directions[sector].perp_dot(probe) >= 0.0
                    && probe.perp_dot(directions[next]) >= 0.0,
                "{probe:?} is not bracketed by sector {sector}"
            );
        }
    }

    #[test]
    fn transformed_summary_bounds_contain_every_2d_template_endpoint() {
        let set = build_2d(&plant(), 2).expect("set builds");
        let template = set
            .templates()
            .iter()
            .find(|template| template.segments.len() >= 4)
            .expect("plant has a nontrivial template");
        let stamp = Stamp {
            template: 1,
            pos: Vec2::new(7.0, -3.0),
            rot: Vec2::from_angle(0.73),
            depth_base: 0,
            order_base: 0,
        };
        let mut accumulator = <D2 as Dimension>::BoundsAccumulator::default();
        template.include_world_bounds(stamp, &mut accumulator);
        let bounds = accumulator.finish().expect("template has geometry");

        for segment in &template.segments {
            for point in stamp.transform_segment(segment) {
                assert!(point.x >= bounds.min.x && point.x <= bounds.max.x);
                assert!(point.y >= bounds.min.y && point.y <= bounds.max.y);
            }
        }
    }

    /// Absolute outward slack of `candidate` relative to `exact`, summed over
    /// all four sides.
    ///
    /// Absolute rather than relative on purpose: a thin diagonal rotated to be
    /// nearly axis-aligned has a near-degenerate exact extent on one axis, so
    /// any relative or area measure explodes even for a correct fit.
    fn bounds_slack(candidate: Bounds2D, exact: Bounds2D) -> f64 {
        f64::from(candidate.max.x - exact.max.x)
            + f64::from(candidate.max.y - exact.max.y)
            + f64::from(exact.min.x - candidate.min.x)
            + f64::from(exact.min.y - candidate.min.y)
    }

    #[test]
    fn transformed_2d_support_table_bounds_beat_aabb_corners_on_a_thin_diagonal() {
        // Four collinear segments at 45 degrees: the local AABB is a square
        // whose two off-diagonal corners are pure phantoms.
        let config = GenerationConfig::new(
            Dimensions::TwoD,
            "A".to_string(),
            1,
            45.0,
            1.0,
            0.0,
            BTreeMap::from([('A', "+FFFF".to_string())]),
        )
        .expect("balanced config");
        let set = build_2d(&config, 1).expect("set builds");
        let template = &set.templates()[1];
        assert_eq!(template.segments.len(), 4);
        assert!(matches!(template.summary, Summary2D::Table(_)));

        // Rotate the local diagonal to just off world +Y; the 0.03 rad offset
        // only keeps the stamp off exact axis alignment and is not
        // load-bearing for the comparison below.
        let stamp = Stamp {
            template: 1,
            pos: Vec2::new(7.0, -3.0),
            rot: Vec2::from_angle(std::f32::consts::FRAC_PI_4 + 0.03),
            depth_base: 0,
            order_base: 0,
        };

        let mut exact = <D2 as Dimension>::BoundsAccumulator::default();
        for segment in &template.segments {
            for point in stamp.transform_segment(segment) {
                exact.include(point);
            }
        }
        let exact = exact.finish().expect("template has geometry");

        let mut table = <D2 as Dimension>::BoundsAccumulator::default();
        template.include_world_bounds(stamp, &mut table);
        let table = table.finish().expect("template has geometry");

        // The representation this stage replaces: the four local AABB corners,
        // transformed and accumulated. Inlined rather than resurrected.
        let (local_min, local_max) = template
            .segments
            .iter()
            .flat_map(|segment| [segment.start, segment.end])
            .fold(
                (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY)),
                |(min, max), point| (min.min(point), max.max(point)),
            );
        let mut aabb = <D2 as Dimension>::BoundsAccumulator::default();
        for corner in [
            Vec2::new(local_min.x, local_min.y),
            Vec2::new(local_min.x, local_max.y),
            Vec2::new(local_max.x, local_min.y),
            Vec2::new(local_max.x, local_max.y),
        ] {
            aabb.include(D2::transform_point(stamp.pos, stamp.rot, corner));
        }
        let aabb = aabb.finish().expect("template has geometry");

        for segment in &template.segments {
            for point in stamp.transform_segment(segment) {
                assert!(point.x >= table.min.x && point.x <= table.max.x);
                assert!(point.y >= table.min.y && point.y <= table.max.y);
            }
        }

        let table_slack = bounds_slack(table, exact);
        let aabb_slack = bounds_slack(aabb, exact);
        println!(
            "exact {exact:?}\ntable {table:?} slack {table_slack}\naabb {aabb:?} slack {aabb_slack}"
        );
        assert!(table_slack >= 0.0, "table bounds must contain exact bounds");
        // Measured on this fixture: table slack 7.1e-5 (dominated by the
        // outward numerical margin, not by sector sampling) against 3.88 for
        // the AABB corners, a factor of ~54_000. The threshold keeps ~50x
        // headroom so it tracks a real regression rather than platform noise.
        assert!(
            table_slack * 1000.0 <= aabb_slack,
            "support table slack {table_slack} must be far below AABB-corner slack {aabb_slack}"
        );
    }

    #[test]
    fn stamped_2d_placements_exercise_many_support_table_sectors() {
        // The support table only earns its keep for arbitrary pulled-back
        // directions; an axis-aligned stamp exercises none of the sector
        // lookup. 29.5 degrees is not a divisor of 90, so accumulated headings
        // sweep the circle.
        let config = GenerationConfig::new(
            Dimensions::TwoD,
            "A".to_string(),
            8,
            29.5,
            1.0,
            0.0,
            BTreeMap::from([('A', "F+[A]--AF+++A".to_string())]),
        )
        .expect("balanced config");
        let set = build_2d(&config, 2).expect("set builds");
        let directions = LazyLock::force(&SUPPORT_DIRECTIONS_2D);

        let mut sectors = std::collections::BTreeSet::new();
        let mut table_stamps = 0_u32;
        for (stamp, template) in set.stamps() {
            if matches!(template.summary, Summary2D::Table(_)) {
                table_stamps += 1;
                sectors.insert(sector_index(
                    directions,
                    Vec2::new(stamp.rot.x, -stamp.rot.y),
                ));
            }

            let mut candidate = <D2 as Dimension>::BoundsAccumulator::default();
            template.include_world_bounds(stamp, &mut candidate);
            let candidate = candidate.finish().expect("stamped template has geometry");
            for segment in &template.segments {
                for point in stamp.transform_segment(segment) {
                    assert!(
                        point.x >= candidate.min.x
                            && point.x <= candidate.max.x
                            && point.y >= candidate.min.y
                            && point.y <= candidate.max.y,
                        "stamp {stamp:?} bound {candidate:?} misses {point:?}"
                    );
                }
            }
        }

        assert!(
            table_stamps > 0,
            "fixture must stamp table-backed templates"
        );
        assert_eq!(
            sectors.len(),
            SUPPORT_DIRECTION_COUNT_2D,
            "table-backed stamps must reach every sector, saw {sectors:?}"
        );
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
    fn lazy_3d_stamped_segments_match_stats() {
        let config = tree_roll_3d();
        let set = build_3d(&config, 2).expect("set builds");

        assert_lazy_segments_match_stats(&set);
    }

    #[test]
    fn transformed_3d_aabb_summary_bounds_contain_every_template_endpoint() {
        let config = GenerationConfig::new(
            Dimensions::ThreeD,
            "A".to_string(),
            1,
            45.0,
            1.0,
            0.0,
            BTreeMap::from([('A', "+&FFFF".to_string())]),
        )
        .expect("balanced config");
        let set = build_3d(&config, 1).expect("set builds");
        let template = &set.templates()[1];
        assert_eq!(template.segments.len(), 4);
        assert_eq!(template.summary.len(), 8);

        let local_direction = (template.segments[0].end - template.segments[0].start).normalize();

        let stamp = Stamp {
            template: 1,
            pos: Vec3::new(7.0, -3.0, 2.0),
            // Map the thin local diagonal to world Y so exact endpoints have
            // negligible horizontal radius while AABB phantom corners do not.
            rot: Quat::from_rotation_arc(local_direction, Vec3::Y),
            depth_base: 0,
            order_base: 0,
        };
        let mut accumulator = <D3 as Dimension>::BoundsAccumulator::default();
        template.include_world_bounds(stamp, &mut accumulator);
        let bounds = accumulator.finish().expect("template has geometry");

        let mut exact = <D3 as Dimension>::BoundsAccumulator::default();

        for segment in &template.segments {
            for point in stamp.transform_segment(segment) {
                assert!(point.xz().distance(bounds.center_xz) <= bounds.radius);
                assert!(point.y >= bounds.min_y && point.y <= bounds.max_y);
                exact.include(point);
            }
        }
        let exact = exact.finish().expect("template has geometry");
        assert!(
            bounds.radius > exact.radius * 100.0,
            "thin diagonal exposes AABB slack"
        );
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
                        stamp_stats(&set).0,
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
                    stamp_stats(&set).0,
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
        set.stamps().for_each(|(_, template)| {
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
