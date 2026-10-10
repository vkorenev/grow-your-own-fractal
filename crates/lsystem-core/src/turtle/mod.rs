pub(crate) mod turtle2d;
pub(crate) mod turtle3d;

use crate::{D2, D3, Dimension, SegmentWithTopologicalDepth};
use glam::{Quat, Vec2, Vec3};

#[doc(hidden)]
pub trait Turtle: Send {
    type Dimension: Dimension;

    fn new(angle_deg: f32, step: f32, initial_heading_deg: f32) -> Self;
    fn position(&self) -> <Self::Dimension as Dimension>::Point;
    fn advance(&mut self, delta: <Self::Dimension as Dimension>::Point);
    fn heading(&self) -> <Self::Dimension as Dimension>::Rotation;
    fn normalized_heading(&self) -> <Self::Dimension as Dimension>::Rotation;
    fn compose_heading(&mut self, rot: <Self::Dimension as Dimension>::Rotation);
    fn topological_depth(&self) -> u32;
    fn add_topological_depth(&mut self, delta: u32);
    fn stack_is_empty(&self) -> bool;
    fn apply(&mut self, symbol: u8) -> Option<SegmentWithTopologicalDepth<Self::Dimension>>;
}

/// Dimension-keyed turtle construction and the point/rotation operations the
/// generic template walk needs. Crate-private: the public face stays
/// `TemplateDimension`, whose blanket impl is bounded on this trait.
#[doc(hidden)]
pub trait TurtleDimension: Dimension {
    type Turtle: Turtle<Dimension = Self>;

    const POINT_ZERO: Self::Point;
    const ROT_IDENTITY: Self::Rotation;

    /// `+X` scaled by the step length: the unit-`F` template's end point.
    fn unit_step(step: f32) -> Self::Point;
    /// Applies a rotation to a local-frame point (world placement).
    #[inline]
    fn rotate(rotation: Self::Rotation, point: Self::Point) -> Self::Point {
        Self::transform_point(Self::POINT_ZERO, rotation, point)
    }
}

impl TurtleDimension for D2 {
    type Turtle = turtle2d::TurtleState2D;

    const POINT_ZERO: Vec2 = Vec2::ZERO;
    // Unit-complex identity rotation.
    const ROT_IDENTITY: Vec2 = Vec2::X;

    #[inline]
    fn unit_step(step: f32) -> Vec2 {
        Vec2::X * step
    }
}

impl TurtleDimension for D3 {
    type Turtle = turtle3d::TurtleState3D;

    const POINT_ZERO: Vec3 = Vec3::ZERO;
    const ROT_IDENTITY: Quat = Quat::IDENTITY;

    #[inline]
    fn unit_step(step: f32) -> Vec3 {
        Vec3::X * step
    }
}

pub(crate) struct DepthSegments<I, T> {
    symbols: I,
    state: T,
}

impl<I, T> DepthSegments<I, T> {
    pub(crate) fn new(symbols: I, state: T) -> Self {
        Self { symbols, state }
    }
}

impl<I, T> Iterator for DepthSegments<I, T>
where
    I: Iterator<Item = u8>,
    T: Turtle,
{
    type Item = SegmentWithTopologicalDepth<T::Dimension>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let symbol = self.symbols.next()?;
            if let Some(segment) = self.state.apply(symbol) {
                return Some(segment);
            }
        }
    }

    // Renderer drains rely on forwarding `fold` to the symbol iterator; the
    // `FoldOnly` and fold-vs-next tests in both turtle modules pin delegation
    // and equivalence with `next`.
    fn fold<B, F>(self, init: B, mut f: F) -> B
    where
        Self: Sized,
        F: FnMut(B, Self::Item) -> B,
    {
        let mut state = self.state;
        self.symbols
            .fold(init, |acc, symbol| match state.apply(symbol) {
                Some(segment) => f(acc, segment),
                None => acc,
            })
    }
}
