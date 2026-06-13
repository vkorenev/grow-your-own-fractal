use std::error::Error;
use std::fmt::{Display, Formatter};

use lsystem_core::Config;

use crate::camera::Camera;
use crate::offscreen::{ExportScene, RenderTarget, validate_width};
use crate::png_export::{ExportError, PngExport};
use crate::wgpu_util;

/// Timing and animation inputs for [`render_animation`].
#[derive(Clone, Debug)]
pub struct AnimationParams {
    /// Frames per second; must be non-zero.
    pub fps: u16,
    /// Total frame count; must be in `1..=AnimationParams::MAX_FRAMES`.
    pub num_frames: u32,
    /// Hue phase at frame 0, in degrees.
    pub initial_hue_phase_degrees: f32,
    /// Signed hue rotation speed in degrees per second; 0.0 = no hue animation.
    pub hue_rotation_dps: f32,
    /// Signed 3D camera auto-rotate speed in degrees per second; 0.0 = none.
    pub auto_rotate_dps: f32,
}

impl AnimationParams {
    /// Upper bound on [`num_frames`](Self::num_frames).
    pub const MAX_FRAMES: u32 = 3600;

    /// Checks the timing fields; called by [`render_animation`] before any
    /// GPU work, and usable by UIs for early validation.
    pub fn validate(&self) -> Result<(), AnimationExportError> {
        if self.fps == 0 {
            return Err(AnimationExportError::ZeroFps);
        }
        if self.num_frames == 0 || self.num_frames > Self::MAX_FRAMES {
            return Err(AnimationExportError::InvalidFrameCount(self.num_frames));
        }
        for (name, val) in [
            ("initial_hue_phase_degrees", self.initial_hue_phase_degrees),
            ("hue_rotation_dps", self.hue_rotation_dps),
            ("auto_rotate_dps", self.auto_rotate_dps),
        ] {
            if !val.is_finite() {
                return Err(AnimationExportError::NonFiniteValue(name));
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
pub enum AnimationExportError {
    /// `num_frames` was outside `1..=AnimationParams::MAX_FRAMES`.
    InvalidFrameCount(u32),
    ZeroFps,
    /// A float field in [`AnimationParams`] was NaN or infinite.
    NonFiniteValue(&'static str),
    Export(ExportError),
}

impl Display for AnimationExportError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidFrameCount(n) => write!(
                f,
                "animation frame count must be in 1..={MAX_FRAMES}, got {n}",
                MAX_FRAMES = AnimationParams::MAX_FRAMES,
            ),
            Self::ZeroFps => write!(f, "animation fps must be > 0"),
            Self::NonFiniteValue(field) => {
                write!(f, "animation param `{field}` must be finite")
            }
            Self::Export(e) => e.fmt(f),
        }
    }
}

impl Error for AnimationExportError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Export(e) => Some(e),
            _ => None,
        }
    }
}

impl From<ExportError> for AnimationExportError {
    fn from(e: ExportError) -> Self {
        Self::Export(e)
    }
}

/// Renders `config` to a looping APNG on `device`.
///
/// Frame timing is analytic (`t = frame / fps`), so frame 0 matches the
/// passed-in `camera` and `initial_hue_phase_degrees` exactly and there is no
/// cumulative drift. Geometry is uploaded once; only uniforms change per
/// frame. Sizing and camera semantics match
/// [`render_png`](crate::png_export::render_png). `on_progress` receives
/// `(completed_frames, total_frames)` after each frame.
pub async fn render_animation(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    config: &Config,
    width: u32,
    camera: &Camera,
    params: &AnimationParams,
    on_progress: impl Fn(u32, u32),
) -> Result<PngExport, AnimationExportError> {
    params.validate()?;
    validate_width(width)?;

    let scene = ExportScene::new(device, queue, config);
    let height = scene.height_for_width(width)?;
    let target = RenderTarget::new(device, width, height);
    let background = config.colors.background.to_array();
    let base_color = scene.color_params();

    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        // 0 = loop forever
        encoder
            .set_animated(params.num_frames, 0)
            .map_err(encode_error)?;
        let mut writer = encoder.write_header().map_err(encode_error)?;

        for frame in 0..params.num_frames {
            let t = frame as f32 / params.fps as f32;

            let hue_offset =
                (params.initial_hue_phase_degrees + params.hue_rotation_dps * t).rem_euclid(360.0);
            scene.write_color_params(queue, base_color.with_hue_offset_degrees(hue_offset));

            let mut frame_camera = camera.clone();
            frame_camera.auto_rotate_by(params.auto_rotate_dps * t);
            scene.write_camera(queue, &frame_camera, width, height);

            let rgba = target
                .render_frame(device, queue, background, &scene)
                .await
                .map_err(ExportError::from)?;

            writer
                .set_frame_delay(1, params.fps)
                .map_err(encode_error)?;
            writer.write_image_data(&rgba).map_err(encode_error)?;

            on_progress(frame + 1, params.num_frames);
        }

        writer.finish().map_err(encode_error)?;
    }

    Ok(PngExport {
        width,
        height,
        bytes,
    })
}

/// Like [`render_animation`], but creates its own headless GPU device.
pub async fn render_animation_standalone(
    config: &Config,
    width: u32,
    camera: &Camera,
    params: &AnimationParams,
    on_progress: impl Fn(u32, u32),
) -> Result<PngExport, AnimationExportError> {
    params.validate()?;
    validate_width(width)?;
    let (device, queue) =
        wgpu_util::create_headless_device("animation_export_device", "animation export")
            .await
            .map_err(ExportError::from)?;
    render_animation(&device, &queue, config, width, camera, params, on_progress).await
}

fn encode_error(e: png::EncodingError) -> AnimationExportError {
    AnimationExportError::Export(ExportError::Encode(e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_params() -> AnimationParams {
        AnimationParams {
            fps: 30,
            num_frames: 10,
            initial_hue_phase_degrees: 0.0,
            hue_rotation_dps: 0.0,
            auto_rotate_dps: 0.0,
        }
    }

    #[test]
    fn validates_zero_frames() {
        let mut p = default_params();
        p.num_frames = 0;
        assert!(matches!(
            p.validate(),
            Err(AnimationExportError::InvalidFrameCount(0))
        ));
    }

    #[test]
    fn validates_too_many_frames() {
        let mut p = default_params();
        p.num_frames = AnimationParams::MAX_FRAMES + 1;
        assert!(matches!(
            p.validate(),
            Err(AnimationExportError::InvalidFrameCount(n)) if n == AnimationParams::MAX_FRAMES + 1
        ));
    }

    #[test]
    fn validates_zero_fps() {
        let mut p = default_params();
        p.fps = 0;
        assert!(matches!(p.validate(), Err(AnimationExportError::ZeroFps)));
    }

    #[test]
    fn valid_params_pass() {
        assert!(default_params().validate().is_ok());
    }

    #[test]
    fn validates_max_frames_accepted() {
        let mut p = default_params();
        p.num_frames = AnimationParams::MAX_FRAMES;
        assert!(p.validate().is_ok());
    }

    #[test]
    fn validates_max_fps_accepted() {
        let p = AnimationParams {
            fps: u16::MAX,
            ..default_params()
        };
        assert!(p.validate().is_ok());
    }

    #[test]
    fn validates_nan_initial_hue() {
        let p = AnimationParams {
            initial_hue_phase_degrees: f32::NAN,
            ..default_params()
        };
        assert!(matches!(
            p.validate(),
            Err(AnimationExportError::NonFiniteValue(
                "initial_hue_phase_degrees"
            ))
        ));
    }

    #[test]
    fn validates_inf_hue_rotation_dps() {
        let p = AnimationParams {
            hue_rotation_dps: f32::INFINITY,
            ..default_params()
        };
        assert!(matches!(
            p.validate(),
            Err(AnimationExportError::NonFiniteValue("hue_rotation_dps"))
        ));
    }

    #[test]
    fn validates_inf_auto_rotate_dps() {
        let p = AnimationParams {
            auto_rotate_dps: f32::NEG_INFINITY,
            ..default_params()
        };
        assert!(matches!(
            p.validate(),
            Err(AnimationExportError::NonFiniteValue("auto_rotate_dps"))
        ));
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod gpu_tests {
    use super::*;
    use lsystem_core::{Dimensions, GenerationConfig};
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
                background: lsystem_core::Rgb::new(0, 0, 0),
                line: lsystem_core::LineColorConfig::Solid(lsystem_core::Rgb::new(255, 255, 255)),
            },
        }
    }

    #[test]
    fn apng_structure_matches_params() {
        let params = AnimationParams {
            fps: 12,
            num_frames: 3,
            initial_hue_phase_degrees: 0.0,
            hue_rotation_dps: 0.0,
            auto_rotate_dps: 0.0,
        };
        let export = pollster::block_on(render_animation_standalone(
            &trivial_config(),
            256,
            &crate::camera::Camera::default(),
            &params,
            |_, _| {},
        ))
        .expect("render_animation_standalone failed");

        assert_eq!(export.width, 256);
        let decoder = png::Decoder::new(std::io::Cursor::new(export.bytes.as_slice()));
        let reader = decoder.read_info().unwrap();
        let info = reader.info();
        assert_eq!((info.width, info.height), (export.width, export.height));
        assert_eq!(
            info.animation_control()
                .expect("missing animation control")
                .num_frames,
            3
        );
        // Each frame delay = 1/12 s → numerator=1, denominator=12
        let fc = info
            .frame_control()
            .expect("missing frame control for first frame");
        assert_eq!(fc.delay_num, 1);
        assert_eq!(fc.delay_den, 12);
    }
}
