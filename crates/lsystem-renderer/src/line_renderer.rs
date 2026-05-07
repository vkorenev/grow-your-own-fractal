use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub struct Transform {
    pub scale: [f32; 2],
    pub offset: [f32; 2],
}

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 2],
}

/// Maximum number of line segments that fit in a 256 MiB vertex buffer (wgpu's guaranteed limit).
/// Each segment occupies 2 vertices × `size_of::<Vertex>()` bytes.
pub const MAX_SEGMENTS: u64 = 268_435_456 / (2 * std::mem::size_of::<Vertex>() as u64);

/// Per-frame color parameters written to the GPU as a uniform.
/// Layout mirrors `ColorParams` in `shader.wgsl`; padding keeps vec4 alignment.
#[repr(C)]
#[derive(Copy, Clone, Default, Pod, Zeroable)]
pub struct ColorParams {
    /// 0 = solid, 1 = gradient, 2 = hue_cycle
    pub mode: u32,
    pub total_segments: u32,
    pub _pad: [u32; 2],
    pub color_start: [f32; 4],
    pub color_end: [f32; 4],
    pub hue_start: f32,
    pub saturation: f32,
    pub value: f32,
    pub _pad2: f32,
}

/// GPU pipeline for rendering colored line-list geometry.
pub struct LinePipeline {
    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    color_params_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    vertex_buffer: wgpu::Buffer,
    vertex_count: u32,
}

impl LinePipeline {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::bytes_of(&Transform {
                scale: [1.0, 1.0],
                offset: [0.0, 0.0],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let color_params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::bytes_of(&ColorParams::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let uniform_entry = wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let color_entry = wgpu::BindGroupLayoutEntry {
            binding: 1,
            ..uniform_entry
        };
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[uniform_entry, color_entry],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
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
            label: None,
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2],
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
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
        });

        // Placeholder; never bound for drawing (vertex_count stays 0 until the first upload).
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: std::mem::size_of::<Vertex>() as u64,
            usage: wgpu::BufferUsages::VERTEX,
            mapped_at_creation: false,
        });

        Self {
            pipeline,
            uniform_buffer,
            color_params_buffer,
            bind_group,
            vertex_buffer,
            vertex_count: 0,
        }
    }

    pub fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        vertices: &[Vertex],
        color_params: ColorParams,
    ) {
        if !vertices.is_empty() {
            self.vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytemuck::cast_slice(vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
        }
        self.vertex_count = vertices.len() as u32;
        queue.write_buffer(
            &self.color_params_buffer,
            0,
            bytemuck::bytes_of(&color_params),
        );
    }

    pub fn write_transform(&self, queue: &wgpu::Queue, transform: Transform) {
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&transform));
    }

    pub fn draw(&self, render_pass: &mut wgpu::RenderPass<'_>) {
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, &self.bind_group, &[]);
        if self.vertex_count > 0 {
            render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            render_pass.draw(0..self.vertex_count, 0..1);
        }
    }
}

pub enum FrameOutcome {
    Ready(
        Box<wgpu::SurfaceTexture>,
        wgpu::TextureView,
        wgpu::CommandEncoder,
        bool,
    ),
    SurfaceLost,
    Skip,
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
    ) -> Result<Self, ()> {
        let instance = wgpu::Instance::default();
        let surface = instance.create_surface(target).map_err(|_| ())?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .map_err(|_| ())?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default())
            .await
            .map_err(|_| ())?;

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
            .unwrap_or(caps.formats[0]);

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: width.max(1),
            height: height.max(1),
            present_mode: caps.present_modes[0],
            alpha_mode: caps.alpha_modes[0],
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
                wgpu::CurrentSurfaceTexture::Success(t) => break (t, false),
                wgpu::CurrentSurfaceTexture::Suboptimal(t) => break (t, true),
                wgpu::CurrentSurfaceTexture::Outdated => {
                    if retried_after_outdated {
                        return FrameOutcome::Skip;
                    }
                    self.surface.configure(&self.device, &self.surface_config);
                    retried_after_outdated = true;
                }
                wgpu::CurrentSurfaceTexture::Lost => return FrameOutcome::SurfaceLost,
                wgpu::CurrentSurfaceTexture::Timeout
                | wgpu::CurrentSurfaceTexture::Occluded
                | wgpu::CurrentSurfaceTexture::Validation => return FrameOutcome::Skip,
            }
        };

        let view = frame.texture.create_view(&Default::default());
        let encoder = self.device.create_command_encoder(&Default::default());
        // The surface is cleared by the caller's render pass (LoadOp::Clear).
        FrameOutcome::Ready(Box::new(frame), view, encoder, reconfigure_after)
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
