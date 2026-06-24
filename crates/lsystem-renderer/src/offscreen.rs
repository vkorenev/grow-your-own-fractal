//! Offscreen GPU rendering shared by the PNG and APNG export paths.

use lsystem_core::{Config, Dimensions};

use crate::bounds_compute::{self, BoundsComputeSupport};
use crate::camera::Camera;
use crate::line_renderer::{ColorParams, LinePipeline2D, LinePipeline3D};
use crate::lsystem_bridge::{
    color_params_from_config, geometry_to_depth_segments, geometry_to_depth_segments_3d,
    geometry_to_segments, geometry_to_segments_3d, viewport_transform,
};
use crate::png_export::{ExportError, MAX_DIMENSION, MIN_DIMENSION};
use crate::readback::{ReadbackError, map_read_buffer};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const BYTES_PER_PIXEL: u32 = 4;

pub(crate) fn validate_width(width: u32) -> Result<(), ExportError> {
    if (MIN_DIMENSION..=MAX_DIMENSION).contains(&width) {
        Ok(())
    } else {
        Err(ExportError::InvalidWidth(width))
    }
}

pub(crate) fn validate_height(height: u32) -> Result<(), ExportError> {
    if (MIN_DIMENSION..=MAX_DIMENSION).contains(&height) {
        Ok(())
    } else {
        Err(ExportError::InvalidHeight(height))
    }
}

enum SceneGeometry {
    TwoD {
        pipeline: LinePipeline2D,
        bounds_min: [f32; 2],
        bounds_max: [f32; 2],
    },
    ThreeD {
        pipeline: LinePipeline3D,
        bounds_min: [f32; 3],
        bounds_max: [f32; 3],
    },
}

/// L-system geometry uploaded to a GPU line pipeline, ready for offscreen
/// rendering. Pairs the pipeline with the geometry bounds of the matching
/// dimensionality and keeps the color params resolved from the config.
pub(crate) struct ExportScene {
    geometry: SceneGeometry,
    color_params: ColorParams,
}

impl ExportScene {
    /// Generates the L-system geometry for `config` and uploads it to `device`.
    pub(crate) async fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bounds_support: BoundsComputeSupport,
        config: &Config,
    ) -> Result<Self, ReadbackError> {
        let line = &config.colors.line;
        let needs_depth =
            line.needs_topological_depth() && config.generation.has_stack_directives();
        match config.generation.dimensions {
            Dimensions::TwoD => {
                let mut pipeline = LinePipeline2D::new(device, FORMAT);
                let (color_params, bounds_min, bounds_max) = if needs_depth {
                    let data = geometry_to_depth_segments(
                        lsystem_core::generate_with_topological_depth(&config.generation),
                    );
                    let cp = color_params_from_config(
                        line,
                        data.segments.len() as u32,
                        Some(data.max_topological_depth()),
                    );
                    pipeline.upload_with_topological_depth(device, queue, &data.segments, cp);
                    let bounds = bounds_compute::depth_segment_bounds_2d(
                        device,
                        queue,
                        bounds_support,
                        &data.segments,
                        pipeline.active_uploaded_segment_buffer(),
                    )
                    .await?;
                    (cp, bounds.0, bounds.1)
                } else {
                    let data = geometry_to_segments(lsystem_core::generate(&config.generation));
                    let cp = color_params_from_config(line, data.segments.len() as u32, None);
                    pipeline.upload(device, queue, &data.segments, cp);
                    let bounds = bounds_compute::segment_bounds_2d(
                        device,
                        queue,
                        bounds_support,
                        &data.segments,
                        pipeline.active_uploaded_segment_buffer(),
                    )
                    .await?;
                    (cp, bounds.0, bounds.1)
                };
                Ok(Self {
                    geometry: SceneGeometry::TwoD {
                        pipeline,
                        bounds_min,
                        bounds_max,
                    },
                    color_params,
                })
            }
            Dimensions::ThreeD => {
                let mut pipeline = LinePipeline3D::new(device, FORMAT);
                let (color_params, bounds_min, bounds_max) = if needs_depth {
                    let data = geometry_to_depth_segments_3d(
                        lsystem_core::generate_3d_with_topological_depth(&config.generation),
                    );
                    let cp = color_params_from_config(
                        line,
                        data.segments.len() as u32,
                        Some(data.max_topological_depth()),
                    );
                    pipeline.upload_with_topological_depth(device, queue, &data.segments, cp);
                    let bounds = bounds_compute::depth_segment_bounds_3d(
                        device,
                        queue,
                        bounds_support,
                        &data.segments,
                        pipeline.active_uploaded_segment_buffer(),
                    )
                    .await?;
                    (cp, bounds.0, bounds.1)
                } else {
                    let data =
                        geometry_to_segments_3d(lsystem_core::generate_3d(&config.generation));
                    let cp = color_params_from_config(line, data.segments.len() as u32, None);
                    pipeline.upload(device, queue, &data.segments, cp);
                    let bounds = bounds_compute::segment_bounds_3d(
                        device,
                        queue,
                        bounds_support,
                        &data.segments,
                        pipeline.active_uploaded_segment_buffer(),
                    )
                    .await?;
                    (cp, bounds.0, bounds.1)
                };
                Ok(Self {
                    geometry: SceneGeometry::ThreeD {
                        pipeline,
                        bounds_min,
                        bounds_max,
                    },
                    color_params,
                })
            }
        }
    }

    /// Color params resolved from the config at upload time.
    pub(crate) fn color_params(&self) -> ColorParams {
        self.color_params
    }

    /// Writes the view uniform. 2D export always fits the geometry bounds
    /// (camera pan/zoom is ignored); 3D uses the camera orientation.
    pub(crate) fn write_camera(
        &self,
        queue: &wgpu::Queue,
        camera: &Camera,
        width: u32,
        height: u32,
    ) {
        match &self.geometry {
            SceneGeometry::TwoD {
                pipeline,
                bounds_min,
                bounds_max,
            } => pipeline.write_transform(
                queue,
                viewport_transform(*bounds_min, *bounds_max, width, height, [0.0, 0.0], 1.0),
            ),
            SceneGeometry::ThreeD {
                pipeline,
                bounds_min,
                bounds_max,
            } => pipeline.write_mvp(
                queue,
                camera.compute_mvp_3d(*bounds_min, *bounds_max, width, height),
            ),
        }
    }

    pub(crate) fn write_color_params(&self, queue: &wgpu::Queue, color_params: ColorParams) {
        match &self.geometry {
            SceneGeometry::TwoD { pipeline, .. } => {
                pipeline.write_color_params(queue, color_params)
            }
            SceneGeometry::ThreeD { pipeline, .. } => {
                pipeline.write_color_params(queue, color_params)
            }
        }
    }

    fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        match &self.geometry {
            SceneGeometry::TwoD { pipeline, .. } => pipeline.draw(pass),
            SceneGeometry::ThreeD { pipeline, .. } => pipeline.draw(pass),
        }
    }
}

/// Offscreen render texture with its readback buffer.
pub(crate) struct RenderTarget {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    readback: wgpu::Buffer,
    layout: ReadbackLayout,
}

impl RenderTarget {
    pub(crate) fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("export_texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("export_texture_view"),
            ..Default::default()
        });
        let layout = ReadbackLayout::new(width, height);
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("export_readback"),
            size: layout.buffer_size(),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Self {
            texture,
            view,
            readback,
            layout,
        }
    }

    /// Renders one frame of `scene` over the `background` color and reads it
    /// back as tightly packed RGBA bytes.
    pub(crate) async fn render_frame(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        background: [f32; 3],
        scene: &ExportScene,
    ) -> Result<Vec<u8>, ReadbackError> {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("export_encoder"),
        });
        {
            let [r, g, b] = background;
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("export_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.view,
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
            scene.draw(&mut pass);
        }
        encoder.copy_texture_to_buffer(
            self.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.layout.padded_bytes_per_row),
                    rows_per_image: None,
                },
            },
            self.texture.size(),
        );
        queue.submit([encoder.finish()]);

        map_read_buffer(device, &self.readback).await?;
        let rgba = {
            let mapped = self.readback.slice(..).get_mapped_range();
            self.layout.strip_padding(&mapped)
        };
        self.readback.unmap();
        Ok(rgba)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_export_width() {
        assert!(matches!(
            validate_width(MIN_DIMENSION - 1),
            Err(ExportError::InvalidWidth(width)) if width == MIN_DIMENSION - 1
        ));
        assert!(validate_width(MIN_DIMENSION).is_ok());
        assert!(validate_width(MAX_DIMENSION).is_ok());
        assert!(matches!(
            validate_width(MAX_DIMENSION + 1),
            Err(ExportError::InvalidWidth(width)) if width == MAX_DIMENSION + 1
        ));
    }

    #[test]
    fn validates_export_height() {
        assert!(matches!(
            validate_height(MIN_DIMENSION - 1),
            Err(ExportError::InvalidHeight(height)) if height == MIN_DIMENSION - 1
        ));
        assert!(validate_height(MIN_DIMENSION).is_ok());
        assert!(validate_height(MAX_DIMENSION).is_ok());
        assert!(matches!(
            validate_height(MAX_DIMENSION + 1),
            Err(ExportError::InvalidHeight(height)) if height == MAX_DIMENSION + 1
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
}
