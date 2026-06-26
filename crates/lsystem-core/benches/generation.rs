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

fn bench_generation(c: &mut Criterion) {
    {
        let mut group = c.benchmark_group("generation_2d");

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

        let config = GenerationConfig {
            dimensions: Dimensions::TwoD,
            axiom: "X".to_string(),
            iterations: 9,
            angle: 23.0,
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

        let config = GenerationConfig {
            dimensions: Dimensions::TwoD,
            axiom: "F".repeat(1_000_000),
            iterations: 0,
            angle: 90.0,
            step: 1.0,
            initial_heading: 0.0,
            rules: BTreeMap::new(),
        };
        group.bench_function("synthetic_2d_forward_heavy", |b| {
            b.iter(|| black_box(checksum_2d(black_box(&config))));
        });

        group.finish();
    }

    {
        let mut group = c.benchmark_group("generation_3d");

        let branching_config = GenerationConfig {
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
            b.iter(|| black_box(checksum_3d(black_box(&branching_config))));
        });

        let hilbert_config = GenerationConfig {
            dimensions: Dimensions::ThreeD,
            axiom: "X".to_string(),
            iterations: 6,
            angle: 90.0,
            step: 1.0,
            initial_heading: 0.0,
            rules: BTreeMap::from([('X', r"^\XF^\XFX-F^//XFX&F+//XFX-F/X-/".to_string())]),
        };
        group.bench_function("hilbert_3d", |b| {
            b.iter(|| black_box(checksum_3d(black_box(&hilbert_config))));
        });

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

        let config_3d = GenerationConfig {
            dimensions: Dimensions::ThreeD,
            axiom: "F".repeat(1_000_000),
            iterations: 0,
            angle: 30.0,
            step: 1.0,
            initial_heading: 0.0,
            rules: BTreeMap::new(),
        };
        group.bench_function("synthetic_3d_forward_heavy", |b| {
            b.iter(|| black_box(checksum_3d(black_box(&config_3d))));
        });

        group.finish();
    }
}

criterion_group!(benches, bench_generation);
criterion_main!(benches);
