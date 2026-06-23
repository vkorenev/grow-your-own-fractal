use std::error::Error;
use std::fmt::{Display, Formatter};

use lsystem_core::Config;

use crate::camera::Camera;
use crate::offscreen::{ExportScene, ReadbackError, RenderTarget, validate_height, validate_width};
use crate::wgpu_util::{self, CreateDeviceError};

/// Minimum export width and height in pixels accepted by [`render_png`] and
/// [`render_animation`](crate::animation_export::render_animation).
pub const MIN_DIMENSION: u32 = 1;
/// Maximum export width and height in pixels.
pub const MAX_DIMENSION: u32 = 8192;

/// An encoded PNG (or APNG) image together with its pixel dimensions.
pub struct PngExport {
    pub width: u32,
    pub height: u32,
    pub bytes: Vec<u8>,
}

/// Failures shared by still-PNG and APNG export.
#[derive(Debug)]
pub enum ExportError {
    InvalidWidth(u32),
    InvalidHeight(u32),
    NoAdapter,
    RequestDevice(wgpu::RequestDeviceError),
    Map(wgpu::BufferAsyncError),
    MapChannelClosed,
    Poll(wgpu::PollError),
    Encode(png::EncodingError),
}

impl Display for ExportError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidWidth(width) => {
                write!(
                    f,
                    "export width must be in {MIN_DIMENSION}..={MAX_DIMENSION}, got {width}"
                )
            }
            Self::InvalidHeight(height) => {
                write!(
                    f,
                    "export height must be in {MIN_DIMENSION}..={MAX_DIMENSION}, got {height}"
                )
            }
            Self::NoAdapter => write!(f, "no GPU adapter available for export"),
            Self::RequestDevice(err) => write!(f, "failed to create export GPU device: {err}"),
            Self::Map(err) => write!(f, "failed to map export readback buffer: {err}"),
            Self::MapChannelClosed => write!(f, "export readback callback was dropped"),
            Self::Poll(err) => write!(f, "failed to poll GPU device for export readback: {err}"),
            Self::Encode(err) => write!(f, "failed to encode PNG: {err}"),
        }
    }
}

impl Error for ExportError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::RequestDevice(err) => Some(err),
            Self::Map(err) => Some(err),
            Self::Poll(err) => Some(err),
            Self::Encode(err) => Some(err),
            _ => None,
        }
    }
}

impl From<ReadbackError> for ExportError {
    fn from(e: ReadbackError) -> Self {
        match e {
            ReadbackError::Map(e) => Self::Map(e),
            ReadbackError::ChannelClosed => Self::MapChannelClosed,
            ReadbackError::Poll(e) => Self::Poll(e),
        }
    }
}

impl From<CreateDeviceError> for ExportError {
    fn from(e: CreateDeviceError) -> Self {
        match e {
            CreateDeviceError::NoAdapter => Self::NoAdapter,
            CreateDeviceError::RequestDevice(e) => Self::RequestDevice(e),
        }
    }
}

/// Renders `config` to a PNG of the given dimensions on `device`.
///
/// `camera` selects the 3D orientation; 2D export always fits the geometry
/// bounds and ignores camera pan/zoom.
pub async fn render_png(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    config: &Config,
    width: u32,
    height: u32,
    camera: &Camera,
) -> Result<PngExport, ExportError> {
    validate_width(width)?;
    validate_height(height)?;

    let scene = ExportScene::new(device, queue, config);
    scene.write_camera(queue, camera, width, height);

    let target = RenderTarget::new(device, width, height);
    let rgba = target
        .render_frame(device, queue, config.colors.background.to_array(), &scene)
        .await?;

    let bytes = encode_png_rgba(width, height, &rgba)?;
    Ok(PngExport {
        width,
        height,
        bytes,
    })
}

/// Like [`render_png`], but creates its own headless GPU device.
pub async fn render_png_standalone(
    config: &Config,
    width: u32,
    height: u32,
    camera: &Camera,
) -> Result<PngExport, ExportError> {
    validate_width(width)?;
    validate_height(height)?;
    let (device, queue) =
        wgpu_util::create_headless_device("png_export_device", "PNG export").await?;
    render_png(&device, &queue, config, width, height, camera).await
}

fn encode_png_rgba(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, ExportError> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(ExportError::Encode)?;
        writer.write_image_data(rgba).map_err(ExportError::Encode)?;
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_png_signature_and_ihdr_dimensions() {
        let rgba = vec![0, 0, 0, 255, 255, 255, 255, 255];
        let png = encode_png_rgba(2, 1, &rgba).unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(&png[16..20], &2u32.to_be_bytes());
        assert_eq!(&png[20..24], &1u32.to_be_bytes());
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod gpu_tests {
    use super::*;
    use lsystem_core::{Dimensions, GenerationConfig, LineColorConfig, Rgb};
    use std::collections::BTreeMap;

    fn trivial_config() -> lsystem_core::Config {
        lsystem_core::Config {
            name: "test".to_string(),
            generation: GenerationConfig {
                dimensions: Dimensions::TwoD,
                axiom: "F".to_string(),
                iterations: 0,
                angle: 90.0,
                step: 1.0,
                initial_heading: 0.0,
                rules: BTreeMap::new(),
            },
            colors: lsystem_core::ColorConfig {
                background: Rgb::new(0, 0, 0),
                line: LineColorConfig::Solid(Rgb::new(255, 255, 255)),
            },
        }
    }

    fn assert_png_renders(config: lsystem_core::Config) {
        let export = pollster::block_on(render_png_standalone(
            &config,
            256,
            128,
            &crate::camera::Camera::default(),
        ))
        .expect("render_png_standalone failed");

        assert_eq!(export.width, 256);
        assert_eq!(export.height, 128);
        let decoder = png::Decoder::new(std::io::Cursor::new(export.bytes.as_slice()));
        let reader = decoder.read_info().unwrap();
        let info = reader.info();
        assert_eq!((info.width, info.height), (256, 128));
    }

    fn depth_gradient_config(dimensions: Dimensions) -> lsystem_core::Config {
        let mut config = trivial_config();
        config.generation.dimensions = dimensions;
        config.generation.axiom = "F[+F]F".to_string();
        config.colors.line = LineColorConfig::Gradient {
            start: Rgb::new(255, 0, 0),
            end: Rgb::new(0, 0, 255),
            topological_depth: true,
        };
        config
    }

    #[test]
    fn png_standalone_non_square_dimensions() {
        assert_png_renders(trivial_config());
    }

    #[test]
    fn png_standalone_compiles_3d_solid_shader() {
        let mut config = trivial_config();
        config.generation.dimensions = Dimensions::ThreeD;
        assert_png_renders(config);
    }

    #[test]
    fn png_standalone_compiles_2d_depth_shader() {
        assert_png_renders(depth_gradient_config(Dimensions::TwoD));
    }

    #[test]
    fn png_standalone_compiles_3d_depth_shader() {
        assert_png_renders(depth_gradient_config(Dimensions::ThreeD));
    }
}
