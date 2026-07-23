use std::{fmt::Debug, sync::LazyLock};

use glam::{Vec2, Vec3, Vec3Swizzles, Vec4};

const DIRECTION_COUNT: usize = 16;
const HALF_DIRECTION_COUNT: usize = DIRECTION_COUNT / 2;
const NUMERICAL_MARGIN_ULPS: f64 = 16.0;

// Generate one half-turn trigonometrically, then negate it so antipodal pairs
// remain exact even when native and WebAssembly libm implementations round the
// trigonometric results differently.
static SUPPORT_DIRECTIONS: LazyLock<[Vec2; DIRECTION_COUNT]> = LazyLock::new(|| {
    let mut directions = [Vec2::ZERO; DIRECTION_COUNT];
    for index in 0..HALF_DIRECTION_COUNT {
        let angle = std::f64::consts::TAU * index as f64 / DIRECTION_COUNT as f64;
        let (sin, cos) = angle.sin_cos();
        let direction = Vec2::new(cos as f32, sin as f32);
        directions[index] = direction;
        directions[index + HALF_DIRECTION_COUNT] = -direction;
    }
    directions
});

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
/// Horizontal bounds are sampled along sixteen fixed support directions. The
/// finished cylinder contains those half-planes after a conservative circular
/// inflation, while the Y interval remains exact apart from its outward
/// floating-point margin.
#[derive(Clone, Copy, Debug)]
pub struct BoundsAccumulator3D {
    supports: Option<Supports3D>,
    directions: &'static [Vec2; DIRECTION_COUNT],
}

impl Default for BoundsAccumulator3D {
    fn default() -> Self {
        Self {
            supports: None,
            directions: LazyLock::force(&SUPPORT_DIRECTIONS),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Supports3D {
    horizontal: [f32; DIRECTION_COUNT],
    min_y: f32,
    max_y: f32,
}

impl Supports3D {
    fn from_point(point: Vec3, directions: &[Vec2; DIRECTION_COUNT]) -> Self {
        let horizontal = std::array::from_fn(|index| directions[index].dot(point.xz()));
        Self {
            horizontal,
            min_y: point.y,
            max_y: point.y,
        }
    }

    fn include(&mut self, point: Vec3, directions: &[Vec2; DIRECTION_COUNT]) {
        for (support, direction) in self.horizontal.iter_mut().zip(directions.iter().copied()) {
            *support = support.max(direction.dot(point.xz()));
        }
        self.min_y = self.min_y.min(point.y);
        self.max_y = self.max_y.max(point.y);
    }

    fn merge(&mut self, other: Self) {
        for (support, other_support) in self.horizontal.iter_mut().zip(other.horizontal) {
            *support = support.max(other_support);
        }
        self.min_y = self.min_y.min(other.min_y);
        self.max_y = self.max_y.max(other.max_y);
    }
}

impl BoundsAccumulator for BoundsAccumulator3D {
    type Point = Vec3;
    type Bounds = BoundingCylinder3D;

    fn include(&mut self, point: Vec3) {
        debug_assert!(point.is_finite(), "bounds point must be finite");
        self.supports = Some(match self.supports {
            Some(mut supports) => {
                supports.include(point, self.directions);
                supports
            }
            None => Supports3D::from_point(point, self.directions),
        });
    }

    fn merge(&mut self, other: &Self) {
        let Some(other) = other.supports else {
            return;
        };
        self.supports = Some(match self.supports {
            Some(mut supports) => {
                supports.merge(other);
                supports
            }
            None => other,
        });
    }

    fn finish(self) -> Option<BoundingCylinder3D> {
        self.supports
            .map(|supports| finish_cylinder(supports, self.directions))
    }
}

fn finish_cylinder(
    supports: Supports3D,
    directions: &[Vec2; DIRECTION_COUNT],
) -> BoundingCylinder3D {
    let horizontal = supports.horizontal.map(f64::from);
    let center = active_set_center(&horizontal, directions)
        .expect("fixed support directions contain a finite nonsingular active set");
    // The stored f32 center is what callers use. Re-evaluate supports against
    // that exact represented value before inflating the radius.
    let center = Vec2::new(center[0] as f32, center[1] as f32);
    let center64 = [f64::from(center.x), f64::from(center.y)];
    let t = horizontal
        .iter()
        .zip(directions.iter().copied())
        .map(|(&support, direction)| {
            support - center64[0] * f64::from(direction.x) - center64[1] * f64::from(direction.y)
        })
        .fold(0.0_f64, f64::max);
    let magnitude = horizontal
        .iter()
        .copied()
        .chain([
            center64[0],
            center64[1],
            f64::from(supports.min_y),
            f64::from(supports.max_y),
        ])
        .map(f64::abs)
        .fold(1.0_f64, f64::max);
    let margin = NUMERICAL_MARGIN_ULPS * f64::from(f32::EPSILON) * magnitude;
    BoundingCylinder3D {
        center_xz: center,
        radius: round_up_f32(t / direction_coverage() + margin),
        min_y: round_down_f32(f64::from(supports.min_y) - margin),
        max_y: round_up_f32(f64::from(supports.max_y) + margin),
    }
}

/// Returns the ideal projection onto the nearest evenly spaced direction.
fn direction_coverage() -> f64 {
    (std::f64::consts::PI / DIRECTION_COUNT as f64).cos()
}

fn active_set_center(
    supports: &[f64; DIRECTION_COUNT],
    directions: &[Vec2; DIRECTION_COUNT],
) -> Option<[f64; 2]> {
    let mut best: Option<(f64, [f64; 2])> = None;
    for first in 0..DIRECTION_COUNT - 2 {
        for second in first + 1..DIRECTION_COUNT - 1 {
            for third in second + 1..DIRECTION_COUNT {
                let Some(center) = solve_active_set(supports, directions, [first, second, third])
                else {
                    continue;
                };
                let t = maximum_support_distance(supports, directions, center);
                if !t.is_finite() {
                    continue;
                }
                if best.is_none_or(|(best_t, _)| t < best_t) {
                    best = Some((t, center));
                }
            }
        }
    }
    best.map(|(_, center)| center)
}

fn solve_active_set(
    supports: &[f64; DIRECTION_COUNT],
    directions: &[Vec2; DIRECTION_COUNT],
    indices: [usize; 3],
) -> Option<[f64; 2]> {
    let rows = indices.map(|index| {
        let direction = directions[index];
        [
            f64::from(direction.x),
            f64::from(direction.y),
            1.0,
            supports[index],
        ]
    });
    let determinant = determinant3(rows.map(|row| [row[0], row[1], row[2]]));
    if determinant.abs() <= f64::EPSILON {
        return None;
    }
    let x = determinant3(rows.map(|row| [row[3], row[1], row[2]])) / determinant;
    let y = determinant3(rows.map(|row| [row[0], row[3], row[2]])) / determinant;
    (x.is_finite() && y.is_finite()).then_some([x, y])
}

fn determinant3(rows: [[f64; 3]; 3]) -> f64 {
    rows[0][0] * (rows[1][1] * rows[2][2] - rows[1][2] * rows[2][1])
        - rows[0][1] * (rows[1][0] * rows[2][2] - rows[1][2] * rows[2][0])
        + rows[0][2] * (rows[1][0] * rows[2][1] - rows[1][1] * rows[2][0])
}

fn maximum_support_distance(
    supports: &[f64; DIRECTION_COUNT],
    directions: &[Vec2; DIRECTION_COUNT],
    center: [f64; 2],
) -> f64 {
    supports
        .iter()
        .zip(directions.iter().copied())
        .map(|(&support, direction)| {
            support - center[0] * f64::from(direction.x) - center[1] * f64::from(direction.y)
        })
        .fold(f64::NEG_INFINITY, f64::max)
}

fn round_up_f32(value: f64) -> f32 {
    let rounded = value as f32;
    if f64::from(rounded) < value {
        if rounded.is_sign_negative() {
            f32::from_bits(rounded.to_bits() - 1)
        } else {
            f32::from_bits(rounded.to_bits() + 1)
        }
    } else {
        rounded
    }
}

fn round_down_f32(value: f64) -> f32 {
    let rounded = value as f32;
    if f64::from(rounded) > value {
        if rounded.is_sign_negative() {
            f32::from_bits(rounded.to_bits() + 1)
        } else {
            f32::from_bits(rounded.to_bits() - 1)
        }
    } else {
        rounded
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

    fn cylinder_contains(bounds: BoundingCylinder3D, point: Vec3) -> bool {
        Vec2::new(point.x, point.z).distance(bounds.center_xz) <= bounds.radius
            && point.y >= bounds.min_y
            && point.y <= bounds.max_y
    }

    fn accumulated_cylinder(points: impl IntoIterator<Item = Vec3>) -> BoundingCylinder3D {
        let mut accumulator = BoundsAccumulator3D::default();
        for point in points {
            accumulator.include(point);
        }
        accumulator
            .finish()
            .expect("non-empty points produce bounds")
    }

    fn assert_cylinder_contains_all(points: &[Vec3]) {
        let bounds = accumulated_cylinder(points.iter().copied());
        assert!(
            points
                .iter()
                .copied()
                .all(|point| cylinder_contains(bounds, point))
        );
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
        let point = Vec3::new(2.0, -3.0, 4.0);
        let bounds = generic_singleton_bounds::<D3>(point).expect("point produces bounds");
        assert!(cylinder_contains(bounds, point));
        assert!(bounds.radius > 0.0);
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
    fn three_dimensional_bounds_contain_every_input_endpoint() {
        let points = [
            Vec3::new(-2.0, -3.0, -4.0),
            Vec3::new(6.0, 7.0, 2.0),
            Vec3::new(1.0, 4.0, -1.0),
        ];
        assert_cylinder_contains_all(&points);
    }

    #[test]
    fn directional_cylinder_contains_degenerate_and_spatial_clouds() {
        let singleton = [Vec3::new(3.0, -2.0, 5.0)];
        let repeated = [Vec3::new(3.0, -2.0, 5.0); 4];
        let collinear = [
            Vec3::new(-4.0, -1.0, 2.0),
            Vec3::new(0.0, 0.0, 2.0),
            Vec3::new(7.0, 3.0, 2.0),
        ];
        let circle = [
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.707_106_77, 0.0, 0.707_106_77),
            Vec3::new(0.0, 0.0, 1.0),
            Vec3::new(-0.707_106_77, 0.0, 0.707_106_77),
            Vec3::new(-1.0, 0.0, 0.0),
            Vec3::new(-0.707_106_77, 0.0, -0.707_106_77),
            Vec3::new(0.0, 0.0, -1.0),
            Vec3::new(0.707_106_77, 0.0, -0.707_106_77),
        ];
        let rectangle = [
            Vec3::new(-3.0, -1.0, -2.0),
            Vec3::new(5.0, -1.0, -2.0),
            Vec3::new(5.0, 4.0, 7.0),
            Vec3::new(-3.0, 4.0, 7.0),
        ];
        let cube = [-1.0, 1.0]
            .into_iter()
            .flat_map(|x| {
                [-1.0, 1.0]
                    .into_iter()
                    .flat_map(move |y| [-1.0, 1.0].into_iter().map(move |z| Vec3::new(x, y, z)))
            })
            .collect::<Vec<_>>();
        let skewed = [
            Vec3::new(-9.0, 4.0, 2.0),
            Vec3::new(11.0, -6.0, -3.0),
            Vec3::new(2.0, 8.0, 12.0),
            Vec3::new(-1.0, -4.0, 7.0),
            Vec3::new(6.0, 1.0, -11.0),
        ];

        for cloud in [
            &singleton[..],
            &repeated,
            &collinear,
            &circle,
            &rectangle,
            &cube,
            &skewed,
        ] {
            assert_cylinder_contains_all(cloud);
        }
    }

    #[test]
    fn directional_cylinder_remains_conservative_across_scales_and_translation() {
        let local = [
            Vec3::new(-2.0, -3.0, 1.0),
            Vec3::new(4.0, 6.0, -5.0),
            Vec3::new(1.0, -2.0, 7.0),
        ];
        for scale in [1.0e-3, 1.0, 1.0e3, 1.0e6] {
            let points = local.map(|point| point * scale);
            assert_cylinder_contains_all(&points);
        }

        let translation = Vec3::new(1.0e6, -1.0e6, 5.0e5);
        let points = local.map(|point| translation + point * 0.01);
        assert_cylinder_contains_all(&points);
    }

    #[test]
    fn generated_directions_are_unit_length_and_antipodal() {
        let directions = BoundsAccumulator3D::default().directions;
        for index in 0..HALF_DIRECTION_COUNT {
            let direction = directions[index];
            let opposite = directions[index + HALF_DIRECTION_COUNT];
            assert!((direction.length_squared() - 1.0).abs() <= 2.0e-7);
            assert_eq!(opposite, -direction);
        }
    }

    #[test]
    fn directional_cylinder_respects_the_sampling_approximation_bound() {
        let points = [
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(-1.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
            Vec3::new(0.0, 0.0, -1.0),
        ];
        let mut accumulator = BoundsAccumulator3D::default();
        for point in points {
            accumulator.include(point);
        }
        let bounds = accumulator.finish().expect("points produce bounds");
        assert!(
            points
                .into_iter()
                .all(|point| cylinder_contains(bounds, point))
        );
        assert!(f64::from(bounds.radius) <= 1.0 / direction_coverage() + 1.0e-4);
    }

    #[test]
    fn directional_cylinder_stays_tighter_than_the_inflated_aabb_baseline() {
        let points = [
            Vec3::new(-9.0, 4.0, 2.0),
            Vec3::new(11.0, -6.0, -3.0),
            Vec3::new(2.0, 8.0, 12.0),
            Vec3::new(-1.0, -4.0, 7.0),
            Vec3::new(6.0, 1.0, -11.0),
        ];
        let bounds = accumulated_cylinder(points);
        let (min, max) = points.iter().fold(
            (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY)),
            |(min, max), point| (min.min(point.xz()), max.max(point.xz())),
        );
        let aabb_radius = f64::from(((max - min) * 0.5).length());
        let magnitude = points
            .iter()
            .flat_map(|point| point.to_array())
            .map(f64::from)
            .map(f64::abs)
            .fold(1.0_f64, f64::max);
        // Cover support, center, margin, and final f32 rounding without making
        // the assertion loose enough to admit a materially broken fit.
        let numerical_allowance = 4.0 * NUMERICAL_MARGIN_ULPS * f64::from(f32::EPSILON) * magnitude;

        assert!(
            f64::from(bounds.radius) <= aabb_radius / direction_coverage() + numerical_allowance,
            "directional radius {} exceeded inflated AABB radius {}",
            bounds.radius,
            aabb_radius / direction_coverage()
        );
    }

    #[test]
    fn direction_coverage_uses_the_half_sampling_angle() {
        assert_eq!(
            direction_coverage(),
            (std::f64::consts::PI / DIRECTION_COUNT as f64).cos()
        );
    }

    #[test]
    fn three_dimensional_accumulation_is_order_independent() {
        let points = [
            Vec3::new(-4.0, 2.0, 5.0),
            Vec3::new(7.0, -3.0, -6.0),
            Vec3::new(1.0, 8.0, 2.0),
            Vec3::new(-2.0, -1.0, 9.0),
        ];
        let mut forward = BoundsAccumulator3D::default();
        let mut reverse = BoundsAccumulator3D::default();
        for point in points {
            forward.include(point);
        }
        for point in points.into_iter().rev() {
            reverse.include(point);
        }
        assert_eq!(forward.finish(), reverse.finish());

        let rotated = [points[2], points[0], points[3], points[1]];
        assert_eq!(accumulated_cylinder(rotated), accumulated_cylinder(points));
    }

    #[test]
    fn three_dimensional_partitions_merge_to_the_same_bound() {
        let points = [
            Vec3::new(-4.0, 2.0, 5.0),
            Vec3::new(7.0, -3.0, -6.0),
            Vec3::new(1.0, 8.0, 2.0),
            Vec3::new(-2.0, -1.0, 9.0),
            Vec3::new(6.0, 5.0, -3.0),
        ];
        let complete = accumulated_cylinder(points);
        for split in 0..=points.len() {
            let mut left = BoundsAccumulator3D::default();
            let mut right = BoundsAccumulator3D::default();
            for point in &points[..split] {
                left.include(*point);
            }
            for point in &points[split..] {
                right.include(*point);
            }
            left.merge(&right);
            assert_eq!(left.finish(), Some(complete));
        }

        let mut partitions = [BoundsAccumulator3D::default(); 3];
        for (index, point) in points.into_iter().enumerate() {
            partitions[index % partitions.len()].include(point);
        }
        let mut merged = BoundsAccumulator3D::default();
        for partition in partitions {
            merged.merge(&partition);
        }
        assert_eq!(merged.finish(), Some(complete));
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
