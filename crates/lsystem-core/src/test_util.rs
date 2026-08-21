//! Shared helpers for tests that pin the `fold` fast path.

use crate::{AnyCompiledGeneration, CompiledGeneration2D, CompiledGeneration3D, GenerationConfig};
use glam::Vec3;

/// `count` directions spread roughly evenly over the sphere via a
/// golden-angle spiral, deliberately not aligned with any fixed direction
/// generator. Shared by `sphere_table`'s own coverage tests and
/// `template`'s 3D support-table tests so the two cannot drift apart.
pub(crate) fn probe_directions_3d(count: usize) -> impl Iterator<Item = Vec3> {
    let golden_angle = std::f64::consts::PI * (3.0 - 5.0_f64.sqrt());
    (0..count).map(move |index| {
        let y = 1.0 - 2.0 * (index as f64 + 0.5) / count as f64;
        let radius = (1.0 - y * y).max(0.0).sqrt();
        let theta = golden_angle * index as f64;
        Vec3::new(
            (radius * theta.cos()) as f32,
            y as f32,
            (radius * theta.sin()) as f32,
        )
    })
}

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
