use std::fmt::Debug;

use glam::{Quat, Vec2, Vec3};

use crate::config::Dimensions;

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

    const RUNTIME: Dimensions;

    /// Places a point expressed in a local frame into a world frame.
    fn transform_point(
        position: Self::Point,
        rotation: Self::Rotation,
        point: Self::Point,
    ) -> Self::Point;
}

impl Dimension for D2 {
    type Point = Vec2;
    type Rotation = Vec2;

    const RUNTIME: Dimensions = Dimensions::TwoD;

    #[inline]
    fn transform_point(position: Vec2, rotation: Vec2, point: Vec2) -> Vec2 {
        position + rotation.rotate(point)
    }
}

impl Dimension for D3 {
    type Point = Vec3;
    type Rotation = Quat;

    const RUNTIME: Dimensions = Dimensions::ThreeD;

    #[inline]
    fn transform_point(position: Vec3, rotation: Quat, point: Vec3) -> Vec3 {
        position + rotation * point
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transforms_2d_point_from_local_to_world_space() {
        let transformed = D2::transform_point(
            Vec2::new(3.0, -2.0),
            Vec2::from_angle(std::f32::consts::FRAC_PI_2),
            Vec2::new(1.0, 2.0),
        );

        assert!(transformed.abs_diff_eq(Vec2::new(1.0, -1.0), 1.0e-6));
    }

    #[test]
    fn transforms_3d_point_from_local_to_world_space() {
        let transformed = D3::transform_point(
            Vec3::new(3.0, -2.0, 1.0),
            Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
            Vec3::new(1.0, 2.0, 3.0),
        );

        assert!(transformed.abs_diff_eq(Vec3::new(1.0, -1.0, 4.0), 1.0e-6));
    }
}
