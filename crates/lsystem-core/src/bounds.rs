use std::fmt::Debug;

use glam::{Vec2, Vec3, Vec4};

/// Axis-aligned bounds containing a set of two-dimensional points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds2D {
    /// Component-wise minimum point.
    pub min: Vec2,
    /// Component-wise maximum point.
    pub max: Vec2,
}

/// A world-Y cylinder containing a set of three-dimensional points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoundingCylinder3D {
    /// Horizontal center, where `x` is world X and `y` is world Z.
    pub center_xz: Vec2,
    /// Horizontal radius around [`Self::center_xz`].
    pub radius: f32,
    /// Minimum world-Y coordinate.
    pub min_y: f32,
    /// Maximum world-Y coordinate.
    pub max_y: f32,
}

/// Incrementally computes mergeable bounds for one point representation.
///
/// Empty accumulators finish as `None`; consumers choose any context-specific
/// fallback after accumulation.
///
/// Code generic over a [`crate::Dimension`] can construct and use its paired
/// accumulator without a dimension-specific factory:
///
/// ```
/// use lsystem_core::{BoundsAccumulator, Dimension};
///
/// fn accumulated_bounds<D>(
///     points: impl IntoIterator<Item = D::Point>,
/// ) -> Option<D::Bounds>
/// where
///     D: Dimension,
/// {
///     let mut accumulator = D::BoundsAccumulator::default();
///     let empty = D::BoundsAccumulator::default();
///     accumulator.merge(&empty);
///     for point in points {
///         accumulator.include(point);
///     }
///     accumulator.finish()
/// }
/// ```
pub trait BoundsAccumulator: Default {
    /// Point type accepted by this accumulator.
    type Point;
    /// Finished bounds type produced by this accumulator.
    type Bounds;

    /// Includes one finite point in the accumulated bounds.
    ///
    /// Passing a non-finite point violates this method's precondition. Debug
    /// builds assert the precondition; release behavior is unspecified but
    /// non-panicking.
    fn include(&mut self, point: Self::Point);

    /// Includes both finite endpoints of a line segment.
    ///
    /// The default keeps custom accumulators source-compatible. Dimension
    /// implementations can override it to combine the two endpoints in one
    /// vector operation.
    fn include_segment(&mut self, start: Self::Point, end: Self::Point) {
        self.include(start);
        self.include(end);
    }

    /// Merges all points represented by `other` into this accumulator.
    fn merge(&mut self, other: &Self);

    /// Returns the finished bounds, or `None` if no points were included.
    fn finish(self) -> Option<Self::Bounds>;
}

/// Mergeable two-dimensional bounds accumulator.
///
/// This accumulator computes exact component-wise endpoint minima and maxima.
#[derive(Clone, Copy, Debug, Default)]
pub struct BoundsAccumulator2D {
    // `[max_x, max_y, -min_x, -min_y]`. Updating this single vector lets the
    // hot upload loop use one packed max operation per endpoint instead of
    // separate Vec2 min and max operations.
    bounds: Option<Vec4>,
}

impl BoundsAccumulator for BoundsAccumulator2D {
    type Point = Vec2;
    type Bounds = Bounds2D;

    fn include(&mut self, point: Vec2) {
        debug_assert!(point.is_finite(), "bounds point must be finite");
        let point = Vec4::new(point.x, point.y, -point.x, -point.y);
        self.bounds = Some(self.bounds.map_or(point, |bounds| bounds.max(point)));
    }

    fn include_segment(&mut self, start: Vec2, end: Vec2) {
        debug_assert!(start.is_finite(), "bounds point must be finite");
        debug_assert!(end.is_finite(), "bounds point must be finite");
        let start = Vec4::new(start.x, start.y, -start.x, -start.y);
        let end = Vec4::new(end.x, end.y, -end.x, -end.y);
        self.bounds = Some(match self.bounds {
            Some(bounds) => bounds.max(start).max(end),
            None => start.max(end),
        });
    }

    fn merge(&mut self, other: &Self) {
        let Some(other) = other.bounds else {
            return;
        };
        self.bounds = Some(match self.bounds {
            Some(bounds) => bounds.max(other),
            None => other,
        });
    }

    fn finish(self) -> Option<Bounds2D> {
        self.bounds.map(|bounds| Bounds2D {
            min: Vec2::new(-bounds.z, -bounds.w),
            max: Vec2::new(bounds.x, bounds.y),
        })
    }
}

/// Mergeable three-dimensional bounds accumulator.
///
/// Stage 1 derives a world-Y cylinder from exact component-wise endpoint
/// minima and maxima. The public type deliberately does not expose that
/// implementation detail so a tighter accumulation algorithm can replace it.
#[derive(Clone, Copy, Debug, Default)]
pub struct BoundsAccumulator3D {
    aabb: Option<(Vec3, Vec3)>,
}

impl BoundsAccumulator for BoundsAccumulator3D {
    type Point = Vec3;
    type Bounds = BoundingCylinder3D;

    fn include(&mut self, point: Vec3) {
        debug_assert!(point.is_finite(), "bounds point must be finite");
        self.aabb = Some(match self.aabb {
            Some((min, max)) => (min.min(point), max.max(point)),
            None => (point, point),
        });
    }

    fn merge(&mut self, other: &Self) {
        let Some((other_min, other_max)) = other.aabb else {
            return;
        };
        self.aabb = Some(match self.aabb {
            Some((min, max)) => (min.min(other_min), max.max(other_max)),
            None => (other_min, other_max),
        });
    }

    fn finish(self) -> Option<BoundingCylinder3D> {
        self.aabb.map(|(min, max)| {
            // Preserve the operation ordering used by the previous renderer
            // camera path so Stage 1 changes only the shape of the typed data.
            let center = (min + max) * 0.5;
            let half = (max - min) * 0.5;
            BoundingCylinder3D {
                center_xz: Vec2::new(center.x, center.z),
                radius: half.x.hypot(half.z),
                min_y: min.y,
                max_y: max.y,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{D2, D3, Dimension};

    fn generic_singleton_bounds<D>(point: D::Point) -> Option<D::Bounds>
    where
        D: Dimension,
    {
        let mut accumulator = D::BoundsAccumulator::default();
        let empty = D::BoundsAccumulator::default();
        accumulator.merge(&empty);
        accumulator.include(point);
        accumulator.finish()
    }

    #[test]
    fn dimension_api_constructs_paired_accumulators_generically() {
        assert_eq!(
            generic_singleton_bounds::<D2>(Vec2::new(2.0, -3.0)),
            Some(Bounds2D {
                min: Vec2::new(2.0, -3.0),
                max: Vec2::new(2.0, -3.0),
            })
        );
        assert_eq!(
            generic_singleton_bounds::<D3>(Vec3::new(2.0, -3.0, 4.0)),
            Some(BoundingCylinder3D {
                center_xz: Vec2::new(2.0, 4.0),
                radius: 0.0,
                min_y: -3.0,
                max_y: -3.0,
            })
        );
    }

    #[test]
    fn empty_accumulators_finish_as_none() {
        assert_eq!(BoundsAccumulator2D::default().finish(), None);
        assert_eq!(BoundsAccumulator3D::default().finish(), None);
    }

    #[test]
    fn two_dimensional_bounds_are_exact() {
        let points = [
            Vec2::new(3.0, -4.0),
            Vec2::new(-2.0, 5.0),
            Vec2::new(1.0, 2.0),
        ];
        let mut accumulator = BoundsAccumulator2D::default();
        for point in points {
            accumulator.include(point);
        }

        let bounds = accumulator.finish().expect("points produce bounds");
        assert_eq!(bounds.min, Vec2::new(-2.0, -4.0));
        assert_eq!(bounds.max, Vec2::new(3.0, 5.0));
        assert!(
            points
                .into_iter()
                .all(|point| point.cmpge(bounds.min).all() && point.cmple(bounds.max).all())
        );
    }

    #[test]
    fn segment_inclusion_matches_individual_points() {
        let segments_2d = [
            [Vec2::new(3.0, -4.0), Vec2::new(-2.0, 5.0)],
            [Vec2::new(1.0, 2.0), Vec2::new(-6.0, -1.0)],
        ];
        let mut by_points_2d = BoundsAccumulator2D::default();
        let mut by_segments_2d = BoundsAccumulator2D::default();
        for [start, end] in segments_2d {
            by_points_2d.include(start);
            by_points_2d.include(end);
            by_segments_2d.include_segment(start, end);
        }
        assert_eq!(by_segments_2d.finish(), by_points_2d.finish());

        let segments_3d = [
            [Vec3::new(3.0, -4.0, 5.0), Vec3::new(-2.0, 5.0, 1.0)],
            [Vec3::new(1.0, 2.0, -6.0), Vec3::new(-6.0, -1.0, 4.0)],
        ];
        let mut by_points_3d = BoundsAccumulator3D::default();
        let mut by_segments_3d = BoundsAccumulator3D::default();
        for [start, end] in segments_3d {
            by_points_3d.include(start);
            by_points_3d.include(end);
            by_segments_3d.include_segment(start, end);
        }
        assert_eq!(by_segments_3d.finish(), by_points_3d.finish());
    }

    #[test]
    fn three_dimensional_bounds_match_the_previous_aabb_cylinder() {
        let points = [
            Vec3::new(-2.0, -3.0, -4.0),
            Vec3::new(6.0, 7.0, 2.0),
            Vec3::new(1.0, 4.0, -1.0),
        ];
        let mut accumulator = BoundsAccumulator3D::default();
        for point in points {
            accumulator.include(point);
        }

        let bounds = accumulator.finish().expect("points produce bounds");
        assert_eq!(bounds.center_xz, Vec2::new(2.0, -1.0));
        assert_eq!(bounds.radius, 5.0);
        assert_eq!(bounds.min_y, -3.0);
        assert_eq!(bounds.max_y, 7.0);
        assert!(points.into_iter().all(|point| {
            Vec2::new(point.x, point.z).distance(bounds.center_xz) <= bounds.radius
                && point.y >= bounds.min_y
                && point.y <= bounds.max_y
        }));
    }

    #[test]
    fn merged_partitions_match_single_accumulators() {
        let points_2d = [
            Vec2::new(-4.0, 2.0),
            Vec2::new(7.0, -3.0),
            Vec2::new(1.0, 8.0),
        ];
        let mut complete_2d = BoundsAccumulator2D::default();
        let mut left_2d = BoundsAccumulator2D::default();
        let mut right_2d = BoundsAccumulator2D::default();
        for point in points_2d {
            complete_2d.include(point);
        }
        let mut merged_into_empty_2d = BoundsAccumulator2D::default();
        merged_into_empty_2d.merge(&complete_2d);
        assert_eq!(merged_into_empty_2d.finish(), complete_2d.finish());
        left_2d.include(points_2d[0]);
        for point in &points_2d[1..] {
            right_2d.include(*point);
        }
        left_2d.merge(&right_2d);
        left_2d.merge(&BoundsAccumulator2D::default());
        assert_eq!(left_2d.finish(), complete_2d.finish());

        let points_3d = [
            Vec3::new(-4.0, 2.0, 5.0),
            Vec3::new(7.0, -3.0, -6.0),
            Vec3::new(1.0, 8.0, 2.0),
        ];
        let mut complete_3d = BoundsAccumulator3D::default();
        let mut left_3d = BoundsAccumulator3D::default();
        let mut right_3d = BoundsAccumulator3D::default();
        for point in points_3d {
            complete_3d.include(point);
        }
        let mut merged_into_empty_3d = BoundsAccumulator3D::default();
        merged_into_empty_3d.merge(&complete_3d);
        assert_eq!(merged_into_empty_3d.finish(), complete_3d.finish());
        left_3d.include(points_3d[0]);
        for point in &points_3d[1..] {
            right_3d.include(*point);
        }
        left_3d.merge(&right_3d);
        left_3d.merge(&BoundsAccumulator3D::default());
        assert_eq!(left_3d.finish(), complete_3d.finish());
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "bounds point must be finite")]
    fn two_dimensional_accumulator_rejects_non_finite_points_in_debug_builds() {
        BoundsAccumulator2D::default().include(Vec2::new(f32::NAN, 0.0));
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "bounds point must be finite")]
    fn three_dimensional_accumulator_rejects_non_finite_points_in_debug_builds() {
        BoundsAccumulator3D::default().include(Vec3::new(0.0, f32::INFINITY, 0.0));
    }
}
