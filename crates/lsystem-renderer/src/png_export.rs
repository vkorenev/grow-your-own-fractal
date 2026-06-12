use std::error::Error;
use std::fmt::{Display, Formatter};

use lsystem_core::Config;

use crate::camera::Camera;
use crate::offscreen::{ExportScene, ReadbackError, RenderTarget, validate_width};
use crate::wgpu_util::{self, CreateDeviceError};

/// Minimum export width in pixels accepted by [`render_png`] and
/// [`render_animation`](crate::animation_export::render_animation).
pub const MIN_WIDTH: u32 = 256;
/// Maximum export width — and derived height — in pixels.
pub const MAX_DIMENSION: u32 = 4096;

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
                    "export width must be in {MIN_WIDTH}..={MAX_DIMENSION}, got {width}"
                )
            }
            Self::InvalidHeight(height) => {
                write!(
                    f,
                    "derived export height must be in 1..={MAX_DIMENSION}, got {height}"
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

/// Renders `config` to a PNG of the given width on `device`.
///
/// The height is derived from the geometry aspect ratio in 2D and equals
/// `width` in 3D. `camera` selects the 3D orientation; 2D export always fits
/// the geometry bounds and ignores camera pan/zoom.
pub async fn render_png(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    config: &Config,
    width: u32,
    camera: &Camera,
) -> Result<PngExport, ExportError> {
    validate_width(width)?;

    let scene = ExportScene::new(device, queue, config);
    let height = scene.height_for_width(width)?;
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
    camera: &Camera,
) -> Result<PngExport, ExportError> {
    let (device, queue) =
        wgpu_util::create_headless_device("png_export_device", "PNG export").await?;
    render_png(&device, &queue, config, width, camera).await
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
