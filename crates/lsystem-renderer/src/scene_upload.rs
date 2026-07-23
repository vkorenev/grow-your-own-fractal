use std::error::Error;
use std::fmt::{Display, Formatter};

use lsystem_core::{
    BoundsAccumulator, CompiledGeneration, D2, D3, DEFAULT_TEMPLATE_SEGMENT_BUDGET,
    LineColorConfig, PreparedGeneration, SegmentWithTopologicalDepth, TemplateDimension,
};

use crate::line_renderer::{LinePipeline, RenderDimension, StagingUnavailable, record_limit};
use crate::lsystem_bridge::color_params_from_config;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SegmentLayout {
    Plain,
    TopologicalDepth,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GenerationMethod {
    Stamped { template_iterations: u16 },
    Interpreted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SceneUploadError {
    SegmentLimitExceeded { total_segments: u64, limit: u64 },
    StagingUnavailable,
}

impl Display for SceneUploadError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SegmentLimitExceeded {
                total_segments,
                limit,
            } => write!(
                f,
                "generated scene has {total_segments} segments, exceeding the {limit}-segment GPU buffer limit"
            ),
            Self::StagingUnavailable => write!(f, "GPU staging memory is unavailable"),
        }
    }
}

impl Error for SceneUploadError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UploadedLayout {
    Plain,
    TopologicalDepth { max_topological_depth: u32 },
}

impl UploadedLayout {
    pub fn max_topological_depth(self) -> Option<u32> {
        match self {
            Self::Plain => None,
            Self::TopologicalDepth {
                max_topological_depth,
            } => Some(max_topological_depth),
        }
    }
}

/// Metadata for a successful upload, including a successful empty scene.
///
/// There is deliberately no `empty` constructor: absence belongs in the
/// consumer's `Option`, because this type must always report a truthful method
/// and layout. `PartialEq` is deliberately omitted because exact equality of
/// floating-point bounds is not a useful scene invariant.
#[derive(Clone, Copy, Debug)]
pub struct UploadedScene<D: RenderDimension> {
    method: GenerationMethod,
    layout: UploadedLayout,
    total_segments: u32,
    bounds: D::Bounds,
}

pub type UploadedScene2D = UploadedScene<D2>;
pub type UploadedScene3D = UploadedScene<D3>;

impl<D: RenderDimension> UploadedScene<D> {
    pub fn method(&self) -> GenerationMethod {
        self.method
    }

    pub fn layout(&self) -> UploadedLayout {
        self.layout
    }

    pub fn total_segments(&self) -> u32 {
        self.total_segments
    }

    pub fn bounds(&self) -> D::Bounds {
        self.bounds
    }
}

fn actual_layout(requested: SegmentLayout, has_stack_directives: bool) -> SegmentLayout {
    if requested == SegmentLayout::TopologicalDepth && has_stack_directives {
        SegmentLayout::TopologicalDepth
    } else {
        SegmentLayout::Plain
    }
}

fn checked_total<D: RenderDimension>(
    total_segments: u64,
    layout: SegmentLayout,
) -> Result<u32, SceneUploadError> {
    let limit = match layout {
        SegmentLayout::Plain => record_limit::<D::PlainRecord>(),
        SegmentLayout::TopologicalDepth => record_limit::<D::DepthRecord>(),
    };
    if total_segments > limit {
        return Err(SceneUploadError::SegmentLimitExceeded {
            total_segments,
            limit,
        });
    }
    Ok(u32::try_from(total_segments).expect("segment cap fits in u32"))
}

fn upload_plain<D: RenderDimension>(
    pipeline: &mut LinePipeline<D>,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    segments: impl Iterator<Item = [D::Point; 2]>,
    line: &LineColorConfig,
    method: GenerationMethod,
    total_segments: u32,
) -> Result<UploadedScene<D>, SceneUploadError> {
    let mut bounds = D::BoundsAccumulator::default();
    pipeline
        .upload_from_iter(
            device,
            queue,
            total_segments,
            segments.map(|[start, end]| {
                bounds.include_segment(start, end);
                D::plain_record(start, end)
            }),
        )
        .map_err(|StagingUnavailable| SceneUploadError::StagingUnavailable)?;

    let bounds = bounds.finish().unwrap_or_else(D::empty_scene_bounds);
    pipeline.write_color_params(queue, color_params_from_config(line, total_segments, None));
    Ok(UploadedScene {
        method,
        layout: UploadedLayout::Plain,
        total_segments,
        bounds,
    })
}

fn upload_with_topological_depth<D: RenderDimension>(
    pipeline: &mut LinePipeline<D>,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    segments: impl Iterator<Item = SegmentWithTopologicalDepth<D>>,
    line: &LineColorConfig,
    method: GenerationMethod,
    total_segments: u32,
) -> Result<UploadedScene<D>, SceneUploadError> {
    let mut bounds = D::BoundsAccumulator::default();
    let mut max_topological_depth = 0;
    pipeline
        .upload_depth_from_iter(
            device,
            queue,
            total_segments,
            segments.map(|segment| {
                let [start, end] = segment.points;
                bounds.include_segment(start, end);
                max_topological_depth = max_topological_depth.max(segment.topological_depth);
                D::depth_record(start, end, segment.topological_depth)
            }),
        )
        .map_err(|StagingUnavailable| SceneUploadError::StagingUnavailable)?;

    let bounds = bounds.finish().unwrap_or_else(D::empty_scene_bounds);
    pipeline.write_color_params(
        queue,
        color_params_from_config(line, total_segments, Some(max_topological_depth)),
    );
    Ok(UploadedScene {
        method,
        layout: UploadedLayout::TopologicalDepth {
            max_topological_depth,
        },
        total_segments,
        bounds,
    })
}

/// Generates and uploads a scene.
///
/// A segment-limit error is returned before the pipeline is mutated, so its
/// previous scene remains drawable. A staging error clears the effective
/// (post-clamp) layout's active buffer because growth may already have replaced
/// its old contents. On success, segment data, active layout, and color
/// parameters are updated together.
///
/// The generation's dimension must match the pipeline's:
///
/// ```compile_fail,E0308
/// use lsystem_core::{CompiledGeneration2D, LineColorConfig, Rgb};
/// use lsystem_renderer::line_renderer::LinePipeline3D;
/// use lsystem_renderer::scene_upload::{SegmentLayout, upload_scene};
///
/// fn mismatched(pipeline: &mut LinePipeline3D, generation: CompiledGeneration2D) {
///     let line = LineColorConfig::Solid(Rgb::new(255, 255, 255));
///     let _ = upload_scene(
///         pipeline,
///         unreachable!(),
///         unreachable!(),
///         generation,
///         &line,
///         SegmentLayout::Plain,
///     );
/// }
/// ```
pub fn upload_scene<D>(
    pipeline: &mut LinePipeline<D>,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    generation: CompiledGeneration<D>,
    line: &LineColorConfig,
    layout: SegmentLayout,
) -> Result<UploadedScene<D>, SceneUploadError>
where
    D: RenderDimension + TemplateDimension,
{
    let plan = generation.plan_templates(DEFAULT_TEMPLATE_SEGMENT_BUDGET);
    let layout = actual_layout(layout, plan.has_stack_directives());
    let counted_total = plan.total_segments();
    let total_segments = checked_total::<D>(counted_total, layout)?;

    match plan.prepare() {
        PreparedGeneration::Stamped(set) => {
            let method = GenerationMethod::Stamped {
                template_iterations: set.template_iterations(),
            };
            match layout {
                SegmentLayout::Plain => upload_plain(
                    pipeline,
                    device,
                    queue,
                    set.segments(),
                    line,
                    method,
                    total_segments,
                ),
                SegmentLayout::TopologicalDepth => upload_with_topological_depth(
                    pipeline,
                    device,
                    queue,
                    set.depth_segments(),
                    line,
                    method,
                    total_segments,
                ),
            }
        }
        PreparedGeneration::Interpreted(generation) => match layout {
            SegmentLayout::Plain => upload_plain(
                pipeline,
                device,
                queue,
                generation.segments(),
                line,
                GenerationMethod::Interpreted,
                total_segments,
            ),
            SegmentLayout::TopologicalDepth => upload_with_topological_depth(
                pipeline,
                device,
                queue,
                generation.depth_segments(),
                line,
                GenerationMethod::Interpreted,
                total_segments,
            ),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene_upload_error_formats_both_variants() {
        assert_eq!(
            SceneUploadError::SegmentLimitExceeded {
                total_segments: 12,
                limit: 10,
            }
            .to_string(),
            "generated scene has 12 segments, exceeding the 10-segment GPU buffer limit"
        );
        assert_eq!(
            SceneUploadError::StagingUnavailable.to_string(),
            "GPU staging memory is unavailable"
        );
        assert!(SceneUploadError::StagingUnavailable.source().is_none());
    }

    #[test]
    fn bracketless_depth_requests_use_the_plain_layout() {
        assert_eq!(
            actual_layout(SegmentLayout::TopologicalDepth, false),
            SegmentLayout::Plain
        );
        assert_eq!(
            actual_layout(SegmentLayout::TopologicalDepth, true),
            SegmentLayout::TopologicalDepth
        );
        assert_eq!(UploadedLayout::Plain.max_topological_depth(), None);
        assert_eq!(
            UploadedLayout::TopologicalDepth {
                max_topological_depth: 7,
            }
            .max_topological_depth(),
            Some(7)
        );
    }
}

#[cfg(all(test, feature = "png", not(target_arch = "wasm32")))]
mod gpu_tests {
    use glam::Vec2;
    use std::collections::BTreeMap;

    use futures_channel::oneshot;
    use lsystem_core::{
        AnyCompiledGeneration, CompiledGeneration2D, CompiledGeneration3D, Dimensions,
        GenerationConfig, Rgb,
    };

    use super::*;
    use crate::line_renderer::{
        ColorParams, LinePipeline2D, LinePipeline3D, Mvp, Segment2D, TopologicalDepthSegment2D,
        Transform,
    };

    const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
    const WIDTH: u32 = 64;
    const HEIGHT: u32 = 64;

    fn generation(
        dimensions: Dimensions,
        axiom: &str,
        iterations: u16,
        rules: BTreeMap<char, String>,
    ) -> GenerationConfig {
        GenerationConfig::new(
            dimensions,
            axiom.to_string(),
            iterations,
            90.0,
            1.0,
            0.0,
            rules,
        )
        .expect("valid generation")
    }

    fn line_color() -> LineColorConfig {
        LineColorConfig::Solid(Rgb::new(255, 255, 255))
    }

    fn compile_2d(config: &GenerationConfig) -> CompiledGeneration2D {
        let AnyCompiledGeneration::TwoD(generation) = config.compile() else {
            panic!("expected 2D generation")
        };
        generation
    }

    fn compile_3d(config: &GenerationConfig) -> CompiledGeneration3D {
        let AnyCompiledGeneration::ThreeD(generation) = config.compile() else {
            panic!("expected 3D generation")
        };
        generation
    }

    /// Shared render/readback plumbing for both dimensions: `draw` lives on
    /// the generic `impl<D: RenderDimension> LinePipeline<D>`, so a single
    /// generic routine covers plain and depth layouts for `D2` and `D3`
    /// alike. `render_2d`/`render_3d` below are thin, dimension-named
    /// wrappers so existing call sites need no changes.
    async fn render<D: RenderDimension>(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipeline: &LinePipeline<D>,
    ) -> Vec<u8> {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("scene_upload_test_texture"),
            size: wgpu::Extent3d {
                width: WIDTH,
                height: HEIGHT,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let padded_bytes_per_row = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("scene_upload_test_readback"),
            size: u64::from(padded_bytes_per_row * HEIGHT),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("scene_upload_test_encoder"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene_upload_test_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });
            pipeline.draw(&mut pass);
        }
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bytes_per_row),
                    rows_per_image: None,
                },
            },
            texture.size(),
        );
        queue.submit([encoder.finish()]);

        let (sender, receiver) = oneshot::channel();
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .expect("device poll succeeds");
        receiver
            .await
            .expect("map callback runs")
            .expect("readback maps");
        let mapped = readback.slice(..).get_mapped_range();
        let mut rgba = Vec::with_capacity((WIDTH * HEIGHT * 4) as usize);
        for row in mapped
            .chunks(padded_bytes_per_row as usize)
            .take(HEIGHT as usize)
        {
            rgba.extend_from_slice(&row[..(WIDTH * 4) as usize]);
        }
        drop(mapped);
        readback.unmap();
        rgba
    }

    async fn render_2d(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipeline: &LinePipeline2D,
    ) -> Vec<u8> {
        render(device, queue, pipeline).await
    }

    async fn render_3d(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipeline: &LinePipeline3D,
    ) -> Vec<u8> {
        render(device, queue, pipeline).await
    }

    #[test]
    fn uploads_report_truthful_layout_method_bounds_and_zero_metadata() {
        pollster::block_on(async {
            let (device, queue) = crate::wgpu_util::create_headless_device(
                "scene_upload_metadata_device",
                "scene upload metadata test",
            )
            .await
            .expect("headless device");
            let mut pipeline_2d = LinePipeline2D::new(&device, FORMAT);
            let mut pipeline_3d = LinePipeline3D::new(&device, FORMAT);

            let plain = generation(Dimensions::TwoD, "F", 1, [('F', "F".to_string())].into());
            let scene = upload_scene(
                &mut pipeline_2d,
                &device,
                &queue,
                compile_2d(&plain),
                &line_color(),
                SegmentLayout::TopologicalDepth,
            )
            .expect("plain upload");
            assert_eq!(scene.layout(), UploadedLayout::Plain);
            assert_eq!(scene.total_segments(), 1);
            assert!(matches!(scene.method(), GenerationMethod::Stamped { .. }));
            assert_eq!(scene.bounds().min, glam::Vec2::ZERO);
            assert_eq!(scene.bounds().max, glam::Vec2::X);

            let depth = generation(
                Dimensions::TwoD,
                "F[+F]F",
                1,
                [('F', "F".to_string())].into(),
            );
            let scene = upload_scene(
                &mut pipeline_2d,
                &device,
                &queue,
                compile_2d(&depth),
                &line_color(),
                SegmentLayout::TopologicalDepth,
            )
            .expect("depth upload");
            assert_eq!(
                scene.layout(),
                UploadedLayout::TopologicalDepth {
                    max_topological_depth: 1,
                }
            );

            let empty = generation(Dimensions::TwoD, "", 1, BTreeMap::new());
            let scene = upload_scene(
                &mut pipeline_2d,
                &device,
                &queue,
                compile_2d(&empty),
                &line_color(),
                SegmentLayout::Plain,
            )
            .expect("empty upload");
            assert_eq!(scene.total_segments(), 0);
            assert_eq!(scene.bounds(), D2::empty_scene_bounds());
            assert!(matches!(scene.method(), GenerationMethod::Stamped { .. }));

            let interpreter = generation(
                Dimensions::TwoD,
                "A",
                1,
                [('A', "F".repeat(DEFAULT_TEMPLATE_SEGMENT_BUDGET as usize))].into(),
            );
            let scene = upload_scene(
                &mut pipeline_2d,
                &device,
                &queue,
                compile_2d(&interpreter),
                &line_color(),
                SegmentLayout::Plain,
            )
            .expect("interpreter upload");
            assert_eq!(scene.method(), GenerationMethod::Interpreted);
            assert_eq!(
                scene.total_segments(),
                DEFAULT_TEMPLATE_SEGMENT_BUDGET as u32
            );

            let interpreter_depth = generation(
                Dimensions::TwoD,
                "A",
                1,
                [(
                    'A',
                    format!("[{}]", "F".repeat(DEFAULT_TEMPLATE_SEGMENT_BUDGET as usize)),
                )]
                .into(),
            );
            let scene = upload_scene(
                &mut pipeline_2d,
                &device,
                &queue,
                compile_2d(&interpreter_depth),
                &line_color(),
                SegmentLayout::TopologicalDepth,
            )
            .expect("depth-aware interpreter upload");
            assert_eq!(scene.method(), GenerationMethod::Interpreted);
            assert_eq!(
                scene.layout(),
                UploadedLayout::TopologicalDepth {
                    max_topological_depth: DEFAULT_TEMPLATE_SEGMENT_BUDGET as u32 - 1,
                }
            );

            let plain_3d = generation(Dimensions::ThreeD, "F", 1, [('F', "F".to_string())].into());
            let scene = upload_scene(
                &mut pipeline_3d,
                &device,
                &queue,
                compile_3d(&plain_3d),
                &line_color(),
                SegmentLayout::TopologicalDepth,
            )
            .expect("3D plain upload");
            assert_eq!(scene.layout(), UploadedLayout::Plain);
            assert_eq!(scene.total_segments(), 1);
            assert!(matches!(scene.method(), GenerationMethod::Stamped { .. }));
            assert_eq!(scene.bounds().center_xz, glam::Vec2::new(0.5, 0.0));
            assert_eq!(scene.bounds().radius, 0.5);
            assert_eq!(scene.bounds().min_y, 0.0);
            assert_eq!(scene.bounds().max_y, 0.0);

            let depth_3d = generation(
                Dimensions::ThreeD,
                "F[+F]F",
                1,
                [('F', "F".to_string())].into(),
            );
            let scene = upload_scene(
                &mut pipeline_3d,
                &device,
                &queue,
                compile_3d(&depth_3d),
                &line_color(),
                SegmentLayout::TopologicalDepth,
            )
            .expect("3D depth upload");
            assert_eq!(scene.total_segments(), 3);
            assert_eq!(
                scene.layout(),
                UploadedLayout::TopologicalDepth {
                    max_topological_depth: 1,
                }
            );
            assert!(matches!(scene.method(), GenerationMethod::Stamped { .. }));

            let empty_3d = generation(Dimensions::ThreeD, "", 1, BTreeMap::new());
            let scene = upload_scene(
                &mut pipeline_3d,
                &device,
                &queue,
                compile_3d(&empty_3d),
                &line_color(),
                SegmentLayout::Plain,
            )
            .expect("empty 3D upload");
            assert_eq!(scene.total_segments(), 0);
            assert_eq!(scene.bounds(), D3::empty_scene_bounds());
        });
    }

    #[test]
    fn cap_errors_cover_stamped_and_interpreter_configs_in_both_dimensions() {
        pollster::block_on(async {
            let (device, queue) = crate::wgpu_util::create_headless_device(
                "scene_upload_cap_device",
                "scene upload cap test",
            )
            .await
            .expect("headless device");
            let mut pipeline_2d = LinePipeline2D::new(&device, FORMAT);
            let mut pipeline_3d = LinePipeline3D::new(&device, FORMAT);
            let line = line_color();

            for dimensions in [Dimensions::TwoD, Dimensions::ThreeD] {
                let stamped = generation(dimensions, "F", 30, [('F', "FF".to_string())].into());
                let interpreter = generation(
                    dimensions,
                    "A",
                    300,
                    [('A', format!("{}A", "F".repeat(65_536)))].into(),
                );
                for config in [stamped, interpreter] {
                    let error = match config.compile() {
                        AnyCompiledGeneration::TwoD(generation) => upload_scene(
                            &mut pipeline_2d,
                            &device,
                            &queue,
                            generation,
                            &line,
                            SegmentLayout::Plain,
                        )
                        .expect_err("2D config exceeds cap"),
                        AnyCompiledGeneration::ThreeD(generation) => upload_scene(
                            &mut pipeline_3d,
                            &device,
                            &queue,
                            generation,
                            &line,
                            SegmentLayout::Plain,
                        )
                        .expect_err("3D config exceeds cap"),
                    };
                    assert!(matches!(
                        error,
                        SceneUploadError::SegmentLimitExceeded { .. }
                    ));
                }
            }
        });
    }

    #[test]
    fn cap_rejection_preserves_pixels_and_staging_failure_draws_only_background() {
        pollster::block_on(async {
            let (device, queue) = crate::wgpu_util::create_headless_device(
                "scene_upload_transaction_device",
                "scene upload transaction test",
            )
            .await
            .expect("headless device");
            let mut pipeline = LinePipeline2D::new(&device, FORMAT);
            let valid = generation(Dimensions::TwoD, "F", 1, [('F', "F".to_string())].into());
            let scene = upload_scene(
                &mut pipeline,
                &device,
                &queue,
                compile_2d(&valid),
                &line_color(),
                SegmentLayout::Plain,
            )
            .expect("valid upload");
            pipeline.write_transform(
                &queue,
                crate::lsystem_bridge::viewport_transform(
                    scene.bounds(),
                    WIDTH,
                    HEIGHT,
                    Vec2::ZERO,
                    1.0,
                ),
            );
            let before = render_2d(&device, &queue, &pipeline).await;
            assert!(before.chunks_exact(4).any(|pixel| pixel != [0, 0, 0, 255]));

            let over_cap = generation(Dimensions::TwoD, "F", 30, [('F', "FF".to_string())].into());
            assert!(matches!(
                upload_scene(
                    &mut pipeline,
                    &device,
                    &queue,
                    compile_2d(&over_cap),
                    &line_color(),
                    SegmentLayout::Plain,
                ),
                Err(SceneUploadError::SegmentLimitExceeded { .. })
            ));
            let after = render_2d(&device, &queue, &pipeline).await;
            assert_eq!(after, before);

            let depth_segment = TopologicalDepthSegment2D {
                start: glam::vec2(-0.8, 0.0),
                end: glam::vec2(0.8, 0.0),
                topological_depth: 0,
            };
            pipeline.upload_with_topological_depth(
                &device,
                &queue,
                &[depth_segment],
                ColorParams::solid(1, glam::Vec4::ONE),
            );
            pipeline.write_transform(
                &queue,
                Transform {
                    scale: glam::Vec2::ONE,
                    offset: glam::Vec2::ZERO,
                },
            );

            let error_scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
            let record_size = std::mem::size_of::<Segment2D>() as u64;
            let invalid_count = (device.limits().max_buffer_size / record_size + 1) as u32;
            let result =
                pipeline.upload_from_iter(&device, &queue, invalid_count, std::iter::empty());
            assert!(matches!(result, Err(StagingUnavailable)));
            assert!(
                error_scope.pop().await.is_some(),
                "oversized buffer creation must raise a validation error"
            );

            let cleared = render_2d(&device, &queue, &pipeline).await;
            assert!(
                cleared.chunks_exact(4).all(|pixel| pixel == [0, 0, 0, 255]),
                "failed target layout must be active with zero drawable records"
            );
        });
    }

    #[test]
    fn renders_3d_scene_with_non_background_pixels_in_both_layouts() {
        pollster::block_on(async {
            let (device, queue) = crate::wgpu_util::create_headless_device(
                "scene_upload_3d_smoke_device",
                "scene upload 3D smoke test",
            )
            .await
            .expect("headless device");
            let mut pipeline = LinePipeline3D::new(&device, FORMAT);

            // `Mvp::default()` is the identity matrix: clip space equals
            // world space directly. Verified this actually places the
            // geometry on screen (not assumed): the turtle's unit-length
            // steps from these small axioms stay near the origin, well
            // inside the [-1, 1] NDC cube, so no bounds-fitting camera (as
            // `offscreen.rs`'s 3D export path builds) is needed here.
            let mvp = Mvp::default();

            // Plain layout: a 3D generation without stack directives.
            let config = generation(Dimensions::ThreeD, "F+F", 0, BTreeMap::new());
            let scene = upload_scene(
                &mut pipeline,
                &device,
                &queue,
                compile_3d(&config),
                &line_color(),
                SegmentLayout::Plain,
            )
            .expect("3D plain upload succeeds");
            assert!(scene.total_segments() > 0);
            pipeline.write_mvp(&queue, mvp);
            let pixels = render_3d(&device, &queue, &pipeline).await;
            assert!(
                pixels.chunks_exact(4).any(|pixel| pixel != [0, 0, 0, 255]),
                "3D plain draw must produce non-background pixels"
            );

            // Depth layout: a bracketed 3D generation.
            let config = generation(Dimensions::ThreeD, "F[+F]F", 0, BTreeMap::new());
            let scene = upload_scene(
                &mut pipeline,
                &device,
                &queue,
                compile_3d(&config),
                &line_color(),
                SegmentLayout::TopologicalDepth,
            )
            .expect("3D depth upload succeeds");
            assert!(scene.layout().max_topological_depth().is_some());
            pipeline.write_mvp(&queue, mvp);
            let pixels = render_3d(&device, &queue, &pipeline).await;
            assert!(
                pixels.chunks_exact(4).any(|pixel| pixel != [0, 0, 0, 255]),
                "3D depth draw must produce non-background pixels"
            );
        });
    }
}
