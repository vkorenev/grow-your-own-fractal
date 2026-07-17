use std::fmt::Debug;

use glam::{Quat, Vec2, Vec3};

use crate::config::Dimensions;

mod sealed {
    pub trait Sealed {}
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct D2;

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct D3;

impl sealed::Sealed for D2 {}
impl sealed::Sealed for D3 {}

/// Type-level identity and representation for a supported spatial dimension.
pub trait Dimension: sealed::Sealed + Copy + Debug + 'static {
    type Point: Copy + PartialEq + Debug;
    type Rotation: Copy + PartialEq + Debug;

    const RUNTIME: Dimensions;
}

impl Dimension for D2 {
    type Point = Vec2;
    type Rotation = Vec2;

    const RUNTIME: Dimensions = Dimensions::TwoD;
}

impl Dimension for D3 {
    type Point = Vec3;
    type Rotation = Quat;

    const RUNTIME: Dimensions = Dimensions::ThreeD;
}
