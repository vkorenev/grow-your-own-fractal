use std::error::Error;
use std::fmt::{Display, Formatter};

use futures_channel::oneshot;
use lsystem_core::{Config, Dimensions};

use crate::camera::Camera;
use crate::line_renderer::{LinePipeline2D, LinePipeline3D};
use crate::lsystem_bridge::{
    color_params_from_config, geometry_to_depth_segments, geometry_to_depth_segments_3d,
    geometry_to_segments, geometry_to_segments_3d, viewport_transform,
};
use crate::wgpu_util;

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const MIN_WIDTH: u32 = 256;
const MAX_DIMENSION: u32 = 4096;
const BYTES_PER_PIXEL: u32 = 4;

pub struct PngExport {
    pub width: u32,
    pub height: u32,
    pub bytes: Vec<u8>,
}

#[derive(Debug)]
pub enum PngExportError {
    InvalidWidth(u32),
    InvalidHeight(u32),
    NoAdapter,
    RequestDevice(wgpu::RequestDeviceError),
    Map(wgpu::BufferAsyncError),
    MapChannelClosed,
    Poll(wgpu::PollError),
    Png(png::EncodingError),
}

impl Display for PngExportError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidWidth(width) => {
                write!(
                    f,
                    "PNG width must be in {MIN_WIDTH}..={MAX_DIMENSION}, got {width}"
                )
            }
            Self::InvalidHeight(height) => {
                write!(
                    f,
                    "derived PNG height must be in 1..={MAX_DIMENSION}, got {height}"
                )
            }
            Self::NoAdapter => write!(f, "no GPU adapter available for PNG export"),
            Self::RequestDevice(err) => write!(f, "failed to create PNG export GPU device: {err}"),
            Self::Map(err) => write!(f, "failed to map PNG readback buffer: {err}"),
            Self::MapChannelClosed => write!(f, "PNG readback callback was dropped"),
            Self::Poll(err) => write!(f, "failed to poll GPU device for PNG readback: {err}"),
            Self::Png(err) => write!(f, "failed to encode PNG: {err}"),
        }
    }
}

impl Error for PngExportError {}

pub async fn render_png_standalone(
    config: &Config,
    width: u32,
    camera: &Camera,
) -> Result<PngExport, PngExportError> {
    let instance = wgpu_util::new_instance().await;
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            compatible_surface: None,
            force_fallback_adapter: false,
        })
        .await
        .map_err(|_| PngExportError::NoAdapter)?;
    let adapter_info = adapter.get_info();
    log::info!(
        "Selected PNG export GPU adapter: {} ({})",
        adapter_info.name,
        adapter_info.backend
    );
    let (device, queue) = adapter
        .request_device(&wgpu_util::device_descriptor("png_export_device", &adapter))
        .await
        .map_err(PngExportError::RequestDevice)?;
    wgpu_util::install_uncaptured_error_handler(&device, "PNG export");

    render_png(&device, &queue, config, width, camera).await
}

pub async fn render_png(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    config: &Config,
    width: u32,
    camera: &Camera,
) -> Result<PngExport, PngExportError> {
    validate_width(width)?;

    let colors = config.effective_colors();
    match config.generation.dimensions {
        Dimensions::ThreeD => {
            let height = width; // square viewport for perspective
            let mut pipeline = LinePipeline3D::new(device, FORMAT);
            let (bounds_min, bounds_max) = if colors.line.needs_topological_depth() {
                let data = geometry_to_depth_segments_3d(
                    lsystem_core::generate_3d_with_topological_depth(&config.generation),
                );
                let color_params = color_params_from_config(
                    &colors.line,
                    data.segments.len() as u32,
                    data.max_topological_depth(),
                );
                pipeline.upload_with_topological_depth(device, queue, &data.segments, color_params);
                (data.bounds_min, data.bounds_max)
            } else {
                let data = geometry_to_segments_3d(lsystem_core::generate_3d(&config.generation));
                let color_params =
                    color_params_from_config(&colors.line, data.segments.len() as u32, 0);
                pipeline.upload(device, queue, &data.segments, color_params);
                (data.bounds_min, data.bounds_max)
            };
            pipeline.write_mvp(
                queue,
                camera.compute_mvp_3d(bounds_min, bounds_max, width, height),
            );
            finish_render(
                device,
                queue,
                width,
                height,
                colors.effective_background(),
                |pass| {
                    pipeline.draw(pass);
                },
            )
            .await
        }
        Dimensions::TwoD => {
            let mut pipeline = LinePipeline2D::new(device, FORMAT);
            let (bounds_min, bounds_max) = if colors.line.needs_topological_depth() {
                let data = geometry_to_depth_segments(
                    lsystem_core::generate_with_topological_depth(&config.generation),
                );
                let color_params = color_params_from_config(
                    &colors.line,
                    data.segments.len() as u32,
                    data.max_topological_depth(),
                );
                pipeline.upload_with_topological_depth(device, queue, &data.segments, color_params);
                (data.bounds_min, data.bounds_max)
            } else {
                let data = geometry_to_segments(lsystem_core::generate(&config.generation));
                let color_params =
                    color_params_from_config(&colors.line, data.segments.len() as u32, 0);
                pipeline.upload(device, queue, &data.segments, color_params);
                (data.bounds_min, data.bounds_max)
            };
            let height = derive_height(width, bounds_min, bounds_max)?;
            pipeline.write_transform(
                queue,
                viewport_transform(bounds_min, bounds_max, width, height, [0.0, 0.0], 1.0),
            );
            finish_render(
                device,
                queue,
                width,
                height,
                colors.effective_background(),
                |pass| {
                    pipeline.draw(pass);
                },
            )
            .await
        }
    }
}

async fn finish_render(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    width: u32,
    height: u32,
    background: [f32; 3],
    draw: impl FnOnce(&mut wgpu::RenderPass<'_>),
) -> Result<PngExport, PngExportError> {
    let extent = wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("png_export_texture"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("png_export_texture_view"),
        ..Default::default()
    });

    let readback_layout = ReadbackLayout::new(width, height);
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("png_export_readback"),
        size: readback_layout.buffer_size(),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("png_export_encoder"),
    });
    {
        let [r, g, b] = background;
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("png_export_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: r as f64,
                        g: g as f64,
                        b: b as f64,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
            multiview_mask: None,
        });
        draw(&mut pass);
    }
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(readback_layout.padded_bytes_per_row),
                rows_per_image: None,
            },
        },
        extent,
    );

    queue.submit([encoder.finish()]);

    let (sender, receiver) = oneshot::channel();
    readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
    // Native needs polling to drive the map_async callback. On browser WebGPU this is a no-op;
    // callback completion is driven by the browser event loop.
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .map_err(PngExportError::Poll)?;

    receiver
        .await
        .map_err(|_| PngExportError::MapChannelClosed)?
        .map_err(PngExportError::Map)?;
    let rgba = {
        let mapped = readback.slice(..).get_mapped_range();
        readback_layout.strip_padding(&mapped)
    };
    readback.unmap();

    let bytes = encode_png_rgba(width, height, &rgba)?;
    Ok(PngExport {
        width,
        height,
        bytes,
    })
}

fn validate_width(width: u32) -> Result<(), PngExportError> {
    if (MIN_WIDTH..=MAX_DIMENSION).contains(&width) {
        Ok(())
    } else {
        Err(PngExportError::InvalidWidth(width))
    }
}

fn derive_height(
    width: u32,
    bounds_min: [f32; 2],
    bounds_max: [f32; 2],
) -> Result<u32, PngExportError> {
    let geom_w = (bounds_max[0] - bounds_min[0]).max(1.0);
    let geom_h = (bounds_max[1] - bounds_min[1]).max(1.0);
    let height = (width as f32 * geom_h / geom_w).ceil();
    if !height.is_finite() {
        return Err(PngExportError::InvalidHeight(0));
    }
    let height = height as u32;
    if (1..=MAX_DIMENSION).contains(&height) {
        Ok(height)
    } else {
        Err(PngExportError::InvalidHeight(height))
    }
}

#[derive(Copy, Clone)]
struct ReadbackLayout {
    height: u32,
    unpadded_bytes_per_row: u32,
    padded_bytes_per_row: u32,
}

impl ReadbackLayout {
    fn new(width: u32, height: u32) -> Self {
        let unpadded_bytes_per_row = width * BYTES_PER_PIXEL;
        Self {
            height,
            unpadded_bytes_per_row,
            padded_bytes_per_row: wgpu::util::align_to(
                unpadded_bytes_per_row,
                wgpu::COPY_BYTES_PER_ROW_ALIGNMENT,
            ),
        }
    }

    fn buffer_size(self) -> u64 {
        (self.padded_bytes_per_row * self.height) as u64
    }

    fn strip_padding(self, padded: &[u8]) -> Vec<u8> {
        let unpadded_bytes_per_row = self.unpadded_bytes_per_row as usize;
        let mut rgba = Vec::with_capacity(unpadded_bytes_per_row * self.height as usize);
        for row in padded
            .chunks(self.padded_bytes_per_row as usize)
            .take(self.height as usize)
        {
            rgba.extend_from_slice(&row[..unpadded_bytes_per_row]);
        }
        rgba
    }
}

fn encode_png_rgba(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, PngExportError> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(PngExportError::Png)?;
        writer.write_image_data(rgba).map_err(PngExportError::Png)?;
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_export_width() {
        assert!(matches!(
            validate_width(MIN_WIDTH - 1),
            Err(PngExportError::InvalidWidth(width)) if width == MIN_WIDTH - 1
        ));
        assert!(validate_width(MIN_WIDTH).is_ok());
        assert!(validate_width(MAX_DIMENSION).is_ok());
        assert!(matches!(
            validate_width(MAX_DIMENSION + 1),
            Err(PngExportError::InvalidWidth(width)) if width == MAX_DIMENSION + 1
        ));
    }

    #[test]
    fn derives_height_from_bounds_with_degenerate_dimensions() {
        assert_eq!(derive_height(1024, [0.0, 0.0], [2.0, 1.0]).unwrap(), 512);
        assert_eq!(derive_height(1024, [0.0, 0.0], [10.0, 0.0]).unwrap(), 103);
        assert_eq!(derive_height(1024, [0.0, 0.0], [0.0, 4.0]).unwrap(), 4096);
        assert!(matches!(
            derive_height(1024, [0.0, 0.0], [0.0, 5.0]),
            Err(PngExportError::InvalidHeight(5120))
        ));
    }

    #[test]
    fn strips_aligned_row_padding() {
        let width = 64;
        let height = 2;
        let row_len = width * BYTES_PER_PIXEL;
        assert_eq!(row_len, wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let padded: Vec<_> = (0..row_len * height).map(|i| i as u8).collect();
        let stripped = ReadbackLayout::new(width, height).strip_padding(&padded);
        assert_eq!(stripped, padded);
    }

    #[test]
    fn strips_unaligned_row_padding() {
        let width = 3;
        let height = 2;
        let padded_row_len = 16;
        let padded = [
            1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 99, 99, 99, 99, 13, 14, 15, 16, 17, 18, 19, 20,
            21, 22, 23, 24, 88, 88, 88, 88,
        ];
        let mut layout = ReadbackLayout::new(width, height);
        layout.padded_bytes_per_row = padded_row_len;
        let stripped = layout.strip_padding(&padded);
        assert_eq!(
            stripped,
            vec![
                1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23,
                24,
            ]
        );
    }

    #[test]
    fn encodes_png_signature_and_ihdr_dimensions() {
        let rgba = vec![0, 0, 0, 255, 255, 255, 255, 255];
        let png = encode_png_rgba(2, 1, &rgba).unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(&png[16..20], &2u32.to_be_bytes());
        assert_eq!(&png[20..24], &1u32.to_be_bytes());
    }
}
