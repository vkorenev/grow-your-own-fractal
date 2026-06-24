use glam::{Vec2, Vec3, Vec4};
use lsystem_core::{LineColorConfig, Segment2DWithTopologicalDepth, Segment3DWithTopologicalDepth};

use crate::line_renderer::{
    ColorParams, Segment2D, Segment3D, TopologicalDepthSegment2D, TopologicalDepthSegment3D,
    Transform,
};

pub struct SegmentData {
    pub segments: Vec<Segment2D>,
}

pub struct SegmentDataBuilder {
    segments: Vec<Segment2D>,
}

impl SegmentDataBuilder {
    pub fn new() -> Self {
        Self {
            segments: Vec::new(),
        }
    }

    pub fn push_segment(&mut self, [a, b]: [Vec2; 2]) {
        self.segments.push(Segment2D { start: a, end: b });
    }

    pub fn finish(self) -> SegmentData {
        SegmentData {
            segments: self.segments,
        }
    }
}

impl Default for SegmentDataBuilder {
    fn default() -> Self {
        Self::new()
    }
}

pub fn geometry_to_segments(segments: impl Iterator<Item = [Vec2; 2]>) -> SegmentData {
    let mut builder = SegmentDataBuilder::new();
    for segment in segments {
        builder.push_segment(segment);
    }
    builder.finish()
}

pub struct TopologicalDepthSegmentData {
    pub segments: Vec<TopologicalDepthSegment2D>,
    max_topological_depth: u32,
}

impl TopologicalDepthSegmentData {
    pub fn max_topological_depth(&self) -> u32 {
        self.max_topological_depth
    }
}

pub struct TopologicalDepthSegmentDataBuilder {
    max_topological_depth: u32,
    segments: Vec<TopologicalDepthSegment2D>,
}

impl TopologicalDepthSegmentDataBuilder {
    pub fn new() -> Self {
        Self {
            max_topological_depth: 0,
            segments: Vec::new(),
        }
    }

    pub fn push_segment(&mut self, segment: Segment2DWithTopologicalDepth) {
        let [a, b] = segment.points;
        self.max_topological_depth = self.max_topological_depth.max(segment.topological_depth);
        self.segments.push(TopologicalDepthSegment2D {
            start: a,
            end: b,
            topological_depth: segment.topological_depth,
        });
    }

    pub fn finish(self) -> TopologicalDepthSegmentData {
        TopologicalDepthSegmentData {
            segments: self.segments,
            max_topological_depth: self.max_topological_depth,
        }
    }
}

impl Default for TopologicalDepthSegmentDataBuilder {
    fn default() -> Self {
        Self::new()
    }
}

pub fn geometry_to_depth_segments(
    segments: impl Iterator<Item = Segment2DWithTopologicalDepth>,
) -> TopologicalDepthSegmentData {
    let mut builder = TopologicalDepthSegmentDataBuilder::new();
    for segment in segments {
        builder.push_segment(segment);
    }
    builder.finish()
}

pub struct SegmentData3D {
    pub segments: Vec<Segment3D>,
}

pub struct SegmentDataBuilder3D {
    segments: Vec<Segment3D>,
}

impl SegmentDataBuilder3D {
    pub fn new() -> Self {
        Self {
            segments: Vec::new(),
        }
    }

    pub fn push_segment(&mut self, [a, b]: [Vec3; 2]) {
        self.segments.push(Segment3D { start: a, end: b });
    }

    pub fn finish(self) -> SegmentData3D {
        SegmentData3D {
            segments: self.segments,
        }
    }
}

impl Default for SegmentDataBuilder3D {
    fn default() -> Self {
        Self::new()
    }
}

pub fn geometry_to_segments_3d(segments: impl Iterator<Item = [Vec3; 2]>) -> SegmentData3D {
    let mut builder = SegmentDataBuilder3D::new();
    for segment in segments {
        builder.push_segment(segment);
    }
    builder.finish()
}

pub struct TopologicalDepthSegmentData3D {
    pub segments: Vec<TopologicalDepthSegment3D>,
    max_topological_depth: u32,
}

impl TopologicalDepthSegmentData3D {
    pub fn max_topological_depth(&self) -> u32 {
        self.max_topological_depth
    }
}

pub struct TopologicalDepthSegmentDataBuilder3D {
    max_topological_depth: u32,
    segments: Vec<TopologicalDepthSegment3D>,
}

impl TopologicalDepthSegmentDataBuilder3D {
    pub fn new() -> Self {
        Self {
            max_topological_depth: 0,
            segments: Vec::new(),
        }
    }

    pub fn push_segment(&mut self, segment: Segment3DWithTopologicalDepth) {
        let [a, b] = segment.points;
        self.max_topological_depth = self.max_topological_depth.max(segment.topological_depth);
        self.segments.push(TopologicalDepthSegment3D {
            start: a,
            end: b,
            topological_depth: segment.topological_depth,
        });
    }

    pub fn finish(self) -> TopologicalDepthSegmentData3D {
        TopologicalDepthSegmentData3D {
            segments: self.segments,
            max_topological_depth: self.max_topological_depth,
        }
    }
}

impl Default for TopologicalDepthSegmentDataBuilder3D {
    fn default() -> Self {
        Self::new()
    }
}

pub fn geometry_to_depth_segments_3d(
    segments: impl Iterator<Item = Segment3DWithTopologicalDepth>,
) -> TopologicalDepthSegmentData3D {
    let mut builder = TopologicalDepthSegmentDataBuilder3D::new();
    for segment in segments {
        builder.push_segment(segment);
    }
    builder.finish()
}

/// Builds the GPU color uniform for the selected line color mode.
///
/// Pass `max_topological_depth` as `Some(n)` when the caller uploaded
/// topological-depth segment instances; only then is the `DepthGradient` mode
/// valid. `None` forces a plain traversal gradient even when `topological_depth`
/// is authored in the config, keeping the vertex shader mode consistent with the
/// uploaded geometry.
pub fn color_params_from_config(
    line: &LineColorConfig,
    total_segments: u32,
    max_topological_depth: Option<u32>,
) -> ColorParams {
    match *line {
        LineColorConfig::Solid(c) => ColorParams::solid(total_segments, rgb_to_rgba(c.to_array())),
        LineColorConfig::Gradient {
            start,
            end,
            topological_depth,
        } => {
            if topological_depth && let Some(max_depth) = max_topological_depth {
                return ColorParams::depth_gradient(
                    total_segments,
                    max_depth,
                    rgb_to_rgba(start.to_array()),
                    rgb_to_rgba(end.to_array()),
                );
            }
            ColorParams::gradient(
                total_segments,
                rgb_to_rgba(start.to_array()),
                rgb_to_rgba(end.to_array()),
            )
        }
        LineColorConfig::HueCycle { initial } => {
            let (hue_start, saturation, value) = initial.to_hsv();
            ColorParams::hue_cycle(total_segments, hue_start, saturation, value)
        }
    }
}

fn rgb_to_rgba([r, g, b]: [f32; 3]) -> Vec4 {
    Vec4::new(r, g, b, 1.0)
}

pub fn fitted_pixels_per_unit(
    bounds_min: [f32; 2],
    bounds_max: [f32; 2],
    width: u32,
    height: u32,
) -> f32 {
    let geom_w = (bounds_max[0] - bounds_min[0]).max(1.0);
    let geom_h = (bounds_max[1] - bounds_min[1]).max(1.0);
    (width as f32 / geom_w).min(height as f32 / geom_h) * 0.9
}

pub fn viewport_transform(
    bounds_min: [f32; 2],
    bounds_max: [f32; 2],
    width: u32,
    height: u32,
    pan: [f32; 2],
    zoom: f32,
) -> Transform {
    let cx = (bounds_min[0] + bounds_max[0]) * 0.5;
    let cy = (bounds_min[1] + bounds_max[1]) * 0.5;
    let ppu = fitted_pixels_per_unit(bounds_min, bounds_max, width, height) * zoom;
    let sx = ppu * 2.0 / width as f32;
    let sy = ppu * 2.0 / height as f32;
    Transform {
        scale: Vec2::new(sx, sy),
        offset: Vec2::new((-cx + pan[0]) * sx, (-cy + pan[1]) * sy),
    }
}

#[cfg(test)]
mod tests {
    use lsystem_core::{Dimensions, GenerationConfig, Rgb, generate};
    use std::collections::BTreeMap;

    use super::*;

    const EPS: f32 = 1e-5;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < EPS
    }

    fn hex_rgba(hex: Rgb) -> Vec4 {
        let [r, g, b] = hex.to_array();
        Vec4::new(r, g, b, 1.0)
    }

    #[test]
    fn solid_maps_to_mode_solid_with_color() {
        let color = Rgb::new(0x1a, 0x33, 0x4d);
        let params = color_params_from_config(&LineColorConfig::Solid(color), 10, None);

        assert_eq!(params.total_segments, 10);
        assert_eq!(params.color_start, hex_rgba(color));
        assert_eq!(params.max_topological_depth, 0);
    }

    #[test]
    fn gradient_maps_to_mode_gradient_with_start_and_end_colors() {
        let start = Rgb::new(0x1a, 0x33, 0x4d);
        let end = Rgb::new(0xb3, 0xcc, 0xe5);
        let params = color_params_from_config(
            &LineColorConfig::Gradient {
                start,
                end,
                topological_depth: false,
            },
            7,
            None,
        );

        assert_eq!(params.total_segments, 7);
        assert_eq!(params.color_start, hex_rgba(start));
        assert_eq!(params.color_end, hex_rgba(end));
        assert_eq!(params.max_topological_depth, 0);
    }

    #[test]
    fn hue_cycle_initial_rgb_maps_to_hsv_uniforms() {
        // Rgb::new(0x40, 0x80, 0x80) ≈ (0.251, 0.502, 0.502) → hue≈180°, sat≈0.5, val≈0.502
        let initial = Rgb::new(0x40, 0x80, 0x80);
        let params = color_params_from_config(&LineColorConfig::HueCycle { initial }, 9, None);

        assert_eq!(params.total_segments, 9);
        let (hue, sat, val) = initial.to_hsv();
        assert!(close(params.hue_start, hue));
        assert!(close(params.saturation, sat));
        assert!(close(params.value, val));
    }

    #[test]
    fn depth_gradient_maps_to_mode_three_with_max_topological_depth() {
        let start = Rgb::new(0x1a, 0x33, 0x4d);
        let end = Rgb::new(0xb3, 0xcc, 0xe5);
        let params = color_params_from_config(
            &LineColorConfig::Gradient {
                start,
                end,
                topological_depth: true,
            },
            5,
            Some(3),
        );

        assert_eq!(params.total_segments, 5);
        assert_eq!(params.max_topological_depth, 3);
        assert_eq!(params.color_start, hex_rgba(start));
        assert_eq!(params.color_end, hex_rgba(end));
    }

    #[test]
    fn depth_gradient_preserves_zero_max_topological_depth() {
        let start = Rgb::new(0x1a, 0x33, 0x4d);
        let end = Rgb::new(0xb3, 0xcc, 0xe5);
        let params = color_params_from_config(
            &LineColorConfig::Gradient {
                start,
                end,
                topological_depth: true,
            },
            1,
            Some(0),
        );

        assert_eq!(params.max_topological_depth, 0);
    }

    #[test]
    fn depth_gradient_without_depth_geometry_falls_back_to_traversal() {
        let start = Rgb::new(0x1a, 0x33, 0x4d);
        let end = Rgb::new(0xb3, 0xcc, 0xe5);
        let traversal_params = color_params_from_config(
            &LineColorConfig::Gradient {
                start,
                end,
                topological_depth: true,
            },
            5,
            None,
        );
        let expected = color_params_from_config(
            &LineColorConfig::Gradient {
                start,
                end,
                topological_depth: false,
            },
            5,
            None,
        );
        assert_eq!(traversal_params.color_start, expected.color_start);
        assert_eq!(traversal_params.color_end, expected.color_end);
        assert_eq!(traversal_params.total_segments, expected.total_segments);
        assert_eq!(
            traversal_params.max_topological_depth,
            expected.max_topological_depth
        );
    }

    fn cfg(axiom: &str) -> GenerationConfig {
        GenerationConfig {
            dimensions: Dimensions::TwoD,
            axiom: axiom.to_string(),
            iterations: 0,
            angle: 90.0,
            step: 1.0,
            initial_heading: 0.0,
            rules: BTreeMap::new(),
        }
    }

    #[test]
    fn empty_geometry_collects_no_segments() {
        let SegmentData { segments } = geometry_to_segments(generate(&cfg("A")));
        assert!(segments.is_empty());
    }

    #[test]
    fn empty_depth_geometry_collects_no_segments_and_zero_max_depth() {
        let data =
            geometry_to_depth_segments(lsystem_core::generate_with_topological_depth(&cfg("A")));

        assert!(data.segments.is_empty());
        assert_eq!(data.max_topological_depth(), 0);
    }

    #[test]
    fn empty_depth_geometry_3d_collects_no_segments_and_zero_max_depth() {
        let data = geometry_to_depth_segments_3d(lsystem_core::generate_3d_with_topological_depth(
            &GenerationConfig {
                dimensions: Dimensions::ThreeD,
                axiom: "A".to_string(),
                iterations: 0,
                angle: 90.0,
                step: 1.0,
                initial_heading: 0.0,
                rules: BTreeMap::new(),
            },
        ));

        assert!(data.segments.is_empty());
        assert_eq!(data.max_topological_depth(), 0);
    }

    #[test]
    fn single_segment_produces_one_segment_record() {
        let SegmentData { segments } = geometry_to_segments(generate(&cfg("F")));
        assert_eq!(segments.len(), 1);
        assert!(close(segments[0].start[0], 0.0) && close(segments[0].start[1], 0.0));
        assert!(close(segments[0].end[0], 1.0) && close(segments[0].end[1], 0.0));
    }

    #[test]
    fn geometry_collects_all_segments() {
        let SegmentData { segments } = geometry_to_segments(generate(&cfg("F+F-F")));
        assert_eq!(segments.len(), 3);
    }

    #[test]
    fn topological_depth_segments_preserve_depth_and_compute_max() {
        let data = geometry_to_depth_segments(lsystem_core::generate_with_topological_depth(&cfg(
            "F[+F]F",
        )));

        assert_eq!(data.segments.len(), 3);
        assert_eq!(data.segments[0].topological_depth, 0);
        assert_eq!(data.segments[1].topological_depth, 1);
        assert_eq!(data.segments[2].topological_depth, 1);
        assert_eq!(data.max_topological_depth(), 1);
    }

    #[test]
    fn topological_depth_segments_3d_preserve_depth_and_compute_max() {
        let data = geometry_to_depth_segments_3d(lsystem_core::generate_3d_with_topological_depth(
            &GenerationConfig {
                dimensions: Dimensions::ThreeD,
                axiom: "F[+F]F".to_string(),
                iterations: 0,
                angle: 90.0,
                step: 1.0,
                initial_heading: 0.0,
                rules: BTreeMap::new(),
            },
        ));

        assert_eq!(data.segments.len(), 3);
        assert_eq!(data.segments[0].topological_depth, 0);
        assert_eq!(data.segments[1].topological_depth, 1);
        assert_eq!(data.segments[2].topological_depth, 1);
        assert_eq!(data.max_topological_depth(), 1);
    }

    #[test]
    fn viewport_transform_fits_and_centers_bounds() {
        let t = viewport_transform([1.0, 0.0], [5.0, 4.0], 200, 200, [0.0, 0.0], 1.0);
        assert!(close(3.0 * t.scale[0] + t.offset[0], 0.0));
        assert!(close(2.0 * t.scale[1] + t.offset[1], 0.0));
        assert!(close(4.0 * t.scale[0], 1.8));
        assert!(close(4.0 * t.scale[1], 1.8));
    }

    #[test]
    fn viewport_transform_keeps_degenerate_bounds_finite() {
        let t = viewport_transform([5.0, 3.0], [5.0, 3.0], 100, 100, [0.0, 0.0], 1.0);
        assert!(t.scale[0].is_finite() && t.scale[0] > 0.0);
        assert!(t.scale[1].is_finite() && t.scale[1] > 0.0);
        assert!(close(5.0 * t.scale[0] + t.offset[0], 0.0));
        assert!(close(3.0 * t.scale[1] + t.offset[1], 0.0));
    }
}
