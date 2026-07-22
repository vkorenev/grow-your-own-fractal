use std::collections::BTreeMap;
use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use lsystem_core::{ColorConfig, Config, Dimensions, GenerationConfig, LineColorConfig, Rgb};
use lsystem_renderer::camera::Camera;
use lsystem_renderer::png_export::render_rgba;
use lsystem_renderer::wgpu_util::create_headless_device;

const IMAGE_SIZE: (u32, u32) = (512, 512);

fn bench_offscreen_render(c: &mut Criterion) {
    let (device, queue) = pollster::block_on(create_headless_device(
        "offscreen_render_bench_device",
        "offscreen render benchmark",
    ))
    .expect("no GPU adapter available for offscreen render benchmark");
    let camera = Camera::default();

    let configs = [
        Config {
            name: "harter_heighway_dragon".to_string(),
            generation: GenerationConfig::new(
                Dimensions::TwoD,
                "FX".to_string(),
                20,
                90.0,
                1.0,
                0.0,
                BTreeMap::from([('X', "X+YF+".to_string()), ('Y', "-FX-Y".to_string())]),
            )
            .expect("balanced config"),
            colors: ColorConfig {
                background: Rgb::new(0, 0, 0),
                line: LineColorConfig::HueCycle {
                    initial: Rgb::new(255, 255, 255),
                },
            },
        },
        Config {
            name: "plant_a".to_string(),
            generation: GenerationConfig::new(
                Dimensions::TwoD,
                "X".to_string(),
                10,
                23.4,
                1.0,
                90.0,
                BTreeMap::from([
                    ('X', "F+[[X]-X]-F[-FX]+X".to_string()),
                    ('F', "FF".to_string()),
                ]),
            )
            .expect("balanced config"),
            colors: ColorConfig {
                background: Rgb::new(0, 0, 0),
                line: LineColorConfig::Gradient {
                    start: Rgb::new(13, 89, 13),
                    end: Rgb::new(153, 230, 26),
                    topological_depth: true,
                },
            },
        },
        Config {
            name: "branching_3d".to_string(),
            generation: GenerationConfig::new(
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
            .expect("balanced config"),
            colors: ColorConfig {
                background: Rgb::new(0, 0, 0),
                line: LineColorConfig::Gradient {
                    start: Rgb::new(89, 89, 13),
                    end: Rgb::new(51, 217, 64),
                    topological_depth: false,
                },
            },
        },
        Config {
            name: "hilbert_3d".to_string(),
            generation: GenerationConfig::new(
                Dimensions::ThreeD,
                "X".to_string(),
                6,
                90.0,
                1.0,
                0.0,
                BTreeMap::from([('X', r"^\XF^\XFX-F^//XFX&F+//XFX-F/X-/".to_string())]),
            )
            .expect("balanced config"),
            colors: ColorConfig {
                background: Rgb::new(0, 0, 0),
                line: LineColorConfig::HueCycle {
                    initial: Rgb::new(255, 255, 255),
                },
            },
        },
    ];

    let (width, height) = IMAGE_SIZE;
    for config in configs {
        let mut group = c.benchmark_group(&config.name);
        group.bench_function("render", |b| {
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
        });
        group.finish();
    }
}

criterion_group!(benches, bench_offscreen_render);
criterion_main!(benches);
