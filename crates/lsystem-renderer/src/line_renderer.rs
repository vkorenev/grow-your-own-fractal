use std::error::Error;
use std::fmt::{Display, Formatter};
use std::sync::Arc;

use bytemuck::{NoUninit, Pod, Zeroable};
use lsystem_core::Dimensions;
use wgpu::util::DeviceExt;

use crate::wgpu_util;

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub struct Segment2D {
    pub start: [f32; 2],
    pub end: [f32; 2],
}

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub struct Segment3D {
    pub start: [f32; 3],
    pub end: [f32; 3],
}

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub struct TopologicalDepthSegment2D {
    pub start: [f32; 2],
    pub end: [f32; 2],
    pub topological_depth: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub struct TopologicalDepthSegment3D {
    pub start: [f32; 3],
    pub end: [f32; 3],
    pub topological_depth: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub struct Transform {
    pub scale: [f32; 2],
    pub offset: [f32; 2],
}

/// Column-major MVP matrix uniform.
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub struct Mvp {
    pub matrix: [[f32; 4]; 4],
}

impl Default for Mvp {
    fn default() -> Self {
        Self {
            matrix: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        }
    }
}

const GUARANTEED_VERTEX_BUFFER_BYTES: u64 = 268_435_456;

/// Maximum number of line segments that fit in a 256 MiB vertex buffer (wgpu's guaranteed limit).
/// Each 2D segment occupies one `Segment2D` instance record.
const MAX_SEGMENTS_2D: u64 =
    GUARANTEED_VERTEX_BUFFER_BYTES / std::mem::size_of::<Segment2D>() as u64;

/// Maximum number of topological-depth 2D line segments that fit in a 256 MiB vertex buffer.
const MAX_DEPTH_SEGMENTS_2D: u64 =
    GUARANTEED_VERTEX_BUFFER_BYTES / std::mem::size_of::<TopologicalDepthSegment2D>() as u64;

/// Maximum number of 3D line segments that fit in a 256 MiB vertex buffer.
/// Each 3D segment occupies one `Segment3D` instance record.
const MAX_SEGMENTS_3D: u64 =
    GUARANTEED_VERTEX_BUFFER_BYTES / std::mem::size_of::<Segment3D>() as u64;

/// Maximum number of topological-depth 3D line segments that fit in a 256 MiB vertex buffer.
const MAX_DEPTH_SEGMENTS_3D: u64 =
    GUARANTEED_VERTEX_BUFFER_BYTES / std::mem::size_of::<TopologicalDepthSegment3D>() as u64;

/// Returns the segment cap appropriate for the given dimensions.
pub fn max_segments_for(dimensions: Dimensions) -> u64 {
    match dimensions {
        Dimensions::ThreeD => MAX_SEGMENTS_3D,
        Dimensions::TwoD => MAX_SEGMENTS_2D,
    }
}

/// Returns the segment cap appropriate for the dimensions and whether topological depth is used.
pub fn max_segments_for_line_color(dimensions: Dimensions, uses_topological_depth: bool) -> u64 {
    match dimensions {
        Dimensions::TwoD if uses_topological_depth => MAX_DEPTH_SEGMENTS_2D,
        Dimensions::ThreeD if uses_topological_depth => MAX_DEPTH_SEGMENTS_3D,
        _ => max_segments_for(dimensions),
    }
}

/// Discriminant values are matched by literal in `shader.wgsl` and `shader3d.wgsl`;
/// keep them in sync when adding or renumbering variants.
#[repr(u32)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, NoUninit, Zeroable)]
pub enum ColorMode {
    #[default]
    Solid = 0,
    Gradient = 1,
    HueCycle = 2,
    DepthGradient = 3,
}

/// Per-frame color parameters written to the GPU as a uniform.
/// Layout mirrors `ColorParams` in `shader.wgsl`; padding keeps vec4 alignment.
#[repr(C)]
#[derive(Copy, Clone, Default, NoUninit, Zeroable)]
pub struct ColorParams {
    pub mode: ColorMode,
    pub total_segments: u32,
    pub max_topological_depth: u32,
    pub _pad: u32,
    pub color_start: [f32; 4],
    pub color_end: [f32; 4],
    pub hue_start: f32,
    pub saturation: f32,
    pub value: f32,
    pub _pad2: f32,
}

impl ColorParams {
    /// Applies a hue offset only for `HueCycle`; other color modes are unchanged.
    pub fn with_hue_offset_degrees(mut self, offset: f32) -> Self {
        if self.mode == ColorMode::HueCycle {
            self.hue_start = (self.hue_start + offset).rem_euclid(360.0);
        }
        self
    }
}

struct GrowableVertexBuffer {
    buffer: Option<wgpu::Buffer>,
    capacity: u64,
    count: u32,
}

impl GrowableVertexBuffer {
    fn new() -> Self {
        Self {
            buffer: None,
            capacity: 0,
            count: 0,
        }
    }

    fn upload<V: bytemuck::Pod>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        segments: &[V],
        label: &str,
    ) {
        let required = std::mem::size_of_val(segments) as u64;
        if required > 0 {
            if self.capacity < required {
                self.capacity = required.next_power_of_two();
                self.buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(label),
                    size: self.capacity,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }));
            }
            if let Some(buffer) = &self.buffer {
                queue.write_buffer(buffer, 0, bytemuck::cast_slice(segments));
            }
        }
        self.count = segments.len() as u32;
    }
}

#[derive(Clone, Copy)]
enum ActiveSegmentBuffer {
    Normal,
    TopologicalDepth,
}

fn draw_line_list(
    render_pass: &mut wgpu::RenderPass<'_>,
    pipeline: &wgpu::RenderPipeline,
    bind_group: &wgpu::BindGroup,
    vertex_buffer: &GrowableVertexBuffer,
    debug_label: &str,
) {
    render_pass.push_debug_group(debug_label);
    render_pass.set_pipeline(pipeline);
    render_pass.set_bind_group(0, bind_group, &[]);
    if vertex_buffer.count > 0
        && let Some(buf) = &vertex_buffer.buffer
    {
        render_pass.set_vertex_buffer(0, buf.slice(..));
        render_pass.draw(0..2, 0..vertex_buffer.count);
    }
    render_pass.pop_debug_group();
}

struct LinePipelineDescriptor<'a> {
    format: wgpu::TextureFormat,
    label: &'static str,
    vertex_entry_point: &'static str,
    array_stride: wgpu::BufferAddress,
    attributes: &'a [wgpu::VertexAttribute],
}

fn create_line_pipeline(
    device: &wgpu::Device,
    pipeline_layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    descriptor: LinePipelineDescriptor<'_>,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(descriptor.label),
        layout: Some(pipeline_layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some(descriptor.vertex_entry_point),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: descriptor.array_stride,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: descriptor.attributes,
            }],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format: descriptor.format,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::LineList,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

pub struct LinePipeline2D {
    pipeline: wgpu::RenderPipeline,
    depth_pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    color_params_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    segment_buffer: GrowableVertexBuffer,
    depth_segment_buffer: GrowableVertexBuffer,
    active_segment_buffer: ActiveSegmentBuffer,
}

impl LinePipeline2D {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("lsystem_2d_shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("lsystem_2d_transform_uniform"),
            contents: bytemuck::bytes_of(&Transform {
                scale: [1.0, 1.0],
                offset: [0.0, 0.0],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let color_params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("lsystem_2d_color_uniform"),
            contents: bytemuck::bytes_of(&ColorParams::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lsystem_2d_bind_group_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(
                            std::mem::size_of::<Transform>() as u64
                        ),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(
                            std::mem::size_of::<ColorParams>() as u64
                        ),
                    },
                    count: None,
                },
            ],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lsystem_2d_bind_group"),
            layout: &bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: color_params_buffer.as_entire_binding(),
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("lsystem_2d_pipeline_layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });

        let normal_attrs = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2];
        let depth_attrs = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Uint32];
        let pipeline = create_line_pipeline(
            device,
            &pipeline_layout,
            &shader,
            LinePipelineDescriptor {
                format,
                label: "lsystem_2d_pipeline",
                vertex_entry_point: "vs_main",
                array_stride: std::mem::size_of::<Segment2D>() as wgpu::BufferAddress,
                attributes: &normal_attrs,
            },
        );
        let depth_pipeline = create_line_pipeline(
            device,
            &pipeline_layout,
            &shader,
            LinePipelineDescriptor {
                format,
                label: "lsystem_2d_depth_pipeline",
                vertex_entry_point: "vs_depth_main",
                array_stride: std::mem::size_of::<TopologicalDepthSegment2D>()
                    as wgpu::BufferAddress,
                attributes: &depth_attrs,
            },
        );

        Self {
            pipeline,
            depth_pipeline,
            uniform_buffer,
            color_params_buffer,
            bind_group,
            segment_buffer: GrowableVertexBuffer::new(),
            depth_segment_buffer: GrowableVertexBuffer::new(),
            active_segment_buffer: ActiveSegmentBuffer::Normal,
        }
    }

    pub fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        segments: &[Segment2D],
        color_params: ColorParams,
    ) {
        self.segment_buffer
            .upload(device, queue, segments, "lsystem_2d_segments");
        self.active_segment_buffer = ActiveSegmentBuffer::Normal;
        queue.write_buffer(
            &self.color_params_buffer,
            0,
            bytemuck::bytes_of(&color_params),
        );
    }

    pub fn upload_with_topological_depth(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        segments: &[TopologicalDepthSegment2D],
        color_params: ColorParams,
    ) {
        self.depth_segment_buffer.upload(
            device,
            queue,
            segments,
            "lsystem_2d_topological_depth_segments",
        );
        self.active_segment_buffer = ActiveSegmentBuffer::TopologicalDepth;
        queue.write_buffer(
            &self.color_params_buffer,
            0,
            bytemuck::bytes_of(&color_params),
        );
    }

    pub fn write_transform(&self, queue: &wgpu::Queue, transform: Transform) {
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&transform));
    }

    pub fn write_color_params(&self, queue: &wgpu::Queue, color_params: ColorParams) {
        queue.write_buffer(
            &self.color_params_buffer,
            0,
            bytemuck::bytes_of(&color_params),
        );
    }

    pub fn draw(&self, render_pass: &mut wgpu::RenderPass<'_>) {
        match self.active_segment_buffer {
            ActiveSegmentBuffer::Normal => draw_line_list(
                render_pass,
                &self.pipeline,
                &self.bind_group,
                &self.segment_buffer,
                "lsystem_2d_line_draw",
            ),
            ActiveSegmentBuffer::TopologicalDepth => draw_line_list(
                render_pass,
                &self.depth_pipeline,
                &self.bind_group,
                &self.depth_segment_buffer,
                "lsystem_2d_depth_line_draw",
            ),
        }
    }
}

pub struct LinePipeline3D {
    pipeline: wgpu::RenderPipeline,
    depth_pipeline: wgpu::RenderPipeline,
    mvp_buffer: wgpu::Buffer,
    color_params_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    segment_buffer: GrowableVertexBuffer,
    depth_segment_buffer: GrowableVertexBuffer,
    active_segment_buffer: ActiveSegmentBuffer,
}

impl LinePipeline3D {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("lsystem_3d_shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader3d.wgsl").into()),
        });

        let mvp_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("lsystem_3d_mvp_uniform"),
            contents: bytemuck::bytes_of(&Mvp::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let color_params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("lsystem_3d_color_uniform"),
            contents: bytemuck::bytes_of(&ColorParams::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lsystem_3d_bind_group_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<Mvp>() as u64),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(
                            std::mem::size_of::<ColorParams>() as u64
                        ),
                    },
                    count: None,
                },
            ],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lsystem_3d_bind_group"),
            layout: &bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: mvp_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: color_params_buffer.as_entire_binding(),
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("lsystem_3d_pipeline_layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });

        let normal_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];
        let depth_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Uint32];
        let pipeline = create_line_pipeline(
            device,
            &pipeline_layout,
            &shader,
            LinePipelineDescriptor {
                format,
                label: "lsystem_3d_pipeline",
                vertex_entry_point: "vs_main",
                array_stride: std::mem::size_of::<Segment3D>() as wgpu::BufferAddress,
                attributes: &normal_attrs,
            },
        );
        let depth_pipeline = create_line_pipeline(
            device,
            &pipeline_layout,
            &shader,
            LinePipelineDescriptor {
                format,
                label: "lsystem_3d_depth_pipeline",
                vertex_entry_point: "vs_depth_main",
                array_stride: std::mem::size_of::<TopologicalDepthSegment3D>()
                    as wgpu::BufferAddress,
                attributes: &depth_attrs,
            },
        );

        Self {
            pipeline,
            depth_pipeline,
            mvp_buffer,
            color_params_buffer,
            bind_group,
            segment_buffer: GrowableVertexBuffer::new(),
            depth_segment_buffer: GrowableVertexBuffer::new(),
            active_segment_buffer: ActiveSegmentBuffer::Normal,
        }
    }

    pub fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        segments: &[Segment3D],
        color_params: ColorParams,
    ) {
        self.segment_buffer
            .upload(device, queue, segments, "lsystem_3d_segments");
        self.active_segment_buffer = ActiveSegmentBuffer::Normal;
        queue.write_buffer(
            &self.color_params_buffer,
            0,
            bytemuck::bytes_of(&color_params),
        );
    }

    pub fn upload_with_topological_depth(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        segments: &[TopologicalDepthSegment3D],
        color_params: ColorParams,
    ) {
        self.depth_segment_buffer.upload(
            device,
            queue,
            segments,
            "lsystem_3d_topological_depth_segments",
        );
        self.active_segment_buffer = ActiveSegmentBuffer::TopologicalDepth;
        queue.write_buffer(
            &self.color_params_buffer,
            0,
            bytemuck::bytes_of(&color_params),
        );
    }

    pub fn write_mvp(&self, queue: &wgpu::Queue, mvp: Mvp) {
        queue.write_buffer(&self.mvp_buffer, 0, bytemuck::bytes_of(&mvp));
    }

    pub fn write_color_params(&self, queue: &wgpu::Queue, color_params: ColorParams) {
        queue.write_buffer(
            &self.color_params_buffer,
            0,
            bytemuck::bytes_of(&color_params),
        );
    }

    pub fn draw(&self, render_pass: &mut wgpu::RenderPass<'_>) {
        match self.active_segment_buffer {
            ActiveSegmentBuffer::Normal => draw_line_list(
                render_pass,
                &self.pipeline,
                &self.bind_group,
                &self.segment_buffer,
                "lsystem_3d_line_draw",
            ),
            ActiveSegmentBuffer::TopologicalDepth => draw_line_list(
                render_pass,
                &self.depth_pipeline,
                &self.bind_group,
                &self.depth_segment_buffer,
                "lsystem_3d_depth_line_draw",
            ),
        }
    }
}

pub struct SurfaceFrame {
    pub frame: Box<wgpu::SurfaceTexture>,
    pub view: wgpu::TextureView,
    pub encoder: wgpu::CommandEncoder,
    pub reconfigure_after_present: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameSkipReason {
    Timeout,
    Occluded,
    Validation,
    RepeatedOutdated,
}

impl Display for FrameSkipReason {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Timeout => write!(f, "surface acquisition timed out"),
            Self::Occluded => write!(f, "surface is occluded"),
            Self::Validation => write!(f, "surface acquisition hit a validation error"),
            Self::RepeatedOutdated => write!(f, "surface remained outdated after reconfigure"),
        }
    }
}

pub enum FrameOutcome {
    Ready(SurfaceFrame),
    SurfaceLost,
    Skipped(FrameSkipReason),
}

#[derive(Debug)]
pub enum GpuInitError {
    CreateSurface(wgpu::CreateSurfaceError),
    RequestAdapter(wgpu::RequestAdapterError),
    RequestDevice(wgpu::RequestDeviceError),
    NoSurfaceConfig,
}

impl Display for GpuInitError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CreateSurface(err) => write!(f, "failed to create GPU surface: {err}"),
            Self::RequestAdapter(err) => write!(f, "failed to request GPU adapter: {err}"),
            Self::RequestDevice(err) => write!(f, "failed to request GPU device: {err}"),
            Self::NoSurfaceConfig => write!(f, "GPU surface has no supported texture formats"),
        }
    }
}

impl Error for GpuInitError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::CreateSurface(err) => Some(err),
            Self::RequestAdapter(err) => Some(err),
            Self::RequestDevice(err) => Some(err),
            Self::NoSurfaceConfig => None,
        }
    }
}

pub struct GpuContext {
    surface: wgpu::Surface<'static>,
    #[allow(clippy::arc_with_non_send_sync)]
    pub device: Arc<wgpu::Device>,
    #[allow(clippy::arc_with_non_send_sync)]
    pub queue: Arc<wgpu::Queue>,
    surface_config: wgpu::SurfaceConfiguration,
}

impl GpuContext {
    pub async fn new(
        target: impl Into<wgpu::SurfaceTarget<'static>>,
        width: u32,
        height: u32,
    ) -> Result<Self, GpuInitError> {
        let instance = wgpu_util::new_instance().await;
        let surface = instance
            .create_surface(target)
            .map_err(GpuInitError::CreateSurface)?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .map_err(GpuInitError::RequestAdapter)?;
        let adapter_info = adapter.get_info();
        log::info!(
            "Selected surface GPU adapter: {} ({})",
            adapter_info.name,
            adapter_info.backend
        );
        let (device, queue) = adapter
            .request_device(&wgpu_util::device_descriptor(
                "lsystem_surface_device",
                &adapter,
            ))
            .await
            .map_err(GpuInitError::RequestDevice)?;
        wgpu_util::install_uncaptured_error_handler(&device, "surface renderer");

        #[allow(clippy::arc_with_non_send_sync)]
        let device = Arc::new(device);
        #[allow(clippy::arc_with_non_send_sync)]
        let queue = Arc::new(queue);

        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .find(|f| f.is_srgb())
            .copied()
            .or_else(|| caps.formats.first().copied())
            .ok_or(GpuInitError::NoSurfaceConfig)?;

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: width.max(1),
            height: height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &surface_config);

        Ok(Self {
            surface,
            device,
            queue,
            surface_config,
        })
    }

    pub fn surface_format(&self) -> wgpu::TextureFormat {
        self.surface_config.format
    }

    pub fn size(&self) -> (u32, u32) {
        (self.surface_config.width, self.surface_config.height)
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.surface_config.width = width;
        self.surface_config.height = height;
        self.surface.configure(&self.device, &self.surface_config);
    }

    pub fn begin_frame(&mut self) -> FrameOutcome {
        let mut retried_after_outdated = false;
        let (frame, reconfigure_after) = loop {
            match self.surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(texture) => {
                    break (texture, false);
                }
                wgpu::CurrentSurfaceTexture::Suboptimal(texture) => {
                    break (texture, true);
                }
                wgpu::CurrentSurfaceTexture::Outdated => {
                    if retried_after_outdated {
                        return FrameOutcome::Skipped(FrameSkipReason::RepeatedOutdated);
                    }
                    self.surface.configure(&self.device, &self.surface_config);
                    retried_after_outdated = true;
                }
                wgpu::CurrentSurfaceTexture::Lost => return FrameOutcome::SurfaceLost,
                wgpu::CurrentSurfaceTexture::Timeout => {
                    return FrameOutcome::Skipped(FrameSkipReason::Timeout);
                }
                wgpu::CurrentSurfaceTexture::Occluded => {
                    return FrameOutcome::Skipped(FrameSkipReason::Occluded);
                }
                wgpu::CurrentSurfaceTexture::Validation => {
                    return FrameOutcome::Skipped(FrameSkipReason::Validation);
                }
            }
        };

        let view = frame.texture.create_view(&Default::default());
        let encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("lsystem_surface_encoder"),
            });
        // The surface is cleared by the caller's render pass (LoadOp::Clear).
        FrameOutcome::Ready(SurfaceFrame {
            frame: Box::new(frame),
            view,
            encoder,
            reconfigure_after_present: reconfigure_after,
        })
    }

    pub fn end_frame(
        &self,
        frame: wgpu::SurfaceTexture,
        encoder: wgpu::CommandEncoder,
        reconfigure_after: bool,
    ) {
        self.queue.submit([encoder.finish()]);
        frame.present();
        if reconfigure_after {
            self.surface.configure(&self.device, &self.surface_config);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_params(mode: ColorMode, hue_start: f32) -> ColorParams {
        ColorParams {
            mode,
            total_segments: 10,
            max_topological_depth: 3,
            color_start: [0.1, 0.2, 0.3, 1.0],
            color_end: [0.7, 0.8, 0.9, 1.0],
            hue_start,
            saturation: 0.5,
            value: 0.75,
            ..Default::default()
        }
    }

    #[test]
    fn depth_gradient_segment_caps_use_depth_record_sizes() {
        assert_eq!(
            max_segments_for_line_color(Dimensions::TwoD, true),
            GUARANTEED_VERTEX_BUFFER_BYTES
                / std::mem::size_of::<TopologicalDepthSegment2D>() as u64
        );
        assert_eq!(
            max_segments_for_line_color(Dimensions::ThreeD, true),
            GUARANTEED_VERTEX_BUFFER_BYTES
                / std::mem::size_of::<TopologicalDepthSegment3D>() as u64
        );
        assert_eq!(
            max_segments_for_line_color(Dimensions::TwoD, false),
            max_segments_for(Dimensions::TwoD)
        );
        assert_eq!(
            max_segments_for_line_color(Dimensions::ThreeD, false),
            max_segments_for(Dimensions::ThreeD)
        );
    }

    #[test]
    fn hue_offset_shifts_hue_cycle_start() {
        let params = sample_params(ColorMode::HueCycle, 180.0);

        let shifted = params.with_hue_offset_degrees(15.0);

        assert_eq!(shifted.mode, ColorMode::HueCycle);
        assert_eq!(shifted.hue_start, 195.0);
        assert_eq!(shifted.total_segments, params.total_segments);
        assert_eq!(shifted.saturation, params.saturation);
        assert_eq!(shifted.value, params.value);
    }

    #[test]
    fn hue_offset_wraps_positive_and_negative_offsets() {
        let params = sample_params(ColorMode::HueCycle, 350.0);

        assert_eq!(params.with_hue_offset_degrees(20.0).hue_start, 10.0);
        assert_eq!(params.with_hue_offset_degrees(-370.0).hue_start, 340.0);
    }

    #[test]
    fn hue_offset_leaves_non_hue_cycle_params_unchanged() {
        for mode in [
            ColorMode::Solid,
            ColorMode::Gradient,
            ColorMode::DepthGradient,
        ] {
            let params = sample_params(mode, 123.0);
            let shifted = params.with_hue_offset_degrees(45.0);

            assert_eq!(shifted.mode, params.mode);
            assert_eq!(shifted.total_segments, params.total_segments);
            assert_eq!(shifted.max_topological_depth, params.max_topological_depth);
            assert_eq!(shifted.color_start, params.color_start);
            assert_eq!(shifted.color_end, params.color_end);
            assert_eq!(shifted.hue_start, params.hue_start);
            assert_eq!(shifted.saturation, params.saturation);
            assert_eq!(shifted.value, params.value);
        }
    }
}
