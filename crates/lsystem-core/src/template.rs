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
//! segment buffer. Consumers can transform templates on the CPU (see the
//! renderer bridge) or upload templates + stamps to the GPU; both the
//! compute-explosion and the two-level-instancing designs of issue #120 read
//! this same data.

use glam::{Quat, Vec2, Vec3};

use crate::compiled_generation::{CompiledGeneration2D, CompiledGeneration3D};
use crate::grammar::CompiledGrammar;
use crate::turtle::turtle2d::TurtleState2D;
use crate::turtle::turtle3d::TurtleState3D;

/// One template segment in the local frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TemplateSegment2D {
    pub start: Vec2,
    pub end: Vec2,
    /// Topological depth relative to the template entry.
    pub depth_offset: u32,
}

/// One template segment in the local frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TemplateSegment3D {
    pub start: Vec3,
    pub end: Vec3,
    /// Topological depth relative to the template entry.
    pub depth_offset: u32,
}

/// Precomputed geometry of one ruled symbol, plus the turtle-state delta from
/// template entry to exit that the placement walk composes per stamp.
#[derive(Clone, Debug, PartialEq)]
pub struct Template2D {
    pub segments: Vec<TemplateSegment2D>,
    pub exit_pos: Vec2,
    /// Net rotation entry→exit as a unit complex number (cos, sin).
    pub exit_rot: Vec2,
    pub exit_depth_delta: u32,
    /// Largest `depth_offset` among `segments`; 0 when there are none.
    pub max_depth_offset: u32,
    /// Local-frame bounding box; both zero when there are no segments.
    pub bounds_min: Vec2,
    pub bounds_max: Vec2,
}

/// Precomputed geometry of one ruled symbol, plus the turtle-state delta from
/// template entry to exit that the placement walk composes per stamp.
#[derive(Clone, Debug, PartialEq)]
pub struct Template3D {
    pub segments: Vec<TemplateSegment3D>,
    pub exit_pos: Vec3,
    /// Net rotation entry→exit.
    pub exit_rot: Quat,
    pub exit_depth_delta: u32,
    /// Largest `depth_offset` among `segments`; 0 when there are none.
    pub max_depth_offset: u32,
    /// Local-frame bounding box; both zero when there are no segments.
    pub bounds_min: Vec3,
    pub bounds_max: Vec3,
}

/// Templates for every ruled symbol of a compiled grammar, at a fixed count
/// of template iterations. Index 0 is the built-in single-`F` template used
/// to stamp bare unruled `F` symbols at the placement boundary.
///
/// The set owns the compiled grammar and the walk parameters it was built
/// from; [`TemplateSet2D::emit_stamps`] needs no further input.
pub struct TemplateSet2D {
    templates: Vec<Template2D>,
    symbol_to_template: [Option<u16>; 256],
    template_iterations: u16,
    generation: CompiledGeneration2D,
}

/// Templates for every ruled symbol of a compiled grammar, at a fixed count
/// of template iterations. Index 0 is the built-in single-`F` template used
/// to stamp bare unruled `F` symbols at the placement boundary.
///
/// The set owns the compiled grammar and the walk parameters it was built
/// from; [`TemplateSet3D::emit_stamps`] needs no further input.
pub struct TemplateSet3D {
    templates: Vec<Template3D>,
    symbol_to_template: [Option<u16>; 256],
    template_iterations: u16,
    generation: CompiledGeneration3D,
}

/// Placement of one template in world space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stamp2D {
    pub template: u16,
    pub pos: Vec2,
    /// World rotation as a unit complex number (cos, sin).
    pub rot: Vec2,
    pub depth_base: u32,
    /// Number of segments emitted before this stamp; also the stamp's offset
    /// into a flat traversal-ordered segment buffer. Saturates at
    /// `u32::MAX`; the stamp walk debug-asserts the total stays in range.
    pub order_base: u32,
}

/// Placement of one template in world space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stamp3D {
    pub template: u16,
    pub pos: Vec3,
    pub rot: Quat,
    pub depth_base: u32,
    /// Number of segments emitted before this stamp; also the stamp's offset
    /// into a flat traversal-ordered segment buffer. Saturates at
    /// `u32::MAX`; the stamp walk debug-asserts the total stays in range.
    pub order_base: u32,
}

/// Totals of a stamp walk, sufficient to size a flat output buffer and select
/// depth-gradient color parameters without transforming any geometry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StampStats {
    pub total_segments: u64,
    pub max_depth: u32,
}

/// Recommended template-segment budget for `build_within_budget` callers.
/// Bounds precomputed template memory (segments × ~20-32 bytes, ≈1-2 MiB),
/// not output size; large enough that typical systems get a deep template.
pub const DEFAULT_TEMPLATE_SEGMENT_BUDGET: u64 = 65_536;

/// Returns the largest template iteration count in `1..=iterations` whose
/// total template segment count stays within `max_template_segments`, or 0
/// when even one iteration exceeds the budget (callers then fall back to the
/// interpreter path). The total counts every segment a built set stores,
/// including the built-in one-segment unit-`F` template.
pub(crate) fn choose_template_iterations(
    grammar: &CompiledGrammar,
    iterations: u16,
    max_template_segments: u64,
) -> u16 {
    let ruled: Vec<u8> = grammar.ruled_symbols().collect();

    // yields[b] = number of drawn segments symbol `b` produces when expanded
    // `m` times.
    let mut yields = [0u64; 256];
    yields[b'F' as usize] = 1;
    let mut best = 0;
    for m in 1..=iterations {
        yields = grammar.advance_drawn_segment_yields(&yields);
        // Start at 1 for the built-in unit-F template every set stores.
        let total = ruled
            .iter()
            .map(|&symbol| yields[symbol as usize])
            .fold(1u64, |a, x| a.saturating_add(x));
        if total <= max_template_segments {
            best = m;
        }
    }
    best
}

/// Implements `build` and `emit_stamps` for one template-set type. The 2D
/// and 3D pipelines are the same walk over different geometry types: the
/// turtle helpers (`heading`, `normalized_heading`, `compose_heading`) hide
/// the heading representation and `$rotate` names the rotation-apply method,
/// so the logic exists once and the two instantiations cannot drift apart.
macro_rules! impl_template_set {
    ($Set:ident, $Generation:ident, $Template:ident, $Segment:ident, $Stamp:ident,
     $Vec:ty, $Turtle:ty, $rot_identity:expr, $rotate:ident) => {
        impl $Set {
            /// Builds templates for every ruled symbol by expanding it
            /// `template_iterations` times through the turtle in the local
            /// frame. When `template_iterations` is outside
            /// `1..=generation`'s iteration count, returns the generation
            /// back as the error so the caller can reuse it for the interpreter
            /// fallback path.
            pub fn build(
                generation: $Generation,
                template_iterations: u16,
            ) -> Result<Self, $Generation> {
                if template_iterations == 0
                    || template_iterations > generation.inner.params.iterations
                {
                    return Err(generation);
                }

                let grammar = &generation.inner.grammar;
                let params = generation.inner.params;

                let unit_end = <$Vec>::X * params.step;
                let mut templates = vec![$Template {
                    segments: vec![$Segment {
                        start: <$Vec>::ZERO,
                        end: unit_end,
                        depth_offset: 0,
                    }],
                    exit_pos: unit_end,
                    exit_rot: $rot_identity,
                    exit_depth_delta: 1,
                    max_depth_offset: 0,
                    bounds_min: <$Vec>::ZERO.min(unit_end),
                    bounds_max: <$Vec>::ZERO.max(unit_end),
                }];
                let mut symbol_to_template = [None; 256];

                for symbol in grammar.ruled_symbols() {
                    let mut state = <$Turtle>::new(params.angle, params.step, 0.0);
                    let mut segments = Vec::new();
                    let mut max_depth_offset = 0;
                    let mut bounds_min = <$Vec>::INFINITY;
                    let mut bounds_max = <$Vec>::NEG_INFINITY;
                    grammar
                        .expand_rule_effects(symbol, template_iterations)
                        .for_each(|byte| {
                            if let Some(segment) = state.apply(byte) {
                                let [start, end] = segment.points;
                                bounds_min = bounds_min.min(start).min(end);
                                bounds_max = bounds_max.max(start).max(end);
                                max_depth_offset = max_depth_offset.max(segment.topological_depth);
                                segments.push($Segment {
                                    start,
                                    end,
                                    depth_offset: segment.topological_depth,
                                });
                            }
                        });
                    debug_assert!(state.stack.is_empty(), "balanced RHS leaves stack empty");
                    if segments.is_empty() {
                        bounds_min = <$Vec>::ZERO;
                        bounds_max = <$Vec>::ZERO;
                    }

                    symbol_to_template[symbol as usize] = Some(templates.len() as u16);
                    templates.push($Template {
                        segments,
                        exit_pos: state.position,
                        exit_rot: state.normalized_heading(),
                        exit_depth_delta: state.topological_depth,
                        max_depth_offset,
                        bounds_min,
                        bounds_max,
                    });
                }

                Ok(Self {
                    templates,
                    symbol_to_template,
                    template_iterations,
                    generation,
                })
            }

            /// Builds at the largest template depth whose total template
            /// segment count fits `max_template_segments`. Returns the generation
            /// back as the error when no depth fits, so the caller can
            /// reuse it for the interpreter fallback path.
            pub fn build_within_budget(
                generation: $Generation,
                max_template_segments: u64,
            ) -> Result<Self, $Generation> {
                let template_iterations = choose_template_iterations(
                    &generation.inner.grammar,
                    generation.inner.params.iterations,
                    max_template_segments,
                );
                Self::build(generation, template_iterations)
            }

            /// Built templates; index 0 is the built-in bare-`F` unit
            /// template.
            pub fn templates(&self) -> &[$Template] {
                &self.templates
            }

            /// The number of expansion levels each template precomputes.
            pub fn template_iterations(&self) -> u16 {
                self.template_iterations
            }

            /// Walks the boundary expansion (`iterations -
            /// template_iterations` levels, unfiltered so ruled symbols
            /// surface) and streams one stamp per template placement that
            /// contributes segments, in traversal order. Placements of
            /// geometry-free templates advance the cursor but emit nothing.
            pub fn emit_stamps(&self, mut sink: impl FnMut($Stamp, &$Template)) -> StampStats {
                let params = self.generation.inner.params;
                let grammar = &self.generation.inner.grammar;
                // The cursor is a plain turtle: effect symbols advance it via
                // the shared apply() transition, so walk semantics cannot
                // drift from the interpreter.
                let mut state = <$Turtle>::new(params.angle, params.step, params.initial_heading);
                let mut order: u32 = 0;
                let mut stats = StampStats {
                    total_segments: 0,
                    max_depth: 0,
                };
                let mut place = |stamp: $Stamp, template: &$Template| {
                    stats.max_depth = stats
                        .max_depth
                        .max(stamp.depth_base.saturating_add(template.max_depth_offset));
                    stats.total_segments += template.segments.len() as u64;
                    sink(stamp, template);
                };

                grammar
                    .expand(params.iterations - self.template_iterations)
                    .for_each(|byte| {
                        let template_index =
                            if let Some(index) = self.symbol_to_template[byte as usize] {
                                index
                            } else if byte == b'F' {
                                // Bare unruled F: stamp the built-in unit
                                // template. apply() emits the entry state (as
                                // a segment) and advances the cursor.
                                let rot = state.heading();
                                let segment = state.apply(byte).expect("F always yields a segment");
                                place(
                                    $Stamp {
                                        template: 0,
                                        pos: segment.points[0],
                                        rot,
                                        depth_base: segment.topological_depth,
                                        order_base: order,
                                    },
                                    &self.templates[0],
                                );
                                order = order.saturating_add(1);
                                return;
                            } else {
                                state.apply(byte);
                                return;
                            };

                        let template = &self.templates[template_index as usize];
                        let rot = state.heading();
                        if !template.segments.is_empty() {
                            place(
                                $Stamp {
                                    template: template_index,
                                    pos: state.position,
                                    rot,
                                    depth_base: state.topological_depth,
                                    order_base: order,
                                },
                                template,
                            );
                        }
                        state.position += rot.$rotate(template.exit_pos);
                        state.compose_heading(template.exit_rot);
                        state.topological_depth = state
                            .topological_depth
                            .saturating_add(template.exit_depth_delta);
                        order = order.saturating_add(template.segments.len() as u32);
                    });

                debug_assert!(
                    u32::try_from(stats.total_segments).is_ok(),
                    "stamp order_base saturated: {} segments exceed u32::MAX",
                    stats.total_segments
                );
                stats
            }
        }
    };
}

impl_template_set!(
    TemplateSet2D,
    CompiledGeneration2D,
    Template2D,
    TemplateSegment2D,
    Stamp2D,
    Vec2,
    TurtleState2D,
    Vec2::X,
    rotate
);

impl_template_set!(
    TemplateSet3D,
    CompiledGeneration3D,
    Template3D,
    TemplateSegment3D,
    Stamp3D,
    Vec3,
    TurtleState3D,
    Quat::IDENTITY,
    mul_vec3
);

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
    ) -> Result<TemplateSet2D, crate::CompiledGeneration2D> {
        TemplateSet2D::build(compile_2d(config), template_iterations)
    }

    fn build_3d(
        config: &GenerationConfig,
        template_iterations: u16,
    ) -> Result<TemplateSet3D, crate::CompiledGeneration3D> {
        TemplateSet3D::build(compile_3d(config), template_iterations)
    }

    fn stamped_segments_2d(
        config: &GenerationConfig,
        template_iterations: u16,
    ) -> (Vec<Segment2DWithTopologicalDepth>, StampStats) {
        let set = build_2d(config, template_iterations).expect("set builds");
        let mut segments = Vec::new();
        let stats = set.emit_stamps(|stamp, template| {
            assert_eq!(
                stamp.order_base as usize,
                segments.len(),
                "order_base must equal segments emitted so far"
            );
            for segment in &template.segments {
                segments.push(Segment2DWithTopologicalDepth {
                    points: [
                        stamp.pos + stamp.rot.rotate(segment.start),
                        stamp.pos + stamp.rot.rotate(segment.end),
                    ],
                    topological_depth: stamp.depth_base.saturating_add(segment.depth_offset),
                });
            }
        });
        (segments, stats)
    }

    fn stamped_segments_3d(
        config: &GenerationConfig,
        template_iterations: u16,
    ) -> (Vec<Segment3DWithTopologicalDepth>, StampStats) {
        let set = build_3d(config, template_iterations).expect("set builds");
        let mut segments = Vec::new();
        let stats = set.emit_stamps(|stamp, template| {
            assert_eq!(
                stamp.order_base as usize,
                segments.len(),
                "order_base must equal segments emitted so far"
            );
            for segment in &template.segments {
                segments.push(Segment3DWithTopologicalDepth {
                    points: [
                        stamp.pos + stamp.rot * segment.start,
                        stamp.pos + stamp.rot * segment.end,
                    ],
                    topological_depth: stamp.depth_base.saturating_add(segment.depth_offset),
                });
            }
        });
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
                let generation = compile_2d(&config);
                let counted = generation.drawn_segment_count();
                assert_eq!(
                    counted,
                    generation.segments().count() as u64,
                    "2D count at iteration {iterations}"
                );
                if iterations > 0 {
                    let set = TemplateSet2D::build(generation, 1).expect("template set builds");
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
            let generation = compile_3d(&config);
            let counted = generation.drawn_segment_count();
            assert_eq!(
                counted,
                generation.segments().count() as u64,
                "3D count at iteration {iterations}"
            );
            if iterations > 0 {
                let set = TemplateSet3D::build(generation, 1).expect("template set builds");
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
    fn build_within_budget_picks_largest_fitting_depth() {
        // Koch's only ruled symbol is F with 4^m template segments, plus the
        // unit-F template: budget 20 fits m=2 (16 + 1), and budget 3 fits no
        // depth at all (m=1 needs 4 + 1).
        let config = koch();
        let set = TemplateSet2D::build_within_budget(compile_2d(&config), 20)
            .expect("budget 20 fits depth 2");
        assert_eq!(set.template_iterations(), 2);

        assert!(TemplateSet2D::build_within_budget(compile_2d(&config), 3).is_err());
    }

    #[test]
    fn choose_template_iterations_respects_budget() {
        // Koch's only ruled symbol is F with 4^m template segments.
        let config = koch();
        let grammar = CompiledGrammar::compile(&config);
        assert_eq!(
            choose_template_iterations(&grammar, config.iterations, 20),
            2
        );
        assert_eq!(
            choose_template_iterations(&grammar, config.iterations, 3),
            0
        );
        assert_eq!(
            choose_template_iterations(&grammar, config.iterations, u64::MAX),
            4
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
        let set = TemplateSet2D::build_within_budget(compile_2d(&config), 2)
            .expect("fixed-point template fits the budget");

        assert_eq!(set.template_iterations(), 31);
    }

    #[test]
    fn budget_counts_the_built_in_unit_template() {
        // Koch at m=2 stores 16 ruled + 1 unit segments: a budget of exactly
        // 16 no longer fits m=2, while 17 does.
        let config = koch();
        let grammar = CompiledGrammar::compile(&config);
        assert_eq!(
            choose_template_iterations(&grammar, config.iterations, 16),
            1
        );
        assert_eq!(
            choose_template_iterations(&grammar, config.iterations, 17),
            2
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
        let grammar = CompiledGrammar::compile(&config);
        assert_eq!(
            choose_template_iterations(&grammar, config.iterations, 0),
            0
        );
        assert_eq!(
            choose_template_iterations(&grammar, config.iterations, 1),
            2
        );
        assert!(TemplateSet2D::build_within_budget(compile_2d(&config), 0).is_err());
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
        let generation = match build_2d(&config, 0) {
            Ok(_) => panic!("depth zero must be invalid"),
            Err(generation) => generation,
        };
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
        let generation = match TemplateSet3D::build_within_budget(compile_3d(&config), 0) {
            Ok(_) => panic!("budget zero must not fit the unit template"),
            Err(generation) => generation,
        };
        assert!(generation.has_stack_directives());
        assert_eq!(generation.depth_segments().count(), 3);
    }
}
