use wgpu::util::DeviceExt;

use crate::line_renderer::{
    Segment2D, Segment3D, TopologicalDepthSegment2D, TopologicalDepthSegment3D,
    UploadedSegmentBuffer,
};
use crate::readback::{ReadbackError, map_read_buffer};

const WORKGROUP_SIZE: u32 = 64;
const BOUNDS_3D_WORDS: usize = 6;
const DIMENSIONS_2D: u32 = 2;
const DIMENSIONS_3D: u32 = 3;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BoundsParams {
    count: u32,
    dimensions: u32,
    stride_words: u32,
    _pad: u32,
}

pub type Bounds2D = ([f32; 2], [f32; 2]);
pub type Bounds3D = ([f32; 3], [f32; 3]);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoundsComputeSupport {
    compute_shaders: bool,
}

impl BoundsComputeSupport {
    pub const fn cpu_only() -> Self {
        Self {
            compute_shaders: false,
        }
    }

    pub const fn compute_shaders() -> Self {
        Self {
            compute_shaders: true,
        }
    }

    pub fn from_adapter(adapter: &wgpu::Adapter) -> Self {
        Self {
            compute_shaders: adapter
                .get_downlevel_capabilities()
                .flags
                .contains(wgpu::DownlevelFlags::COMPUTE_SHADERS),
        }
    }

    pub const fn supports_compute_shaders(self) -> bool {
        self.compute_shaders
    }
}

pub async fn segment_bounds_2d(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    support: BoundsComputeSupport,
    segments: &[Segment2D],
    uploaded: Option<UploadedSegmentBuffer>,
) -> Result<Bounds2D, ReadbackError> {
    if segments.is_empty() {
        return Ok(fallback_bounds_2d());
    }
    if !support.supports_compute_shaders() {
        return Ok(cpu_segment_bounds_2d(segments));
    }
    compute_bounds_2d(
        device,
        queue,
        segments.len() as u32,
        DIMENSIONS_2D,
        std::mem::size_of::<Segment2D>() as u32 / 4,
        bytemuck::cast_slice(segments),
        uploaded,
    )
    .await
}

pub async fn standalone_segment_bounds_2d(segments: &[Segment2D]) -> Bounds2D {
    if segments.is_empty() {
        return fallback_bounds_2d();
    }
    match crate::wgpu_util::create_headless_device("bounds_compute_device", "bounds compute").await
    {
        Ok((device, queue, support)) => segment_bounds_2d(&device, &queue, support, segments, None)
            .await
            .unwrap_or_else(|error| {
                log::warn!("GPU 2D bounds readback failed; using CPU fallback: {error}");
                cpu_segment_bounds_2d(segments)
            }),
        Err(error) => {
            log::warn!("Failed to create GPU bounds device; using CPU fallback: {error:?}");
            cpu_segment_bounds_2d(segments)
        }
    }
}

pub async fn depth_segment_bounds_2d(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    support: BoundsComputeSupport,
    segments: &[TopologicalDepthSegment2D],
    uploaded: Option<UploadedSegmentBuffer>,
) -> Result<Bounds2D, ReadbackError> {
    if segments.is_empty() {
        return Ok(fallback_bounds_2d());
    }
    if !support.supports_compute_shaders() {
        return Ok(cpu_depth_segment_bounds_2d(segments));
    }
    compute_bounds_2d(
        device,
        queue,
        segments.len() as u32,
        DIMENSIONS_2D,
        std::mem::size_of::<TopologicalDepthSegment2D>() as u32 / 4,
        bytemuck::cast_slice(segments),
        uploaded,
    )
    .await
}

pub async fn standalone_depth_segment_bounds_2d(
    segments: &[TopologicalDepthSegment2D],
) -> Bounds2D {
    if segments.is_empty() {
        return fallback_bounds_2d();
    }
    match crate::wgpu_util::create_headless_device("bounds_compute_device", "bounds compute").await
    {
        Ok((device, queue, support)) => {
            depth_segment_bounds_2d(&device, &queue, support, segments, None)
                .await
                .unwrap_or_else(|error| {
                    log::warn!("GPU 2D depth bounds readback failed; using CPU fallback: {error}");
                    cpu_depth_segment_bounds_2d(segments)
                })
        }
        Err(error) => {
            log::warn!("Failed to create GPU bounds device; using CPU fallback: {error:?}");
            cpu_depth_segment_bounds_2d(segments)
        }
    }
}

pub async fn segment_bounds_3d(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    support: BoundsComputeSupport,
    segments: &[Segment3D],
    uploaded: Option<UploadedSegmentBuffer>,
) -> Result<Bounds3D, ReadbackError> {
    if segments.is_empty() {
        return Ok(fallback_bounds_3d());
    }
    if !support.supports_compute_shaders() {
        return Ok(cpu_segment_bounds_3d(segments));
    }
    compute_bounds_3d(
        device,
        queue,
        segments.len() as u32,
        DIMENSIONS_3D,
        std::mem::size_of::<Segment3D>() as u32 / 4,
        bytemuck::cast_slice(segments),
        uploaded,
    )
    .await
}

pub async fn standalone_segment_bounds_3d(segments: &[Segment3D]) -> Bounds3D {
    if segments.is_empty() {
        return fallback_bounds_3d();
    }
    match crate::wgpu_util::create_headless_device("bounds_compute_device", "bounds compute").await
    {
        Ok((device, queue, support)) => segment_bounds_3d(&device, &queue, support, segments, None)
            .await
            .unwrap_or_else(|error| {
                log::warn!("GPU 3D bounds readback failed; using CPU fallback: {error}");
                cpu_segment_bounds_3d(segments)
            }),
        Err(error) => {
            log::warn!("Failed to create GPU bounds device; using CPU fallback: {error:?}");
            cpu_segment_bounds_3d(segments)
        }
    }
}

pub async fn depth_segment_bounds_3d(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    support: BoundsComputeSupport,
    segments: &[TopologicalDepthSegment3D],
    uploaded: Option<UploadedSegmentBuffer>,
) -> Result<Bounds3D, ReadbackError> {
    if segments.is_empty() {
        return Ok(fallback_bounds_3d());
    }
    if !support.supports_compute_shaders() {
        return Ok(cpu_depth_segment_bounds_3d(segments));
    }
    compute_bounds_3d(
        device,
        queue,
        segments.len() as u32,
        DIMENSIONS_3D,
        std::mem::size_of::<TopologicalDepthSegment3D>() as u32 / 4,
        bytemuck::cast_slice(segments),
        uploaded,
    )
    .await
}

pub async fn standalone_depth_segment_bounds_3d(
    segments: &[TopologicalDepthSegment3D],
) -> Bounds3D {
    if segments.is_empty() {
        return fallback_bounds_3d();
    }
    match crate::wgpu_util::create_headless_device("bounds_compute_device", "bounds compute").await
    {
        Ok((device, queue, support)) => {
            depth_segment_bounds_3d(&device, &queue, support, segments, None)
                .await
                .unwrap_or_else(|error| {
                    log::warn!("GPU 3D depth bounds readback failed; using CPU fallback: {error}");
                    cpu_depth_segment_bounds_3d(segments)
                })
        }
        Err(error) => {
            log::warn!("Failed to create GPU bounds device; using CPU fallback: {error:?}");
            cpu_depth_segment_bounds_3d(segments)
        }
    }
}

async fn compute_bounds_2d(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    count: u32,
    dimensions: u32,
    stride_words: u32,
    segment_bytes: &[u8],
    uploaded: Option<UploadedSegmentBuffer>,
) -> Result<Bounds2D, ReadbackError> {
    let words = compute_bounds_words(
        device,
        queue,
        count,
        dimensions,
        stride_words,
        segment_bytes,
        uploaded,
    )
    .await?;
    Ok((
        [
            ordered_u32_to_float(words[0]),
            ordered_u32_to_float(words[1]),
        ],
        [
            ordered_u32_to_float(words[3]),
            ordered_u32_to_float(words[4]),
        ],
    ))
}

async fn compute_bounds_3d(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    count: u32,
    dimensions: u32,
    stride_words: u32,
    segment_bytes: &[u8],
    uploaded: Option<UploadedSegmentBuffer>,
) -> Result<Bounds3D, ReadbackError> {
    let words = compute_bounds_words(
        device,
        queue,
        count,
        dimensions,
        stride_words,
        segment_bytes,
        uploaded,
    )
    .await?;
    Ok((
        [
            ordered_u32_to_float(words[0]),
            ordered_u32_to_float(words[1]),
            ordered_u32_to_float(words[2]),
        ],
        [
            ordered_u32_to_float(words[3]),
            ordered_u32_to_float(words[4]),
            ordered_u32_to_float(words[5]),
        ],
    ))
}

async fn compute_bounds_words(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    count: u32,
    dimensions: u32,
    stride_words: u32,
    segment_bytes: &[u8],
    uploaded: Option<UploadedSegmentBuffer>,
) -> Result<[u32; BOUNDS_3D_WORDS], ReadbackError> {
    let count = uploaded
        .as_ref()
        .map(|uploaded| uploaded.count)
        .unwrap_or(count);
    let segment_buffer;
    let segment_buffer = match uploaded {
        Some(uploaded) => uploaded.buffer,
        None => {
            segment_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("bounds_compute_temp_segments"),
                contents: segment_bytes,
                usage: wgpu::BufferUsages::STORAGE,
            });
            segment_buffer
        }
    };

    let init_words = [u32::MAX, u32::MAX, u32::MAX, 0, 0, 0];
    let bounds_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("bounds_compute_output"),
        contents: bytemuck::cast_slice(&init_words),
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
    });
    let params = BoundsParams {
        count,
        dimensions,
        stride_words,
        _pad: 0,
    };
    let params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("bounds_compute_params"),
        contents: bytemuck::bytes_of(&params),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("bounds_compute_readback"),
        size: std::mem::size_of::<[u32; BOUNDS_3D_WORDS]>() as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("bounds_compute_shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("bounds_compute.wgsl").into()),
    });
    let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("bounds_compute_bind_group_layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("bounds_compute_bind_group"),
        layout: &bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &segment_buffer,
                    offset: 0,
                    size: wgpu::BufferSize::new(segment_bytes.len() as u64),
                }),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &bounds_buffer,
                    offset: 0,
                    size: wgpu::BufferSize::new(
                        std::mem::size_of::<[u32; BOUNDS_3D_WORDS]>() as u64
                    ),
                }),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: params_buffer.as_entire_binding(),
            },
        ],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("bounds_compute_pipeline_layout"),
        bind_group_layouts: &[Some(&bind_group_layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("bounds_compute_pipeline"),
        layout: Some(&pipeline_layout),
        module: &shader,
        entry_point: Some("bounds_segments"),
        compilation_options: Default::default(),
        cache: None,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("bounds_compute_encoder"),
    });
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("bounds_compute_pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups(count.div_ceil(WORKGROUP_SIZE), 1, 1);
    }
    encoder.copy_buffer_to_buffer(
        &bounds_buffer,
        0,
        &readback,
        0,
        std::mem::size_of::<[u32; BOUNDS_3D_WORDS]>() as u64,
    );
    queue.submit([encoder.finish()]);

    map_read_buffer(device, &readback).await?;
    let mut words = [0u32; BOUNDS_3D_WORDS];
    {
        let mapped = readback.slice(..).get_mapped_range();
        words.copy_from_slice(bytemuck::cast_slice(&mapped));
    }
    readback.unmap();
    Ok(words)
}

fn fallback_bounds_2d() -> Bounds2D {
    ([-1.0, -1.0], [1.0, 1.0])
}

fn fallback_bounds_3d() -> Bounds3D {
    ([-1.0, -1.0, -1.0], [1.0, 1.0, 1.0])
}

fn ordered_u32_to_float(u: u32) -> f32 {
    if (u & 0x8000_0000) != 0 {
        f32::from_bits(u & 0x7fff_ffff)
    } else {
        f32::from_bits(!u)
    }
}

fn cpu_segment_bounds_2d(segments: &[Segment2D]) -> Bounds2D {
    reduce_bounds_2d(
        segments
            .iter()
            .flat_map(|segment| [segment.start, segment.end]),
    )
}

fn cpu_depth_segment_bounds_2d(segments: &[TopologicalDepthSegment2D]) -> Bounds2D {
    reduce_bounds_2d(
        segments
            .iter()
            .flat_map(|segment| [segment.start, segment.end]),
    )
}

fn cpu_segment_bounds_3d(segments: &[Segment3D]) -> Bounds3D {
    reduce_bounds_3d(
        segments
            .iter()
            .flat_map(|segment| [segment.start, segment.end]),
    )
}

fn cpu_depth_segment_bounds_3d(segments: &[TopologicalDepthSegment3D]) -> Bounds3D {
    reduce_bounds_3d(
        segments
            .iter()
            .flat_map(|segment| [segment.start, segment.end]),
    )
}

fn reduce_bounds_2d(points: impl Iterator<Item = glam::Vec2>) -> Bounds2D {
    let mut min = glam::Vec2::splat(f32::INFINITY);
    let mut max = glam::Vec2::splat(f32::NEG_INFINITY);
    for point in points {
        min = min.min(point);
        max = max.max(point);
    }
    ([min.x, min.y], [max.x, max.y])
}

fn reduce_bounds_3d(points: impl Iterator<Item = glam::Vec3>) -> Bounds3D {
    let mut min = glam::Vec3::splat(f32::INFINITY);
    let mut max = glam::Vec3::splat(f32::NEG_INFINITY);
    for point in points {
        min = min.min(point);
        max = max.max(point);
    }
    ([min.x, min.y, min.z], [max.x, max.y, max.z])
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    const EPS: f32 = 1e-5;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < EPS
    }

    fn assert_bounds_2d(actual: Bounds2D, min: [f32; 2], max: [f32; 2]) {
        assert!(close(actual.0[0], min[0]), "min x: {:?}", actual);
        assert!(close(actual.0[1], min[1]), "min y: {:?}", actual);
        assert!(close(actual.1[0], max[0]), "max x: {:?}", actual);
        assert!(close(actual.1[1], max[1]), "max y: {:?}", actual);
    }

    fn assert_bounds_3d(actual: Bounds3D, min: [f32; 3], max: [f32; 3]) {
        assert!(close(actual.0[0], min[0]), "min x: {:?}", actual);
        assert!(close(actual.0[1], min[1]), "min y: {:?}", actual);
        assert!(close(actual.0[2], min[2]), "min z: {:?}", actual);
        assert!(close(actual.1[0], max[0]), "max x: {:?}", actual);
        assert!(close(actual.1[1], max[1]), "max y: {:?}", actual);
        assert!(close(actual.1[2], max[2]), "max z: {:?}", actual);
    }

    #[test]
    fn cpu_fallback_bounds_2d_cover_negative_coordinates() {
        let segments = [
            Segment2D {
                start: glam::Vec2::new(2.0, -3.0),
                end: glam::Vec2::new(-4.0, 1.0),
            },
            Segment2D {
                start: glam::Vec2::new(0.5, 6.0),
                end: glam::Vec2::new(3.0, -2.0),
            },
        ];
        assert_bounds_2d(cpu_segment_bounds_2d(&segments), [-4.0, -3.0], [3.0, 6.0]);
    }

    #[test]
    fn cpu_fallback_bounds_3d_use_packed_vertex_layout() {
        let segments = [TopologicalDepthSegment3D {
            start: glam::Vec3::new(-2.0, 3.0, -4.0),
            end: glam::Vec3::new(5.0, -6.0, 7.0),
            topological_depth: 9,
        }];
        assert_bounds_3d(
            cpu_depth_segment_bounds_3d(&segments),
            [-2.0, -6.0, -4.0],
            [5.0, 3.0, 7.0],
        );
    }

    #[test]
    fn zero_segments_use_fallback_bounds() {
        let (device, queue, support) = pollster::block_on(
            crate::wgpu_util::create_headless_device("bounds_zero_test_device", "bounds zero test"),
        )
        .expect("failed to create test device");
        let bounds = pollster::block_on(segment_bounds_2d(&device, &queue, support, &[], None))
            .expect("bounds failed");
        assert_bounds_2d(bounds, [-1.0, -1.0], [1.0, 1.0]);
    }

    #[test]
    fn gpu_bounds_2d_are_tight() {
        let segments = [
            Segment2D {
                start: glam::Vec2::new(0.0, 0.0),
                end: glam::Vec2::new(2.0, 1.0),
            },
            Segment2D {
                start: glam::Vec2::new(-1.0, 3.0),
                end: glam::Vec2::new(4.0, -2.0),
            },
        ];
        let (device, queue, support) = pollster::block_on(
            crate::wgpu_util::create_headless_device("bounds_2d_test_device", "bounds 2D test"),
        )
        .expect("failed to create test device");
        let bounds =
            pollster::block_on(segment_bounds_2d(&device, &queue, support, &segments, None))
                .expect("bounds failed");
        assert_bounds_2d(bounds, [-1.0, -2.0], [4.0, 3.0]);
    }

    #[test]
    fn gpu_bounds_3d_depth_records_are_tight() {
        let segments = [
            TopologicalDepthSegment3D {
                start: glam::Vec3::new(-2.0, 3.0, -4.0),
                end: glam::Vec3::new(5.0, -6.0, 7.0),
                topological_depth: 9,
            },
            TopologicalDepthSegment3D {
                start: glam::Vec3::new(1.0, -8.0, 2.0),
                end: glam::Vec3::new(0.0, 4.0, -9.0),
                topological_depth: 1,
            },
        ];
        let (device, queue, support) =
            pollster::block_on(crate::wgpu_util::create_headless_device(
                "bounds_3d_depth_test_device",
                "bounds 3D depth test",
            ))
            .expect("failed to create test device");
        let bounds = pollster::block_on(depth_segment_bounds_3d(
            &device, &queue, support, &segments, None,
        ))
        .expect("bounds failed");
        assert_bounds_3d(bounds, [-2.0, -8.0, -9.0], [5.0, 4.0, 7.0]);
    }
}
