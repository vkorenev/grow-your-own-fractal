//! Bounds-carrying world-space geometry streams.
//!
//! `PreparedGeneration::segments` and `depth_segments` are the production
//! geometry walks. They fold conservative bounds while yielding: once per
//! placement template summaries when stamped, and each endpoint when
//! interpreted. Consumers therefore do not need to know which generation
//! strategy produced the geometry.
//!
//! Hot paths consume these streams by value through `drain` or `fold`, which
//! preserves the placement walk's specialized internal iteration. Consumers
//! that need to suspend between segments may use `next` and then `finish`.

use crate::compiled_generation::PreparedGeneration;
use crate::template::{Stamp, TemplateDimension, TemplateSegment};
use crate::{BoundsAccumulator, SegmentWithTopologicalDepth};

/// Streaming world-space segments with fused conservative bounds.
pub struct SegmentStream<'a, D: TemplateDimension> {
    walk: SegmentWalk<'a, D>,
    bounds: D::BoundsAccumulator,
}

enum SegmentWalk<'a, D: TemplateDimension> {
    Stamped {
        placements: D::StampPlacements<'a>,
        current: Option<(Stamp<D>, std::slice::Iter<'a, TemplateSegment<D>>)>,
    },
    Interpreted(Box<dyn Iterator<Item = SegmentWithTopologicalDepth<D>> + Send + 'a>),
}

impl<D: TemplateDimension> PreparedGeneration<D> {
    /// Lazily yields world-space segment endpoints in traversal order while
    /// accumulating conservative scene bounds.
    pub fn segments(&self) -> SegmentStream<'_, D> {
        SegmentStream {
            walk: match self {
                Self::Stamped(set) => SegmentWalk::Stamped {
                    placements: D::stamp_placements(set),
                    current: None,
                },
                Self::Interpreted(generation) => {
                    SegmentWalk::Interpreted(Box::new(D::depth_segments(generation)))
                }
            },
            bounds: D::BoundsAccumulator::default(),
        }
    }

    /// Lazily yields world-space segments with topological depth in traversal
    /// order while accumulating conservative scene bounds and depth metadata.
    pub fn depth_segments(&self) -> DepthSegmentStream<'_, D> {
        DepthSegmentStream {
            walk: match self {
                Self::Stamped(set) => SegmentWalk::Stamped {
                    placements: D::stamp_placements(set),
                    current: None,
                },
                Self::Interpreted(generation) => {
                    SegmentWalk::Interpreted(Box::new(D::depth_segments(generation)))
                }
            },
            bounds: D::BoundsAccumulator::default(),
            max_topological_depth: 0,
        }
    }
}

impl<'a, D: TemplateDimension> SegmentStream<'a, D> {
    /// Consumes the remaining stream with internal iteration and returns the
    /// accumulated bounds, or `None` when nothing was ever yielded.
    pub fn drain(self, mut sink: impl FnMut([D::Point; 2])) -> Option<D::Bounds> {
        self.fold_with_bounds((), |(), segment| sink(segment)).1
    }

    /// Bounds of everything yielded so far. After exhaustion this is the full
    /// scene bound; while suspended it conservatively covers the yielded
    /// prefix.
    pub fn finish(self) -> Option<D::Bounds> {
        self.bounds.finish()
    }

    fn fold_with_bounds<B>(
        self,
        init: B,
        mut f: impl FnMut(B, [D::Point; 2]) -> B,
    ) -> (B, Option<D::Bounds>) {
        let Self { walk, mut bounds } = self;
        let accumulated = match walk {
            SegmentWalk::Stamped {
                placements,
                current,
            } => {
                let mut acc = init;
                if let Some((stamp, segments)) = current {
                    acc =
                        segments.fold(acc, |acc, segment| f(acc, stamp.transform_segment(segment)));
                }
                placements.fold(acc, |acc, (stamp, template)| {
                    template.include_world_bounds(stamp, &mut bounds);
                    template
                        .segments
                        .iter()
                        .fold(acc, |acc, segment| f(acc, stamp.transform_segment(segment)))
                })
            }
            SegmentWalk::Interpreted(segments) => segments.fold(init, |acc, segment| {
                let [start, end] = segment.points;
                bounds.include_segment(start, end);
                f(acc, [start, end])
            }),
        };
        (accumulated, bounds.finish())
    }
}

impl<'a, D: TemplateDimension> Iterator for SegmentStream<'a, D> {
    type Item = [D::Point; 2];

    fn next(&mut self) -> Option<Self::Item> {
        match &mut self.walk {
            SegmentWalk::Stamped {
                placements,
                current,
            } => loop {
                if let Some((stamp, segments)) = current
                    && let Some(segment) = segments.next()
                {
                    return Some(stamp.transform_segment(segment));
                }
                let (stamp, template) = placements.next()?;
                template.include_world_bounds(stamp, &mut self.bounds);
                *current = Some((stamp, template.segments.iter()));
            },
            SegmentWalk::Interpreted(segments) => {
                let segment = segments.next()?;
                let [start, end] = segment.points;
                self.bounds.include_segment(start, end);
                Some([start, end])
            }
        }
    }

    fn fold<B, F>(self, init: B, f: F) -> B
    where
        F: FnMut(B, Self::Item) -> B,
    {
        self.fold_with_bounds(init, f).0
    }
}

/// Final bounds and topological-depth metadata for one depth stream.
#[derive(Clone, Copy, Debug)]
pub struct DepthStreamSummary<D: crate::Dimension> {
    /// Conservative scene bounds, or `None` when nothing was yielded.
    pub bounds: Option<D::Bounds>,
    /// Largest topological depth among yielded records, or zero when empty.
    pub max_topological_depth: u32,
}

/// Streaming depth segments with fused conservative bounds and depth metadata.
pub struct DepthSegmentStream<'a, D: TemplateDimension> {
    walk: SegmentWalk<'a, D>,
    bounds: D::BoundsAccumulator,
    max_topological_depth: u32,
}

impl<'a, D: TemplateDimension> DepthSegmentStream<'a, D> {
    /// Consumes the remaining stream with internal iteration.
    pub fn drain(
        self,
        mut sink: impl FnMut(SegmentWithTopologicalDepth<D>),
    ) -> DepthStreamSummary<D> {
        self.fold_with_summary((), |(), segment| sink(segment)).1
    }

    /// Summary of everything yielded so far; complete after exhaustion.
    pub fn finish(self) -> DepthStreamSummary<D> {
        DepthStreamSummary {
            bounds: self.bounds.finish(),
            max_topological_depth: self.max_topological_depth,
        }
    }

    fn fold_with_summary<B>(
        self,
        init: B,
        mut f: impl FnMut(B, SegmentWithTopologicalDepth<D>) -> B,
    ) -> (B, DepthStreamSummary<D>) {
        let Self {
            walk,
            mut bounds,
            mut max_topological_depth,
        } = self;
        let accumulated = match walk {
            SegmentWalk::Stamped {
                placements,
                current,
            } => {
                let mut acc = init;
                if let Some((stamp, segments)) = current {
                    acc = segments.fold(acc, |acc, segment| {
                        f(acc, stamp.transform_depth_segment(segment))
                    });
                }
                placements.fold(acc, |acc, (stamp, template)| {
                    template.include_world_bounds(stamp, &mut bounds);
                    max_topological_depth = max_topological_depth
                        .max(stamp.depth_base.saturating_add(template.max_depth_offset));
                    template.segments.iter().fold(acc, |acc, segment| {
                        f(acc, stamp.transform_depth_segment(segment))
                    })
                })
            }
            SegmentWalk::Interpreted(segments) => segments.fold(init, |acc, segment| {
                let [start, end] = segment.points;
                bounds.include_segment(start, end);
                max_topological_depth = max_topological_depth.max(segment.topological_depth);
                f(acc, segment)
            }),
        };
        (
            accumulated,
            DepthStreamSummary {
                bounds: bounds.finish(),
                max_topological_depth,
            },
        )
    }
}

impl<'a, D: TemplateDimension> Iterator for DepthSegmentStream<'a, D> {
    type Item = SegmentWithTopologicalDepth<D>;

    fn next(&mut self) -> Option<Self::Item> {
        match &mut self.walk {
            SegmentWalk::Stamped {
                placements,
                current,
            } => loop {
                if let Some((stamp, segments)) = current
                    && let Some(segment) = segments.next()
                {
                    return Some(stamp.transform_depth_segment(segment));
                }
                let (stamp, template) = placements.next()?;
                template.include_world_bounds(stamp, &mut self.bounds);
                self.max_topological_depth = self
                    .max_topological_depth
                    .max(stamp.depth_base.saturating_add(template.max_depth_offset));
                *current = Some((stamp, template.segments.iter()));
            },
            SegmentWalk::Interpreted(segments) => {
                let segment = segments.next()?;
                let [start, end] = segment.points;
                self.bounds.include_segment(start, end);
                self.max_topological_depth =
                    self.max_topological_depth.max(segment.topological_depth);
                Some(segment)
            }
        }
    }

    fn fold<B, F>(self, init: B, f: F) -> B
    where
        F: FnMut(B, Self::Item) -> B,
    {
        self.fold_with_summary(init, f).0
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use glam::Vec2;

    use super::*;
    use crate::test_util::compile_2d;
    use crate::{
        BoundsAccumulator2D, DEFAULT_TEMPLATE_SEGMENT_BUDGET, Dimensions, GenerationConfig,
    };

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

    fn prepared_stamped() -> PreparedGeneration<crate::D2> {
        let prepared = compile_2d(&plant())
            .plan_templates(DEFAULT_TEMPLATE_SEGMENT_BUDGET)
            .prepare();
        assert!(matches!(prepared, PreparedGeneration::Stamped(_)));
        prepared
    }

    fn prepared_interpreted() -> PreparedGeneration<crate::D2> {
        PreparedGeneration::Interpreted(compile_2d(&plant()))
    }

    #[test]
    fn drain_and_external_iteration_yield_identical_segments_and_bounds() {
        for prepared in [prepared_stamped(), prepared_interpreted()] {
            let mut drained = Vec::new();
            let drained_bounds = prepared
                .segments()
                .drain(|segment| drained.push(segment))
                .expect("plant produces geometry");

            let mut stream = prepared.segments();
            let stepped: Vec<[Vec2; 2]> = stream.by_ref().collect();
            let stepped_bounds = stream.finish().expect("plant produces geometry");

            assert_eq!(drained, stepped);
            assert_eq!(drained_bounds, stepped_bounds);
        }
    }

    #[test]
    fn stream_bounds_contain_every_yielded_endpoint() {
        for prepared in [prepared_stamped(), prepared_interpreted()] {
            let mut endpoints = Vec::new();
            let bounds = prepared
                .segments()
                .drain(|[start, end]| endpoints.extend([start, end]))
                .expect("plant produces geometry");
            assert!(
                endpoints.iter().all(|point| {
                    point.cmpge(bounds.min).all() && point.cmple(bounds.max).all()
                })
            );
        }
    }

    #[test]
    fn interpreted_stream_bounds_equal_direct_endpoint_accumulation() {
        let mut exact = BoundsAccumulator2D::default();
        let bounds = prepared_interpreted()
            .segments()
            .drain(|[start, end]| exact.include_segment(start, end));
        assert_eq!(bounds, exact.finish());
    }

    #[test]
    fn stream_segment_count_matches_plan_total() {
        let plan = compile_2d(&plant()).plan_templates(DEFAULT_TEMPLATE_SEGMENT_BUDGET);
        let total = plan.total_segments();
        assert_eq!(plan.prepare().segments().count() as u64, total);
    }

    #[test]
    fn partial_iteration_finish_contains_the_yielded_prefix() {
        let prepared = prepared_stamped();
        let mut stream = prepared.segments();
        let prefix: Vec<[Vec2; 2]> = stream.by_ref().take(10).collect();
        let bounds = stream.finish().expect("prefix produces bounds");
        assert!(
            prefix
                .iter()
                .flatten()
                .all(|point| { point.cmpge(bounds.min).all() && point.cmple(bounds.max).all() })
        );
    }

    #[test]
    fn empty_generation_stream_finishes_none() {
        let config = GenerationConfig::new(
            Dimensions::TwoD,
            "X".to_string(),
            0,
            90.0,
            1.0,
            0.0,
            BTreeMap::new(),
        )
        .expect("balanced config");
        let prepared = compile_2d(&config)
            .plan_templates(DEFAULT_TEMPLATE_SEGMENT_BUDGET)
            .prepare();
        assert!(matches!(prepared, PreparedGeneration::Interpreted(_)));
        let mut stream = prepared.segments();
        assert!(stream.next().is_none());
        assert_eq!(stream.finish(), None);
    }

    #[test]
    fn depth_stream_summary_matches_yielded_records() {
        for prepared in [prepared_stamped(), prepared_interpreted()] {
            let mut yielded_max = 0;
            let mut endpoints = Vec::new();
            let summary = prepared.depth_segments().drain(|segment| {
                yielded_max = yielded_max.max(segment.topological_depth);
                endpoints.extend(segment.points);
            });
            assert!(yielded_max > 0, "plant reaches nonzero depth");
            assert_eq!(summary.max_topological_depth, yielded_max);
            let bounds = summary.bounds.expect("plant produces geometry");
            assert!(
                endpoints.iter().all(|point| {
                    point.cmpge(bounds.min).all() && point.cmple(bounds.max).all()
                })
            );
        }
    }

    #[test]
    fn depth_stream_points_match_plain_stream() {
        for prepared in [prepared_stamped(), prepared_interpreted()] {
            let mut depth_points = Vec::new();
            prepared
                .depth_segments()
                .drain(|segment| depth_points.push(segment.points));
            let mut plain_points = Vec::new();
            prepared
                .segments()
                .drain(|segment| plain_points.push(segment));
            assert_eq!(depth_points, plain_points);
        }
    }
}
