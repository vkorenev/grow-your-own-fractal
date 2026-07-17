//! Shared helpers for tests that pin the `fold` fast path.

use crate::{AnyCompiledGeneration, CompiledGeneration2D, CompiledGeneration3D, GenerationConfig};

pub(crate) fn compile_2d(config: &GenerationConfig) -> CompiledGeneration2D {
    let AnyCompiledGeneration::TwoD(generation) = config.compile() else {
        panic!("expected a 2D generation config")
    };
    generation
}

pub(crate) fn compile_3d(config: &GenerationConfig) -> CompiledGeneration3D {
    let AnyCompiledGeneration::ThreeD(generation) = config.compile() else {
        panic!("expected a 3D generation config")
    };
    generation
}

/// Wraps an iterator so that draining it through `next` panics while `fold`
/// delegates normally. Tests use it to prove a consumer stays on the
/// specialized `fold` path.
pub(crate) struct FoldOnly<I> {
    inner: I,
}

impl<I> FoldOnly<I> {
    pub(crate) fn new(inner: I) -> Self {
        Self { inner }
    }
}

impl<I: Iterator> Iterator for FoldOnly<I> {
    type Item = I::Item;

    fn next(&mut self) -> Option<Self::Item> {
        panic!("iterator must be drained through fold, not next")
    }

    fn fold<B, F>(self, init: B, f: F) -> B
    where
        Self: Sized,
        F: FnMut(B, Self::Item) -> B,
    {
        self.inner.fold(init, f)
    }
}

/// Drains an iterator through repeated `next` calls, bypassing any `fold`
/// specialization. Equivalence tests use this as the oracle side; `collect`
/// would stop being a valid oracle if std ever routed it through `fold`.
pub(crate) fn collect_with_next<T>(mut iter: impl Iterator<Item = T>) -> Vec<T> {
    let mut items = Vec::new();
    loop {
        match iter.next() {
            Some(item) => items.push(item),
            None => return items,
        }
    }
}
