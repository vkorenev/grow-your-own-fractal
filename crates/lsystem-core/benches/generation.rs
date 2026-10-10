use std::collections::BTreeMap;
use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use lsystem_core::{
    AnyCompiledGeneration, CompiledGeneration2D, CompiledGeneration3D, D2, D3, Dimensions,
    GenerationConfig, PreparedGeneration,
};

fn compile_2d(config: &GenerationConfig) -> CompiledGeneration2D {
    let AnyCompiledGeneration::TwoD(generation) = config.compile() else {
        panic!("expected a 2D generation config")
    };
    generation
}

fn compile_3d(config: &GenerationConfig) -> CompiledGeneration3D {
    let AnyCompiledGeneration::ThreeD(generation) = config.compile() else {
        panic!("expected a 3D generation config")
    };
    generation
}

fn checksum_2d(config: &GenerationConfig) -> f32 {
    compile_2d(config)
        .segments()
        .fold(0.0, |acc, [a, b]| acc + a.x + a.y + b.x + b.y)
}

fn checksum_2d_with_topological_depth(config: &GenerationConfig) -> f32 {
    compile_2d(config)
        .depth_segments()
        .fold(0.0, |acc, segment| {
            let [a, b] = segment.points;
            acc + a.x + a.y + b.x + b.y + segment.topological_depth as f32
        })
}

fn checksum_3d(config: &GenerationConfig) -> f32 {
    compile_3d(config)
        .segments()
        .fold(0.0, |acc, [a, b]| acc + a.x + a.y + a.z + b.x + b.y + b.z)
}

fn checksum_3d_with_topological_depth(config: &GenerationConfig) -> f32 {
    compile_3d(config)
        .depth_segments()
        .fold(0.0, |acc, segment| {
            let [a, b] = segment.points;
            acc + a.x + a.y + a.z + b.x + b.y + b.z + segment.topological_depth as f32
        })
}

/// Stamped counterparts use the same bounds-carrying streams as production
/// consumers, matching the interpreter checksums segment for segment (modulo
/// f32 rounding).
fn checksum_2d_stamped(prepared: &PreparedGeneration<D2>) -> f32 {
    prepared
        .segments()
        .fold(0.0, |acc, [a, b]| acc + a.x + a.y + b.x + b.y)
}

fn checksum_2d_stamped_with_topological_depth(prepared: &PreparedGeneration<D2>) -> f32 {
    prepared.depth_segments().fold(0.0, |acc, segment| {
        let [a, b] = segment.points;
        acc + a.x + a.y + b.x + b.y + segment.topological_depth as f32
    })
}

fn checksum_3d_stamped(prepared: &PreparedGeneration<D3>) -> f32 {
    prepared
        .segments()
        .fold(0.0, |acc, [a, b]| acc + a.x + a.y + a.z + b.x + b.y + b.z)
}

fn checksum_3d_stamped_with_topological_depth(prepared: &PreparedGeneration<D3>) -> f32 {
    prepared.depth_segments().fold(0.0, |acc, segment| {
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
        let config = GenerationConfig::new(
            Dimensions::TwoD,
            "FX".to_string(),
            20,
            90.0,
            1.0,
            0.0,
            BTreeMap::from([('X', "X+YF+".to_string()), ('Y', "-FX-Y".to_string())]),
        )
        .expect("balanced config");
        let stamped = PreparedGeneration::Stamped(
            compile_2d(&config)
                .build_templates(10)
                .expect("template set builds"),
        );
        group.bench_function("dragon", |b| {
            b.iter(|| black_box(checksum_2d(black_box(&config))));
        });
        group.bench_function("dragon_stamped", |b| {
            b.iter(|| black_box(checksum_2d_stamped(black_box(&stamped))));
        });

        // Plant-style 2D branching covers non-right-angle turns, stack traffic,
        // ignored symbols, and topological depth metadata that affect emitted segments.
        let config = GenerationConfig::new(
            Dimensions::TwoD,
            "X".to_string(),
            9,
            23.4,
            1.0,
            90.0,
            BTreeMap::from([
                ('X', "F+[[X]-X]-F[-FX]+X".to_string()),
                ('F', "FF".to_string()),
            ]),
        )
        .expect("balanced config");
        let stamped = PreparedGeneration::Stamped(
            compile_2d(&config)
                .build_templates(4)
                .expect("template set builds"),
        );
        group.bench_function("plant_a", |b| {
            b.iter(|| black_box(checksum_2d_with_topological_depth(black_box(&config))));
        });
        group.bench_function("plant_a_stamped", |b| {
            b.iter(|| {
                black_box(checksum_2d_stamped_with_topological_depth(black_box(
                    &stamped,
                )))
            });
        });

        // Synthetic forward-heavy 2D isolates F/f movement so regressions in
        // the most direct segment-emission path are not hidden by rotations.
        let config = GenerationConfig::new(
            Dimensions::TwoD,
            "F".to_string(),
            9,
            90.0,
            1.0,
            0.0,
            BTreeMap::from([('F', "FFFFfF".to_string())]),
        )
        .expect("balanced config");
        let stamped = PreparedGeneration::Stamped(
            compile_2d(&config)
                .build_templates(4)
                .expect("template set builds"),
        );
        group.bench_function("synthetic_2d_forward_heavy", |b| {
            b.iter(|| black_box(checksum_2d(black_box(&config))));
        });
        group.bench_function("synthetic_2d_forward_heavy_stamped", |b| {
            b.iter(|| black_box(checksum_2d_stamped(black_box(&stamped))));
        });

        group.finish();
    }

    {
        let mut group = c.benchmark_group("generation_3d");

        // Branching 3D combines yaw, pitch, and stack operations to guard
        // realistic orientation-heavy segment generation from regressions.
        let config = GenerationConfig::new(
            Dimensions::ThreeD,
            "X".to_string(),
            10,
            30.0,
            1.0,
            90.0,
            BTreeMap::from([
                ('X', "Y[+X][-X][&X][^X]".to_string()),
                ('Y', "FFFZ".to_string()),
                ('Z', "FZ".to_string()),
            ]),
        )
        .expect("balanced config");
        let stamped = PreparedGeneration::Stamped(
            compile_3d(&config)
                .build_templates(5)
                .expect("template set builds"),
        );
        group.bench_function("branching_rotation_heavy", |b| {
            b.iter(|| black_box(checksum_3d(black_box(&config))));
        });
        group.bench_function("branching_rotation_heavy_stamped", |b| {
            b.iter(|| black_box(checksum_3d_stamped(black_box(&stamped))));
        });

        // Bracketless 3D Hilbert covers all rotation axes at 90 degrees, so
        // all-axis orientation cost is visible without stack traffic.
        let config = GenerationConfig::new(
            Dimensions::ThreeD,
            "X".to_string(),
            6,
            90.0,
            1.0,
            0.0,
            BTreeMap::from([('X', r"^\XF^\XFX-F^//XFX&F+//XFX-F/X-/".to_string())]),
        )
        .expect("balanced config");
        let stamped = PreparedGeneration::Stamped(
            compile_3d(&config)
                .build_templates(3)
                .expect("template set builds"),
        );
        group.bench_function("hilbert_3d", |b| {
            b.iter(|| black_box(checksum_3d(black_box(&config))));
        });
        group.bench_function("hilbert_3d_stamped", |b| {
            b.iter(|| black_box(checksum_3d_stamped(black_box(&stamped))));
        });

        // Roll-heavy 3D tree exercises stack and topological depth-aware segment output;
        // it guards roll handling because roll should not change forward motion.
        let config = GenerationConfig::new(
            Dimensions::ThreeD,
            "A".to_string(),
            9,
            40.0,
            1.0,
            90.0,
            BTreeMap::from([
                ('A', r"F[+/A]/[-/A]F[&/A]/[^/A]".to_string()),
                ('F', "FF".to_string()),
            ]),
        )
        .expect("balanced config");
        let stamped = PreparedGeneration::Stamped(
            compile_3d(&config)
                .build_templates(4)
                .expect("template set builds"),
        );
        group.bench_function("tree_roll_heavy", |b| {
            b.iter(|| black_box(checksum_3d_with_topological_depth(black_box(&config))));
        });
        group.bench_function("tree_roll_heavy_stamped", |b| {
            b.iter(|| {
                black_box(checksum_3d_stamped_with_topological_depth(black_box(
                    &stamped,
                )))
            });
        });

        // Synthetic forward-heavy 3D isolates F/f movement so the direct 3D
        // segment-emission path has a baseline without rotation or stack noise.
        let config = GenerationConfig::new(
            Dimensions::ThreeD,
            "F".to_string(),
            9,
            30.0,
            1.0,
            0.0,
            BTreeMap::from([('F', "FFFFfF".to_string())]),
        )
        .expect("balanced config");
        let stamped = PreparedGeneration::Stamped(
            compile_3d(&config)
                .build_templates(4)
                .expect("template set builds"),
        );
        group.bench_function("synthetic_3d_forward_heavy", |b| {
            b.iter(|| black_box(checksum_3d(black_box(&config))));
        });
        group.bench_function("synthetic_3d_forward_heavy_stamped", |b| {
            b.iter(|| black_box(checksum_3d_stamped(black_box(&stamped))));
        });

        group.finish();
    }
}

criterion_group!(benches, bench_generation);
criterion_main!(benches);
