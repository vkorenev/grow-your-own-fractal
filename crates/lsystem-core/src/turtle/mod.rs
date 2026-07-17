pub(crate) mod turtle2d;
pub(crate) mod turtle3d;

use crate::{Dimension, SegmentWithTopologicalDepth};

pub(crate) trait Turtle {
    type Dimension: Dimension;

    fn apply(&mut self, symbol: u8) -> Option<SegmentWithTopologicalDepth<Self::Dimension>>;
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
