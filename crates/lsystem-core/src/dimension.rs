use std::fmt::Debug;

use glam::{Quat, Vec2, Vec3};

use crate::bounds::{
    BoundingCylinder3D, Bounds2D, BoundsAccumulator as BoundsAccumulatorTrait, BoundsAccumulator2D,
    BoundsAccumulator3D,
};
use crate::config::Dimensions;
use crate::template::{Summary2D, Summary3D};

mod sealed {
    pub trait Sealed {}
}

/// Type-level marker for two-dimensional generation.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct D2;

/// Type-level marker for three-dimensional generation.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct D3;

impl sealed::Sealed for D2 {}
impl sealed::Sealed for D3 {}

/// Type-level identity and representation for a supported spatial dimension.
pub trait Dimension: sealed::Sealed + Copy + Debug + 'static {
    type Point: Copy + PartialEq + Debug;
    type Rotation: Copy + PartialEq + Debug;
    /// Finished bounds representation paired with this dimension.
    type Bounds: Copy + Debug;
    /// Incremental bounds accumulator paired with this dimension.
    type BoundsAccumulator: BoundsAccumulatorTrait<Point = Self::Point, Bounds = Self::Bounds>;
    /// Conservative local-frame bounds summary stored per template.
    type Summary: Clone + Debug + PartialEq;

    const RUNTIME: Dimensions;

    /// Places a point expressed in a local frame into a world frame.
    fn transform_point(
        position: Self::Point,
        rotation: Self::Rotation,
        point: Self::Point,
    ) -> Self::Point;

    /// Places both endpoints of a local-frame segment into a world frame.
    ///
    /// Keeping the pair as a dimension-specific operation preserves SIMD
    /// code generation for 3D quaternion transforms in generic consumers.
    fn transform_points(
        position: Self::Point,
        rotation: Self::Rotation,
        points: [Self::Point; 2],
    ) -> [Self::Point; 2];
}

#[inline(always)]
fn transform_point_2d(position: Vec2, rotation: Vec2, point: Vec2) -> Vec2 {
    position + rotation.rotate(point)
}

#[inline(always)]
fn transform_point_3d(position: Vec3, rotation: Quat, point: Vec3) -> Vec3 {
    position + rotation * point
}

impl Dimension for D2 {
    type Point = Vec2;
    type Rotation = Vec2;
    type Bounds = Bounds2D;
    type BoundsAccumulator = BoundsAccumulator2D;
    type Summary = Summary2D;

    const RUNTIME: Dimensions = Dimensions::TwoD;

    #[inline]
    fn transform_point(position: Vec2, rotation: Vec2, point: Vec2) -> Vec2 {
        transform_point_2d(position, rotation, point)
    }

    #[inline]
    fn transform_points(position: Vec2, rotation: Vec2, points: [Vec2; 2]) -> [Vec2; 2] {
        [
            transform_point_2d(position, rotation, points[0]),
            transform_point_2d(position, rotation, points[1]),
        ]
    }
}

impl Dimension for D3 {
    type Point = Vec3;
    type Rotation = Quat;
    type Bounds = BoundingCylinder3D;
    type BoundsAccumulator = BoundsAccumulator3D;
    type Summary = Summary3D;

    const RUNTIME: Dimensions = Dimensions::ThreeD;

    #[inline]
    fn transform_point(position: Vec3, rotation: Quat, point: Vec3) -> Vec3 {
        transform_point_3d(position, rotation, point)
    }

    #[inline]
    fn transform_points(position: Vec3, rotation: Quat, points: [Vec3; 2]) -> [Vec3; 2] {
        points.map(|point| transform_point_3d(position, rotation, point))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transforms_2d_point_from_local_to_world_space() {
        let position = Vec2::new(3.0, -2.0);
        let rotation = Vec2::from_angle(std::f32::consts::FRAC_PI_2);
        let point = Vec2::new(1.0, 2.0);
        let transformed = D2::transform_point(position, rotation, point);

        assert!(transformed.abs_diff_eq(Vec2::new(1.0, -1.0), 1.0e-6));
        let transformed_pair = D2::transform_points(position, rotation, [point, Vec2::X]);
        assert!(transformed_pair[0].abs_diff_eq(transformed, 1.0e-6));
        assert!(transformed_pair[1].abs_diff_eq(Vec2::new(3.0, -1.0), 1.0e-6));
    }

    #[test]
    fn transforms_3d_point_from_local_to_world_space() {
        let position = Vec3::new(3.0, -2.0, 1.0);
        let rotation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let point = Vec3::new(1.0, 2.0, 3.0);
        let transformed = D3::transform_point(position, rotation, point);

        assert!(transformed.abs_diff_eq(Vec3::new(1.0, -1.0, 4.0), 1.0e-6));
        let transformed_pair = D3::transform_points(position, rotation, [point, Vec3::X]);
        assert!(transformed_pair[0].abs_diff_eq(transformed, 1.0e-6));
        assert!(transformed_pair[1].abs_diff_eq(Vec3::new(3.0, -1.0, 1.0), 1.0e-6));
    }
}
