use std::collections::BTreeMap;
use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use lsystem_core::{
    Dimensions, GenerationConfig, generate, generate_3d, generate_3d_with_topological_depth,
    generate_with_topological_depth,
};

fn checksum_2d(config: &GenerationConfig) -> f32 {
    generate(config).fold(0.0, |acc, [a, b]| acc + a.x + a.y + b.x + b.y)
}

fn checksum_2d_with_topological_depth(config: &GenerationConfig) -> f32 {
    generate_with_topological_depth(config).fold(0.0, |acc, segment| {
        let [a, b] = segment.points;
        acc + a.x + a.y + b.x + b.y + segment.topological_depth as f32
    })
}

fn checksum_3d(config: &GenerationConfig) -> f32 {
    generate_3d(config).fold(0.0, |acc, [a, b]| acc + a.x + a.y + a.z + b.x + b.y + b.z)
}

fn checksum_3d_with_topological_depth(config: &GenerationConfig) -> f32 {
    generate_3d_with_topological_depth(config).fold(0.0, |acc, segment| {
        let [a, b] = segment.points;
        acc + a.x + a.y + a.z + b.x + b.y + b.z + segment.topological_depth as f32
    })
}

/// Benchmarks segment generation cost, not parsing or rendering.
///
/// Each case emphasizes a different turtle path so optimization wins and
/// regressions are easier to localize than in one broad aggregate workload.
fn bench_generation(c: &mut Criterion) {
    {
        let mut group = c.benchmark_group("generation_2d");

        // Bracketless 2D dragon stresses right-angle turns and drawn segments
        // without stack or topological depth costs, isolating common planar turtle work.
        let config = GenerationConfig {
            dimensions: Dimensions::TwoD,
            axiom: "FX".to_string(),
            iterations: 20,
            angle: 90.0,
            step: 1.0,
            initial_heading: 0.0,
            rules: BTreeMap::from([('X', "X+YF+".to_string()), ('Y', "-FX-Y".to_string())]),
        };
        group.bench_function("dragon", |b| {
            b.iter(|| black_box(checksum_2d(black_box(&config))));
        });

        // Plant-style 2D branching covers non-right-angle turns, stack traffic,
        // ignored symbols, and topological depth metadata that affect emitted segments.
        let config = GenerationConfig {
            dimensions: Dimensions::TwoD,
            axiom: "X".to_string(),
            iterations: 9,
            angle: 23.4,
            step: 1.0,
            initial_heading: 90.0,
            rules: BTreeMap::from([
                ('X', "F+[[X]-X]-F[-FX]+X".to_string()),
                ('F', "FF".to_string()),
            ]),
        };
        group.bench_function("plant_a", |b| {
            b.iter(|| black_box(checksum_2d_with_topological_depth(black_box(&config))));
        });

        // Synthetic forward-heavy 2D isolates F/f movement so regressions in
        // the most direct segment-emission path are not hidden by rotations.
        let config = GenerationConfig {
            dimensions: Dimensions::TwoD,
            axiom: "F".to_string(),
            iterations: 9,
            angle: 90.0,
            step: 1.0,
            initial_heading: 0.0,
            rules: BTreeMap::from([('F', "FFFFfF".to_string())]),
        };
        group.bench_function("synthetic_2d_forward_heavy", |b| {
            b.iter(|| black_box(checksum_2d(black_box(&config))));
        });

        group.finish();
    }

    {
        let mut group = c.benchmark_group("generation_3d");

        // Branching 3D combines yaw, pitch, and stack operations to guard
        // realistic orientation-heavy segment generation from regressions.
        let config = GenerationConfig {
            dimensions: Dimensions::ThreeD,
            axiom: "X".to_string(),
            iterations: 10,
            angle: 30.0,
            step: 1.0,
            initial_heading: 90.0,
            rules: BTreeMap::from([
                ('X', "Y[+X][-X][&X][^X]".to_string()),
                ('Y', "FFFZ".to_string()),
                ('Z', "FZ".to_string()),
            ]),
        };
        group.bench_function("branching_rotation_heavy", |b| {
            b.iter(|| black_box(checksum_3d(black_box(&config))));
        });

        // Bracketless 3D Hilbert covers all rotation axes at 90 degrees, so
        // all-axis orientation cost is visible without stack traffic.
        let config = GenerationConfig {
            dimensions: Dimensions::ThreeD,
            axiom: "X".to_string(),
            iterations: 6,
            angle: 90.0,
            step: 1.0,
            initial_heading: 0.0,
            rules: BTreeMap::from([('X', r"^\XF^\XFX-F^//XFX&F+//XFX-F/X-/".to_string())]),
        };
        group.bench_function("hilbert_3d", |b| {
            b.iter(|| black_box(checksum_3d(black_box(&config))));
        });

        // Roll-heavy 3D tree exercises stack and topological depth-aware segment output;
        // it guards roll handling because roll should not change forward motion.
        let config = GenerationConfig {
            dimensions: Dimensions::ThreeD,
            axiom: "A".to_string(),
            iterations: 9,
            angle: 40.0,
            step: 1.0,
            initial_heading: 90.0,
            rules: BTreeMap::from([
                ('A', r"F[+/A]/[-/A]F[&/A]/[^/A]".to_string()),
                ('F', "FF".to_string()),
            ]),
        };
        group.bench_function("tree_roll_heavy", |b| {
            b.iter(|| black_box(checksum_3d_with_topological_depth(black_box(&config))));
        });

        // Synthetic forward-heavy 3D isolates F/f movement so the direct 3D
        // segment-emission path has a baseline without rotation or stack noise.
        let config = GenerationConfig {
            dimensions: Dimensions::ThreeD,
            axiom: "F".to_string(),
            iterations: 9,
            angle: 30.0,
            step: 1.0,
            initial_heading: 0.0,
            rules: BTreeMap::from([('F', "FFFFfF".to_string())]),
        };
        group.bench_function("synthetic_3d_forward_heavy", |b| {
            b.iter(|| black_box(checksum_3d(black_box(&config))));
        });

        group.finish();
    }
}

criterion_group!(benches, bench_generation);
criterion_main!(benches);
