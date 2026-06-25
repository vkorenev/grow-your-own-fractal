use std::collections::BTreeMap;
use std::hint::black_box;
use std::ops::RangeInclusive;
use std::time::Duration;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use lsystem_core::{ColorConfig, Config, Dimensions, GenerationConfig, LineColorConfig, Rgb};
use lsystem_renderer::camera::Camera;
use lsystem_renderer::png_export::render_rgba;
use lsystem_renderer::wgpu_util::create_headless_device;

struct RenderBenchCase {
    fractal_name: &'static str,
    iterations: RangeInclusive<u32>,
    image_size: (u32, u32),
    colors: ColorConfig,
}

fn bench_offscreen_render(c: &mut Criterion) {
    let (device, queue) = pollster::block_on(create_headless_device(
        "offscreen_render_bench_device",
        "offscreen render benchmark",
    ))
    .expect("no GPU adapter available for offscreen render benchmark");
    let camera = Camera::default();

    let mut bench_render_case =
        |case: RenderBenchCase, generation_for_iterations: &dyn Fn(u32) -> GenerationConfig| {
            let (width, height) = case.image_size;
            let group_name = format!("offscreen_rgba_{}_{}x{}", case.fractal_name, width, height);
            let mut group = c.benchmark_group(group_name);
            group
                .sample_size(50)
                .measurement_time(Duration::from_secs(10));
            for iterations in case.iterations {
                group.bench_with_input(
                    BenchmarkId::from_parameter(iterations),
                    &iterations,
                    |b, &iterations| {
                        let config = Config {
                            name: case.fractal_name.to_string(),
                            generation: generation_for_iterations(iterations),
                            colors: case.colors,
                        };
                        b.iter(|| {
                            let export = pollster::block_on(render_rgba(
                                black_box(&device),
                                black_box(&queue),
                                black_box(&config),
                                black_box(width),
                                black_box(height),
                                black_box(&camera),
                            ))
                            .expect("offscreen RGBA render failed");
                            black_box(export.rgba.len())
                        });
                    },
                );
            }
            group.finish();
        };

    bench_render_case(
        RenderBenchCase {
            fractal_name: "harter_heighway_dragon",
            iterations: 19..=21,
            image_size: (512, 512),
            colors: ColorConfig {
                background: Rgb::new(0, 0, 0),
                line: LineColorConfig::HueCycle {
                    initial: Rgb::new(255, 255, 255),
                },
            },
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
    bench_render_case(
        RenderBenchCase {
            fractal_name: "branching_3d",
            iterations: 9..=11,
            image_size: (512, 512),
            colors: ColorConfig {
                background: Rgb::new(0, 0, 0),
                line: LineColorConfig::Gradient {
                    start: Rgb::new(89, 89, 13),
                    end: Rgb::new(51, 217, 64),
                    topological_depth: false,
                },
            },
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

criterion_group!(benches, bench_offscreen_render);
criterion_main!(benches);
