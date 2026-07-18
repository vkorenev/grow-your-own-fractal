use std::error::Error;
use std::fmt::{Display, Formatter};
use std::marker::PhantomData;
use std::sync::Arc;

use encase::UniformBuffer;
use lsystem_core::{D2, D3, Dimension, Dimensions};
use wgpu::util::DeviceExt;

pub use crate::generated_shader_2d::{
    ColorParams, Segment2D, TopologicalDepthSegment2D, Transform,
};
pub use crate::generated_shader_3d::{Mvp, Segment3D, TopologicalDepthSegment3D};

use crate::generated_shader_2d;
use crate::generated_shader_3d;
use crate::lsystem_bridge::BoundsPoint;
use crate::wgpu_util;

impl Default for Mvp {
    fn default() -> Self {
        Self {
            matrix: glam::Mat4::IDENTITY,
        }
    }
}

/// Maximum number of records of `R` that fit in the platform-selected wgpu max buffer size.
pub(crate) const fn record_limit<R>() -> u64 {
    wgpu_util::MAX_BUFFER_SIZE_BYTES / std::mem::size_of::<R>() as u64
}

/// Returns the segment cap appropriate for the given dimensions.
pub fn max_segments_for(dimensions: Dimensions) -> u64 {
    match dimensions {
        Dimensions::ThreeD => record_limit::<Segment3D>(),
        Dimensions::TwoD => record_limit::<Segment2D>(),
    }
}

/// Returns the segment cap appropriate for the dimensions and whether topological depth is used.
pub fn max_segments_for_line_color(dimensions: Dimensions, uses_topological_depth: bool) -> u64 {
    match dimensions {
        Dimensions::TwoD if uses_topological_depth => record_limit::<TopologicalDepthSegment2D>(),
        Dimensions::ThreeD if uses_topological_depth => record_limit::<TopologicalDepthSegment3D>(),
        _ => max_segments_for(dimensions),
    }
}

/// Renderer capabilities selected by a type-level spatial dimension.
pub trait RenderDimension: Dimension<Point: BoundsPoint> {
    type PlainRecord: bytemuck::Pod;
    type DepthRecord: bytemuck::Pod;

    fn rotate(rotation: Self::Rotation, point: Self::Point) -> Self::Point;
    fn plain_record(start: Self::Point, end: Self::Point) -> Self::PlainRecord;
    fn depth_record(
        start: Self::Point,
        end: Self::Point,
        topological_depth: u32,
    ) -> Self::DepthRecord;
}

impl RenderDimension for D2 {
    type PlainRecord = Segment2D;
    type DepthRecord = TopologicalDepthSegment2D;

    fn rotate(rotation: Self::Rotation, point: Self::Point) -> Self::Point {
        rotation.rotate(point)
    }

    fn plain_record(start: Self::Point, end: Self::Point) -> Self::PlainRecord {
        Segment2D { start, end }
    }

    fn depth_record(
        start: Self::Point,
        end: Self::Point,
        topological_depth: u32,
    ) -> Self::DepthRecord {
        TopologicalDepthSegment2D {
            start,
            end,
            topological_depth,
        }
    }
}

impl RenderDimension for D3 {
    type PlainRecord = Segment3D;
    type DepthRecord = TopologicalDepthSegment3D;

    fn rotate(rotation: Self::Rotation, point: Self::Point) -> Self::Point {
        rotation * point
    }

    fn plain_record(start: Self::Point, end: Self::Point) -> Self::PlainRecord {
        Segment3D { start, end }
    }

    fn depth_record(
        start: Self::Point,
        end: Self::Point,
        topological_depth: u32,
    ) -> Self::DepthRecord {
        TopologicalDepthSegment3D {
            start,
            end,
            topological_depth,
        }
    }
}

/// Discriminant values are matched by literal in `shaders/common.wesl`;
/// keep them in sync when adding or renumbering variants.
#[repr(u32)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum ColorMode {
    #[default]
    Solid = 0,
    Gradient = 1,
    HueCycle = 2,
    DepthGradient = 3,
}

impl Default for ColorParams {
    fn default() -> Self {
        Self {
            mode: ColorMode::Solid as u32,
            total_segments: 0,
            max_topological_depth: 0,
            color_start: glam::Vec4::ZERO,
            color_end: glam::Vec4::ZERO,
            hue_start: 0.0,
            saturation: 0.0,
            value: 0.0,
        }
    }
}

impl ColorParams {
    pub fn solid(total_segments: u32, color_start: glam::Vec4) -> Self {
        Self {
            mode: ColorMode::Solid as u32,
            total_segments,
            color_start,
            ..Default::default()
        }
    }

    pub fn gradient(total_segments: u32, color_start: glam::Vec4, color_end: glam::Vec4) -> Self {
        Self {
            mode: ColorMode::Gradient as u32,
            total_segments,
            color_start,
            color_end,
            ..Default::default()
        }
    }

    pub fn hue_cycle(total_segments: u32, hue_start: f32, saturation: f32, value: f32) -> Self {
        Self {
            mode: ColorMode::HueCycle as u32,
            total_segments,
            hue_start,
            saturation,
            value,
            ..Default::default()
        }
    }

    pub fn depth_gradient(
        total_segments: u32,
        max_topological_depth: u32,
        color_start: glam::Vec4,
        color_end: glam::Vec4,
    ) -> Self {
        Self {
            mode: ColorMode::DepthGradient as u32,
            total_segments,
            max_topological_depth,
            color_start,
            color_end,
            ..Default::default()
        }
    }

    /// Applies a hue offset only for `HueCycle`; other color modes are unchanged.
    pub fn with_hue_offset_degrees(mut self, offset: f32) -> Self {
        if self.mode == ColorMode::HueCycle as u32 {
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

pub(crate) struct StagingUnavailable;

/// Writes exactly one record per pre-sized staging slot. Count mismatches are
/// internal contract violations and panic in every build so callers can never
/// report metadata for a partial upload.
fn write_records<V: bytemuck::Pod>(
    mut view: wgpu::WriteOnly<'_, [u8]>,
    records: impl Iterator<Item = V>,
) {
    for record in records {
        let mut chunk = view
            .split_off(..std::mem::size_of::<V>())
            .expect("more records than staging bytes");
        chunk.copy_from_slice(bytemuck::bytes_of(&record));
    }
    assert!(view.is_empty(), "fewer records than staging bytes");
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
        // Data that already exists as a slice stays on `write_buffer`: on the
        // browser WebGPU backend `write_buffer_with` allocates and zeroes an
        // internal staging Vec, adding work without removing this copy.
        let required = std::mem::size_of_val(segments) as u64;
        if required > 0 {
            self.ensure_capacity(device, required, label);
            if let Some(buffer) = &self.buffer {
                queue.write_buffer(buffer, 0, bytemuck::cast_slice(segments));
            }
        }
        self.count = segments.len() as u32;
    }

    fn upload_from_iter<V: bytemuck::Pod>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        count: u32,
        records: impl Iterator<Item = V>,
        label: &str,
    ) -> Result<(), StagingUnavailable> {
        if count == 0 {
            self.count = 0;
            return Ok(());
        }

        let required = count as u64 * std::mem::size_of::<V>() as u64;
        self.ensure_capacity(device, required, label);
        let size = wgpu::BufferSize::new(required).expect("nonzero record upload size");
        let Some(mut view) = queue.write_buffer_with(
            self.buffer.as_ref().expect("capacity ensures a buffer"),
            0,
            size,
        ) else {
            self.count = 0;
            return Err(StagingUnavailable);
        };
        write_records(view.slice(..), records);
        drop(view);
        self.count = count;
        Ok(())
    }

    fn ensure_capacity(&mut self, device: &wgpu::Device, required: u64, label: &str) {
        if self.capacity < required {
            self.capacity = required.next_power_of_two();
            self.buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: self.capacity,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
    }
}

#[derive(Clone, Copy)]
enum ActiveSegmentBuffer {
    Normal,
    TopologicalDepth,
}

struct PipelineUploadTarget<'a> {
    vertex_buffer: &'a mut GrowableVertexBuffer,
    active_segment_buffer: &'a mut ActiveSegmentBuffer,
    color_params_buffer: &'a wgpu::Buffer,
    label: &'static str,
    target: ActiveSegmentBuffer,
}

impl PipelineUploadTarget<'_> {
    fn upload<V: bytemuck::Pod>(
        self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        count: u32,
        records: impl Iterator<Item = V>,
        color_params: ColorParams,
    ) -> Result<(), StagingUnavailable> {
        let result = self
            .vertex_buffer
            .upload_from_iter(device, queue, count, records, self.label);
        // Switch even on failure: the target buffer now has count zero, while
        // leaving the other layout active could draw a stale incompatible scene.
        *self.active_segment_buffer = self.target;
        if result.is_ok() {
            queue.write_buffer(self.color_params_buffer, 0, &color_params.uniform_bytes());
        }
        result
    }
}

macro_rules! impl_uniform_bytes {
    ($ty:ty) => {
        impl $ty {
            fn uniform_bytes(&self) -> Vec<u8> {
                UniformBuffer::<Vec<u8>>::content_of(self)
                    .expect(concat!(stringify!($ty), " uniform layout should encode"))
            }
        }
    };
}

impl_uniform_bytes!(Transform);
impl_uniform_bytes!(Mvp);
impl_uniform_bytes!(ColorParams);

fn draw_line_list(
    render_pass: &mut wgpu::RenderPass<'_>,
    pipeline: &wgpu::RenderPipeline,
    bind_groups: &[(u32, &wgpu::BindGroup)],
    vertex_buffer: &GrowableVertexBuffer,
    debug_label: &str,
) {
    render_pass.push_debug_group(debug_label);
    render_pass.set_pipeline(pipeline);
    for (index, bind_group) in bind_groups {
        render_pass.set_bind_group(*index, *bind_group, &[]);
    }
    if vertex_buffer.count > 0
        && let Some(buf) = &vertex_buffer.buffer
    {
        render_pass.set_vertex_buffer(0, buf.slice(..));
        render_pass.draw(0..2, 0..vertex_buffer.count);
    }
    render_pass.pop_debug_group();
}

fn create_line_pipeline(
    device: &wgpu::Device,
    pipeline_layout: &wgpu::PipelineLayout,
    label: &'static str,
    vertex: wgpu::VertexState,
    fragment: wgpu::FragmentState,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(pipeline_layout),
        vertex,
        fragment: Some(fragment),
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

/// Ties each generated bind-group wrapper to its dimension marker so the
/// shared constructor cannot accept a bind group built from the other
/// dimension's shader module. Private on purpose: the bound sits on the
/// private `from_parts`, which keeps the generated modules crate-private
/// (a public associated type would force them `pub`).
trait DimensionBindGroup<D: RenderDimension> {
    fn into_raw(self) -> wgpu::BindGroup;
}

impl DimensionBindGroup<D2> for generated_shader_2d::bind_groups::BindGroup0 {
    fn into_raw(self) -> wgpu::BindGroup {
        self.inner().clone()
    }
}

impl DimensionBindGroup<D3> for generated_shader_3d::bind_groups::BindGroup0 {
    fn into_raw(self) -> wgpu::BindGroup {
        self.inner().clone()
    }
}

struct PipelineLabels {
    segment: &'static str,
    depth_segment: &'static str,
    draw: &'static str,
    depth_draw: &'static str,
}

/// Named construction bundle for `from_parts`, so same-typed parts
/// (the two pipelines, the two uniform buffers) cannot be swapped by
/// argument order.
struct PipelineParts<B> {
    pipeline: wgpu::RenderPipeline,
    depth_pipeline: wgpu::RenderPipeline,
    view_buffer: wgpu::Buffer,
    color_params_buffer: wgpu::Buffer,
    bind_group: B,
    labels: PipelineLabels,
}

pub struct LinePipeline<D: RenderDimension> {
    pipeline: wgpu::RenderPipeline,
    depth_pipeline: wgpu::RenderPipeline,
    view_buffer: wgpu::Buffer,
    color_params_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    segment_buffer: GrowableVertexBuffer,
    depth_segment_buffer: GrowableVertexBuffer,
    active_segment_buffer: ActiveSegmentBuffer,
    labels: PipelineLabels,
    dimension: PhantomData<fn() -> D>,
}

pub type LinePipeline2D = LinePipeline<D2>;
pub type LinePipeline3D = LinePipeline<D3>;

impl LinePipeline<D2> {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("lsystem_2d_shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!(concat!(env!("OUT_DIR"), "/shader_2d.wgsl")).into(),
            ),
        });

        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("lsystem_2d_transform_uniform"),
            contents: &(Transform {
                scale: glam::Vec2::ONE,
                offset: glam::Vec2::ZERO,
            })
            .uniform_bytes(),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let color_params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("lsystem_2d_color_uniform"),
            contents: &ColorParams::default().uniform_bytes(),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bgl = generated_shader_2d::bind_groups::BindGroup0::get_bind_group_layout(device);

        let bind_group = generated_shader_2d::bind_groups::BindGroup0::from_bindings(
            device,
            generated_shader_2d::bind_groups::BindGroupLayout0 {
                color_params: color_params_buffer.as_entire_buffer_binding(),
                transform: uniform_buffer.as_entire_buffer_binding(),
            },
        );

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("lsystem_2d_pipeline_layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });

        let fragment_entry = generated_shader_2d::fs_main_entry([Some(wgpu::ColorTargetState {
            format,
            blend: Some(wgpu::BlendState::REPLACE),
            write_mask: wgpu::ColorWrites::ALL,
        })]);
        let vertex_entry = generated_shader_2d::vs_main_2d_entry(wgpu::VertexStepMode::Instance);
        let depth_vertex_entry =
            generated_shader_2d::vs_depth_main_2d_entry(wgpu::VertexStepMode::Instance);
        let pipeline = create_line_pipeline(
            device,
            &pipeline_layout,
            "lsystem_2d_pipeline",
            generated_shader_2d::vertex_state(&shader, &vertex_entry),
            generated_shader_2d::fragment_state(&shader, &fragment_entry),
        );
        let depth_pipeline = create_line_pipeline(
            device,
            &pipeline_layout,
            "lsystem_2d_depth_pipeline",
            generated_shader_2d::vertex_state(&shader, &depth_vertex_entry),
            generated_shader_2d::fragment_state(&shader, &fragment_entry),
        );

        Self::from_parts(PipelineParts {
            pipeline,
            depth_pipeline,
            view_buffer: uniform_buffer,
            color_params_buffer,
            bind_group,
            labels: PipelineLabels {
                segment: "lsystem_2d_segments",
                depth_segment: "lsystem_2d_topological_depth_segments",
                draw: "lsystem_2d_line_draw",
                depth_draw: "lsystem_2d_depth_line_draw",
            },
        })
    }

    pub fn write_transform(&self, queue: &wgpu::Queue, transform: Transform) {
        self.write_view_bytes(queue, &transform.uniform_bytes());
    }
}

impl LinePipeline<D3> {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("lsystem_3d_shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!(concat!(env!("OUT_DIR"), "/shader_3d.wgsl")).into(),
            ),
        });

        let mvp_uniform = Mvp::default().uniform_bytes();
        let mvp_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("lsystem_3d_mvp_uniform"),
            contents: &mvp_uniform,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let color_params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("lsystem_3d_color_uniform"),
            contents: &ColorParams::default().uniform_bytes(),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bgl = generated_shader_3d::bind_groups::BindGroup0::get_bind_group_layout(device);

        let bind_group = generated_shader_3d::bind_groups::BindGroup0::from_bindings(
            device,
            generated_shader_3d::bind_groups::BindGroupLayout0 {
                color_params: color_params_buffer.as_entire_buffer_binding(),
                mvp: mvp_buffer.as_entire_buffer_binding(),
            },
        );

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("lsystem_3d_pipeline_layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });

        let fragment_entry = generated_shader_3d::fs_main_entry([Some(wgpu::ColorTargetState {
            format,
            blend: Some(wgpu::BlendState::REPLACE),
            write_mask: wgpu::ColorWrites::ALL,
        })]);
        let vertex_entry = generated_shader_3d::vs_main_3d_entry(wgpu::VertexStepMode::Instance);
        let depth_vertex_entry =
            generated_shader_3d::vs_depth_main_3d_entry(wgpu::VertexStepMode::Instance);
        let pipeline = create_line_pipeline(
            device,
            &pipeline_layout,
            "lsystem_3d_pipeline",
            generated_shader_3d::vertex_state(&shader, &vertex_entry),
            generated_shader_3d::fragment_state(&shader, &fragment_entry),
        );
        let depth_pipeline = create_line_pipeline(
            device,
            &pipeline_layout,
            "lsystem_3d_depth_pipeline",
            generated_shader_3d::vertex_state(&shader, &depth_vertex_entry),
            generated_shader_3d::fragment_state(&shader, &fragment_entry),
        );

        Self::from_parts(PipelineParts {
            pipeline,
            depth_pipeline,
            view_buffer: mvp_buffer,
            color_params_buffer,
            bind_group,
            labels: PipelineLabels {
                segment: "lsystem_3d_segments",
                depth_segment: "lsystem_3d_topological_depth_segments",
                draw: "lsystem_3d_line_draw",
                depth_draw: "lsystem_3d_depth_line_draw",
            },
        })
    }

    pub fn write_mvp(&self, queue: &wgpu::Queue, mvp: Mvp) {
        self.write_view_bytes(queue, &mvp.uniform_bytes());
    }
}

impl<D: RenderDimension> LinePipeline<D> {
    fn from_parts<B: DimensionBindGroup<D>>(parts: PipelineParts<B>) -> Self {
        Self {
            pipeline: parts.pipeline,
            depth_pipeline: parts.depth_pipeline,
            view_buffer: parts.view_buffer,
            color_params_buffer: parts.color_params_buffer,
            bind_group: parts.bind_group.into_raw(),
            segment_buffer: GrowableVertexBuffer::new(),
            depth_segment_buffer: GrowableVertexBuffer::new(),
            active_segment_buffer: ActiveSegmentBuffer::Normal,
            labels: parts.labels,
            dimension: PhantomData,
        }
    }

    fn write_view_bytes(&self, queue: &wgpu::Queue, bytes: &[u8]) {
        queue.write_buffer(&self.view_buffer, 0, bytes);
    }

    pub fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        segments: &[D::PlainRecord],
        color_params: ColorParams,
    ) {
        self.segment_buffer
            .upload(device, queue, segments, self.labels.segment);
        self.active_segment_buffer = ActiveSegmentBuffer::Normal;
        self.write_color_params(queue, color_params);
    }

    pub(crate) fn upload_from_iter(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        count: u32,
        segments: impl Iterator<Item = D::PlainRecord>,
        color_params: ColorParams,
    ) -> Result<(), StagingUnavailable> {
        PipelineUploadTarget {
            vertex_buffer: &mut self.segment_buffer,
            active_segment_buffer: &mut self.active_segment_buffer,
            color_params_buffer: &self.color_params_buffer,
            label: self.labels.segment,
            target: ActiveSegmentBuffer::Normal,
        }
        .upload(device, queue, count, segments, color_params)
    }

    pub fn upload_with_topological_depth(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        segments: &[D::DepthRecord],
        color_params: ColorParams,
    ) {
        self.depth_segment_buffer
            .upload(device, queue, segments, self.labels.depth_segment);
        self.active_segment_buffer = ActiveSegmentBuffer::TopologicalDepth;
        self.write_color_params(queue, color_params);
    }

    pub(crate) fn upload_depth_from_iter(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        count: u32,
        segments: impl Iterator<Item = D::DepthRecord>,
        color_params: ColorParams,
    ) -> Result<(), StagingUnavailable> {
        PipelineUploadTarget {
            vertex_buffer: &mut self.depth_segment_buffer,
            active_segment_buffer: &mut self.active_segment_buffer,
            color_params_buffer: &self.color_params_buffer,
            label: self.labels.depth_segment,
            target: ActiveSegmentBuffer::TopologicalDepth,
        }
        .upload(device, queue, count, segments, color_params)
    }

    pub fn write_color_params(&self, queue: &wgpu::Queue, color_params: ColorParams) {
        queue.write_buffer(&self.color_params_buffer, 0, &color_params.uniform_bytes());
    }

    pub fn draw(&self, render_pass: &mut wgpu::RenderPass<'_>) {
        let bind_groups: &[(u32, &wgpu::BindGroup)] = &[(0, &self.bind_group)];
        match self.active_segment_buffer {
            ActiveSegmentBuffer::Normal => draw_line_list(
                render_pass,
                &self.pipeline,
                bind_groups,
                &self.segment_buffer,
                self.labels.draw,
            ),
            ActiveSegmentBuffer::TopologicalDepth => draw_line_list(
                render_pass,
                &self.depth_pipeline,
                bind_groups,
                &self.depth_segment_buffer,
                self.labels.depth_draw,
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
    use encase::ShaderType;

    use super::*;

    fn sample_params(mode: ColorMode, hue_start: f32) -> ColorParams {
        let color_start = glam::vec4(0.1, 0.2, 0.3, 1.0);
        let color_end = glam::vec4(0.7, 0.8, 0.9, 1.0);
        match mode {
            ColorMode::Solid => ColorParams::solid(10, color_start),
            ColorMode::Gradient => ColorParams::gradient(10, color_start, color_end),
            ColorMode::HueCycle => ColorParams::hue_cycle(10, hue_start, 0.5, 0.75),
            ColorMode::DepthGradient => ColorParams::depth_gradient(10, 3, color_start, color_end),
        }
    }

    fn push_u32(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn push_f32(bytes: &mut Vec<u8>, value: f32) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn push_vec2(bytes: &mut Vec<u8>, value: glam::Vec2) {
        push_f32(bytes, value.x);
        push_f32(bytes, value.y);
    }

    fn push_vec4(bytes: &mut Vec<u8>, value: glam::Vec4) {
        push_f32(bytes, value.x);
        push_f32(bytes, value.y);
        push_f32(bytes, value.z);
        push_f32(bytes, value.w);
    }

    fn push_mat4(bytes: &mut Vec<u8>, value: glam::Mat4) {
        for column in value.to_cols_array_2d() {
            for component in column {
                push_f32(bytes, component);
            }
        }
    }

    fn expected_transform_bytes(transform: Transform) -> Vec<u8> {
        let mut bytes = Vec::new();
        push_vec2(&mut bytes, transform.scale);
        push_vec2(&mut bytes, transform.offset);
        bytes
    }

    fn expected_mvp_bytes(mvp: Mvp) -> Vec<u8> {
        let mut bytes = Vec::new();
        push_mat4(&mut bytes, mvp.matrix);
        bytes
    }

    fn expected_color_params_bytes(params: ColorParams) -> Vec<u8> {
        let mut bytes = Vec::new();
        push_u32(&mut bytes, params.mode);
        push_u32(&mut bytes, params.total_segments);
        push_u32(&mut bytes, params.max_topological_depth);
        push_u32(&mut bytes, 0);
        push_vec4(&mut bytes, params.color_start);
        push_vec4(&mut bytes, params.color_end);
        push_f32(&mut bytes, params.hue_start);
        push_f32(&mut bytes, params.saturation);
        push_f32(&mut bytes, params.value);
        push_f32(&mut bytes, 0.0);
        bytes
    }

    fn sample_segments() -> [Segment2D; 2] {
        [
            Segment2D {
                start: glam::vec2(1.0, 2.0),
                end: glam::vec2(3.0, 4.0),
            },
            Segment2D {
                start: glam::vec2(-5.0, 6.0),
                end: glam::vec2(7.0, -8.0),
            },
        ]
    }

    #[test]
    fn render_dimension_2d_rotates_and_constructs_records() {
        let start = glam::vec2(1.0, 2.0);
        let end = glam::vec2(3.0, 4.0);

        let rotated = D2::rotate(glam::Vec2::from_angle(std::f32::consts::FRAC_PI_2), start);
        assert!(rotated.abs_diff_eq(glam::vec2(-2.0, 1.0), 1.0e-6));

        let plain = D2::plain_record(start, end);
        assert_eq!(plain.start, start);
        assert_eq!(plain.end, end);

        let depth = D2::depth_record(start, end, 7);
        assert_eq!(depth.start, start);
        assert_eq!(depth.end, end);
        assert_eq!(depth.topological_depth, 7);
    }

    #[test]
    fn render_dimension_3d_rotates_and_constructs_records() {
        let start = glam::vec3(1.0, 2.0, 3.0);
        let end = glam::vec3(4.0, 5.0, 6.0);

        let rotated = D3::rotate(
            glam::Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
            start,
        );
        assert!(rotated.abs_diff_eq(glam::vec3(-2.0, 1.0, 3.0), 1.0e-6));

        let plain = D3::plain_record(start, end);
        assert_eq!(plain.start, start);
        assert_eq!(plain.end, end);

        let depth = D3::depth_record(start, end, 11);
        assert_eq!(depth.start, start);
        assert_eq!(depth.end, end);
        assert_eq!(depth.topological_depth, 11);
    }

    #[test]
    fn write_records_fills_staging_bytes_sequentially() {
        let segments = sample_segments();
        let expected = bytemuck::cast_slice(&segments);
        let mut bytes = vec![0u8; expected.len()];

        write_records(wgpu::WriteOnly::from_mut(&mut bytes), segments.into_iter());

        assert_eq!(bytes, expected);
    }

    #[test]
    #[should_panic(expected = "more records than staging bytes")]
    fn write_records_panics_when_there_are_too_many_records() {
        let segments = sample_segments();
        let mut bytes = vec![0u8; std::mem::size_of::<Segment2D>()];

        write_records(wgpu::WriteOnly::from_mut(&mut bytes), segments.into_iter());
    }

    #[test]
    #[should_panic(expected = "fewer records than staging bytes")]
    fn write_records_panics_when_there_are_too_few_records() {
        let segments = sample_segments();
        let mut bytes = vec![0u8; std::mem::size_of_val(&segments)];

        write_records(
            wgpu::WriteOnly::from_mut(&mut bytes),
            segments.into_iter().take(1),
        );
    }

    #[test]
    fn encase_uniform_sizes_match_wgsl_layouts() {
        assert_eq!(Transform::min_size().get(), 16);
        assert_eq!(Mvp::min_size().get(), 64);
        assert_eq!(ColorParams::min_size().get(), 64);
    }

    #[test]
    fn default_color_params_use_solid_mode() {
        assert_eq!(ColorParams::default().mode, ColorMode::Solid as u32);
    }

    #[test]
    fn transform_uniform_encoding_matches_wgsl_layout() {
        let transform = Transform {
            scale: glam::vec2(1.25, -2.5),
            offset: glam::vec2(3.75, -4.125),
        };

        assert_eq!(
            transform.uniform_bytes(),
            expected_transform_bytes(transform)
        );
    }

    #[test]
    fn mvp_uniform_encoding_matches_wgsl_layout() {
        let mvp = Mvp {
            matrix: glam::Mat4::from_cols_array_2d(&[
                [1.0, 2.0, 3.0, 4.0],
                [5.0, 6.0, 7.0, 8.0],
                [9.0, 10.0, 11.0, 12.0],
                [13.0, 14.0, 15.0, 16.0],
            ]),
        };

        assert_eq!(mvp.uniform_bytes(), expected_mvp_bytes(mvp));
    }

    #[test]
    fn color_params_uniform_encoding_matches_wgsl_layout() {
        let params = sample_params(ColorMode::DepthGradient, 270.0);

        assert_eq!(params.uniform_bytes(), expected_color_params_bytes(params));
    }

    #[test]
    fn segment_caps_use_platform_selected_max_buffer_size() {
        assert_eq!(
            max_segments_for(Dimensions::TwoD),
            wgpu_util::MAX_BUFFER_SIZE_BYTES / std::mem::size_of::<Segment2D>() as u64
        );
        assert_eq!(
            max_segments_for(Dimensions::ThreeD),
            wgpu_util::MAX_BUFFER_SIZE_BYTES / std::mem::size_of::<Segment3D>() as u64
        );
    }

    #[test]
    fn depth_gradient_segment_caps_use_depth_record_sizes() {
        assert_eq!(
            max_segments_for_line_color(Dimensions::TwoD, true),
            wgpu_util::MAX_BUFFER_SIZE_BYTES
                / std::mem::size_of::<TopologicalDepthSegment2D>() as u64
        );
        assert_eq!(
            max_segments_for_line_color(Dimensions::ThreeD, true),
            wgpu_util::MAX_BUFFER_SIZE_BYTES
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

        assert_eq!(shifted.mode, ColorMode::HueCycle as u32);
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
