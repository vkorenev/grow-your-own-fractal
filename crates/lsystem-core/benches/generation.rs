use std::collections::BTreeMap;
use std::hint::black_box;
use std::ops::RangeInclusive;
use std::time::Duration;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use lsystem_core::{Dimensions, GenerationConfig, generate, generate_3d};

struct GenerationBenchCase {
    fractal_name: &'static str,
    iterations: RangeInclusive<u32>,
}

fn bench_generation(c: &mut Criterion) {
    let mut bench_generation_case =
        |case: GenerationBenchCase, generation_for_iterations: &dyn Fn(u32) -> GenerationConfig| {
            let group_name = format!("generate_{}", case.fractal_name);
            let mut group = c.benchmark_group(group_name);
            group
                .sample_size(50)
                .measurement_time(Duration::from_secs(10));
            for iterations in case.iterations {
                group.bench_with_input(
                    BenchmarkId::from_parameter(iterations),
                    &iterations,
                    |b, &iterations| {
                        let config = generation_for_iterations(iterations);
                        match config.dimensions {
                            Dimensions::TwoD => {
                                b.iter(|| black_box(generate(black_box(&config)).count()));
                            }
                            Dimensions::ThreeD => {
                                b.iter(|| black_box(generate_3d(black_box(&config)).count()));
                            }
                        }
                    },
                );
            }
            group.finish();
        };

    bench_generation_case(
        GenerationBenchCase {
            fractal_name: "harter_heighway_dragon",
            iterations: 19..=21,
        },
        &|iterations| GenerationConfig {
            dimensions: Dimensions::TwoD,
            axiom: "FX".to_string(),
            iterations,
            angle: 90.0,
            step: 1.0,
            initial_heading: 0.0,
            rules: BTreeMap::from([('X', "X+YF+".to_string()), ('Y', "-FX-Y".to_string())]),
        },
    );
    bench_generation_case(
        GenerationBenchCase {
            fractal_name: "branching_3d",
            iterations: 9..=11,
        },
        &|iterations| GenerationConfig {
            dimensions: Dimensions::ThreeD,
            axiom: "X".to_string(),
            iterations,
            angle: 30.0,
            step: 1.0,
            initial_heading: 90.0,
            rules: BTreeMap::from([
                ('X', "Y[+X][-X][&X][^X]".to_string()),
                ('Y', "FFFZ".to_string()),
                ('Z', "FZ".to_string()),
            ]),
        },
    );
}

criterion_group!(benches, bench_generation);
criterion_main!(benches);
