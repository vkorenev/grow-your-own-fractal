use std::collections::BTreeMap;
use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use lsystem_core::{
    CompiledGrammar, Dimensions, GenerationConfig, GenerationParams, TemplateSet2D, TemplateSet3D,
    compile_generation, generate, generate_3d, generate_3d_with_topological_depth,
    generate_with_topological_depth,
};

fn checksum_2d(config: &GenerationConfig) -> f32 {
    let (grammar, params) = compile_generation(config);
    generate(&grammar, &params).fold(0.0, |acc, [a, b]| acc + a.x + a.y + b.x + b.y)
}

fn checksum_2d_with_topological_depth(config: &GenerationConfig) -> f32 {
    let (grammar, params) = compile_generation(config);
    generate_with_topological_depth(&grammar, &params).fold(0.0, |acc, segment| {
        let [a, b] = segment.points;
        acc + a.x + a.y + b.x + b.y + segment.topological_depth as f32
    })
}

fn checksum_3d(config: &GenerationConfig) -> f32 {
    let (grammar, params) = compile_generation(config);
    generate_3d(&grammar, &params).fold(0.0, |acc, [a, b]| acc + a.x + a.y + a.z + b.x + b.y + b.z)
}

fn checksum_3d_with_topological_depth(config: &GenerationConfig) -> f32 {
    let (grammar, params) = compile_generation(config);
    generate_3d_with_topological_depth(&grammar, &params).fold(0.0, |acc, segment| {
        let [a, b] = segment.points;
        acc + a.x + a.y + a.z + b.x + b.y + b.z + segment.topological_depth as f32
    })
}

/// Stamped counterparts include template building and the placement walk, so
/// they measure the full alternative pipeline, matching the interpreter
/// checksums segment for segment (modulo f32 rounding).
fn checksum_2d_stamped(config: &GenerationConfig, template_iterations: u32) -> f32 {
    let set = TemplateSet2D::build(
        CompiledGrammar::compile(config),
        GenerationParams::from(config),
        template_iterations,
    )
    .expect("template set builds");
    let mut acc = 0.0f32;
    set.emit_stamps(|stamp, template| {
        for segment in &template.segments {
            let a = stamp.pos + stamp.rot.rotate(segment.start);
            let b = stamp.pos + stamp.rot.rotate(segment.end);
            acc += a.x + a.y + b.x + b.y;
        }
    });
    acc
}

fn checksum_2d_stamped_with_topological_depth(
    config: &GenerationConfig,
    template_iterations: u32,
) -> f32 {
    let set = TemplateSet2D::build(
        CompiledGrammar::compile(config),
        GenerationParams::from(config),
        template_iterations,
    )
    .expect("template set builds");
    let mut acc = 0.0f32;
    set.emit_stamps(|stamp, template| {
        for segment in &template.segments {
            let a = stamp.pos + stamp.rot.rotate(segment.start);
            let b = stamp.pos + stamp.rot.rotate(segment.end);
            let depth = stamp.depth_base.saturating_add(segment.depth_offset);
            acc += a.x + a.y + b.x + b.y + depth as f32;
        }
    });
    acc
}

fn checksum_3d_stamped(config: &GenerationConfig, template_iterations: u32) -> f32 {
    let set = TemplateSet3D::build(
        CompiledGrammar::compile(config),
        GenerationParams::from(config),
        template_iterations,
    )
    .expect("template set builds");
    let mut acc = 0.0f32;
    set.emit_stamps(|stamp, template| {
        for segment in &template.segments {
            let a = stamp.pos + stamp.rot * segment.start;
            let b = stamp.pos + stamp.rot * segment.end;
            acc += a.x + a.y + a.z + b.x + b.y + b.z;
        }
    });
    acc
}

fn checksum_3d_stamped_with_topological_depth(
    config: &GenerationConfig,
    template_iterations: u32,
) -> f32 {
    let set = TemplateSet3D::build(
        CompiledGrammar::compile(config),
        GenerationParams::from(config),
        template_iterations,
    )
    .expect("template set builds");
    let mut acc = 0.0f32;
    set.emit_stamps(|stamp, template| {
        for segment in &template.segments {
            let a = stamp.pos + stamp.rot * segment.start;
            let b = stamp.pos + stamp.rot * segment.end;
            let depth = stamp.depth_base.saturating_add(segment.depth_offset);
            acc += a.x + a.y + a.z + b.x + b.y + b.z + depth as f32;
        }
    });
    acc
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
        group.bench_function("dragon", |b| {
            b.iter(|| black_box(checksum_2d(black_box(&config))));
        });
        group.bench_function("dragon_stamped", |b| {
            b.iter(|| black_box(checksum_2d_stamped(black_box(&config), 10)));
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
        group.bench_function("plant_a", |b| {
            b.iter(|| black_box(checksum_2d_with_topological_depth(black_box(&config))));
        });
        group.bench_function("plant_a_stamped", |b| {
            b.iter(|| {
                black_box(checksum_2d_stamped_with_topological_depth(
                    black_box(&config),
                    4,
                ))
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
        group.bench_function("synthetic_2d_forward_heavy", |b| {
            b.iter(|| black_box(checksum_2d(black_box(&config))));
        });
        group.bench_function("synthetic_2d_forward_heavy_stamped", |b| {
            b.iter(|| black_box(checksum_2d_stamped(black_box(&config), 4)));
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
        group.bench_function("branching_rotation_heavy", |b| {
            b.iter(|| black_box(checksum_3d(black_box(&config))));
        });
        group.bench_function("branching_rotation_heavy_stamped", |b| {
            b.iter(|| black_box(checksum_3d_stamped(black_box(&config), 5)));
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
        group.bench_function("hilbert_3d", |b| {
            b.iter(|| black_box(checksum_3d(black_box(&config))));
        });
        group.bench_function("hilbert_3d_stamped", |b| {
            b.iter(|| black_box(checksum_3d_stamped(black_box(&config), 3)));
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
        group.bench_function("tree_roll_heavy", |b| {
            b.iter(|| black_box(checksum_3d_with_topological_depth(black_box(&config))));
        });
        group.bench_function("tree_roll_heavy_stamped", |b| {
            b.iter(|| {
                black_box(checksum_3d_stamped_with_topological_depth(
                    black_box(&config),
                    4,
                ))
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
        group.bench_function("synthetic_3d_forward_heavy", |b| {
            b.iter(|| black_box(checksum_3d(black_box(&config))));
        });
        group.bench_function("synthetic_3d_forward_heavy_stamped", |b| {
            b.iter(|| black_box(checksum_3d_stamped(black_box(&config), 4)));
        });

        group.finish();
    }
}

criterion_group!(benches, bench_generation);
criterion_main!(benches);
