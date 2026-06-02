use glam::{Vec2, Vec3};
use lsystem_core::{
    LineColorConfig, Rgb, Segment2DWithTopologicalDepth, Segment3DWithTopologicalDepth,
    color_util::rgb_to_hsv,
};

use crate::line_renderer::{
    ColorMode, ColorParams, Segment2D, Segment3D, TopologicalDepthSegment2D,
    TopologicalDepthSegment3D, Transform,
};

pub struct SegmentData {
    pub segments: Vec<Segment2D>,
    pub bounds_min: [f32; 2],
    pub bounds_max: [f32; 2],
}

pub struct SegmentDataBuilder {
    min_x: f32,
    min_y: f32,
    max_x: f32,
    max_y: f32,
    segments: Vec<Segment2D>,
}

impl SegmentDataBuilder {
    pub fn new() -> Self {
        Self {
            min_x: f32::INFINITY,
            min_y: f32::INFINITY,
            max_x: f32::NEG_INFINITY,
            max_y: f32::NEG_INFINITY,
            segments: Vec::new(),
        }
    }

    pub fn push_segment(&mut self, [a, b]: [Vec2; 2]) {
        self.min_x = self.min_x.min(a.x).min(b.x);
        self.min_y = self.min_y.min(a.y).min(b.y);
        self.max_x = self.max_x.max(a.x).max(b.x);
        self.max_y = self.max_y.max(a.y).max(b.y);
        self.segments.push(Segment2D {
            start: [a.x, a.y],
            end: [b.x, b.y],
        });
    }

    pub fn finish(self) -> SegmentData {
        let (bounds_min, bounds_max) = if self.min_x.is_infinite() {
            ([-1.0, -1.0], [1.0, 1.0])
        } else {
            ([self.min_x, self.min_y], [self.max_x, self.max_y])
        };

        SegmentData {
            segments: self.segments,
            bounds_min,
            bounds_max,
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
    pub bounds_min: [f32; 2],
    pub bounds_max: [f32; 2],
    max_topological_depth: u32,
}

impl TopologicalDepthSegmentData {
    pub fn max_topological_depth(&self) -> u32 {
        self.max_topological_depth
    }
}

pub struct TopologicalDepthSegmentDataBuilder {
    min_x: f32,
    min_y: f32,
    max_x: f32,
    max_y: f32,
    max_topological_depth: u32,
    segments: Vec<TopologicalDepthSegment2D>,
}

impl TopologicalDepthSegmentDataBuilder {
    pub fn new() -> Self {
        Self {
            min_x: f32::INFINITY,
            min_y: f32::INFINITY,
            max_x: f32::NEG_INFINITY,
            max_y: f32::NEG_INFINITY,
            max_topological_depth: 0,
            segments: Vec::new(),
        }
    }

    pub fn push_segment(&mut self, segment: Segment2DWithTopologicalDepth) {
        let [a, b] = segment.points;
        self.min_x = self.min_x.min(a.x).min(b.x);
        self.min_y = self.min_y.min(a.y).min(b.y);
        self.max_x = self.max_x.max(a.x).max(b.x);
        self.max_y = self.max_y.max(a.y).max(b.y);
        self.max_topological_depth = self.max_topological_depth.max(segment.topological_depth);
        self.segments.push(TopologicalDepthSegment2D {
            start: [a.x, a.y],
            end: [b.x, b.y],
            topological_depth: segment.topological_depth,
        });
    }

    pub fn finish(self) -> TopologicalDepthSegmentData {
        let (bounds_min, bounds_max) = if self.min_x.is_infinite() {
            ([-1.0, -1.0], [1.0, 1.0])
        } else {
            ([self.min_x, self.min_y], [self.max_x, self.max_y])
        };

        TopologicalDepthSegmentData {
            segments: self.segments,
            bounds_min,
            bounds_max,
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
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
}

pub struct SegmentDataBuilder3D {
    min_x: f32,
    min_y: f32,
    min_z: f32,
    max_x: f32,
    max_y: f32,
    max_z: f32,
    segments: Vec<Segment3D>,
}

impl SegmentDataBuilder3D {
    pub fn new() -> Self {
        Self {
            min_x: f32::INFINITY,
            min_y: f32::INFINITY,
            min_z: f32::INFINITY,
            max_x: f32::NEG_INFINITY,
            max_y: f32::NEG_INFINITY,
            max_z: f32::NEG_INFINITY,
            segments: Vec::new(),
        }
    }

    pub fn push_segment(&mut self, [a, b]: [Vec3; 2]) {
        self.min_x = self.min_x.min(a.x).min(b.x);
        self.min_y = self.min_y.min(a.y).min(b.y);
        self.min_z = self.min_z.min(a.z).min(b.z);
        self.max_x = self.max_x.max(a.x).max(b.x);
        self.max_y = self.max_y.max(a.y).max(b.y);
        self.max_z = self.max_z.max(a.z).max(b.z);
        self.segments.push(Segment3D {
            start: [a.x, a.y, a.z],
            end: [b.x, b.y, b.z],
        });
    }

    pub fn finish(self) -> SegmentData3D {
        let (bounds_min, bounds_max) = if self.min_x.is_infinite() {
            ([-1.0, -1.0, -1.0], [1.0, 1.0, 1.0])
        } else {
            (
                [self.min_x, self.min_y, self.min_z],
                [self.max_x, self.max_y, self.max_z],
            )
        };

        SegmentData3D {
            segments: self.segments,
            bounds_min,
            bounds_max,
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
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
    max_topological_depth: u32,
}

impl TopologicalDepthSegmentData3D {
    pub fn max_topological_depth(&self) -> u32 {
        self.max_topological_depth
    }
}

pub struct TopologicalDepthSegmentDataBuilder3D {
    min_x: f32,
    min_y: f32,
    min_z: f32,
    max_x: f32,
    max_y: f32,
    max_z: f32,
    max_topological_depth: u32,
    segments: Vec<TopologicalDepthSegment3D>,
}

impl TopologicalDepthSegmentDataBuilder3D {
    pub fn new() -> Self {
        Self {
            min_x: f32::INFINITY,
            min_y: f32::INFINITY,
            min_z: f32::INFINITY,
            max_x: f32::NEG_INFINITY,
            max_y: f32::NEG_INFINITY,
            max_z: f32::NEG_INFINITY,
            max_topological_depth: 0,
            segments: Vec::new(),
        }
    }

    pub fn push_segment(&mut self, segment: Segment3DWithTopologicalDepth) {
        let [a, b] = segment.points;
        self.min_x = self.min_x.min(a.x).min(b.x);
        self.min_y = self.min_y.min(a.y).min(b.y);
        self.min_z = self.min_z.min(a.z).min(b.z);
        self.max_x = self.max_x.max(a.x).max(b.x);
        self.max_y = self.max_y.max(a.y).max(b.y);
        self.max_z = self.max_z.max(a.z).max(b.z);
        self.max_topological_depth = self.max_topological_depth.max(segment.topological_depth);
        self.segments.push(TopologicalDepthSegment3D {
            start: [a.x, a.y, a.z],
            end: [b.x, b.y, b.z],
            topological_depth: segment.topological_depth,
        });
    }

    pub fn finish(self) -> TopologicalDepthSegmentData3D {
        let (bounds_min, bounds_max) = if self.min_x.is_infinite() {
            ([-1.0, -1.0, -1.0], [1.0, 1.0, 1.0])
        } else {
            (
                [self.min_x, self.min_y, self.min_z],
                [self.max_x, self.max_y, self.max_z],
            )
        };

        TopologicalDepthSegmentData3D {
            segments: self.segments,
            bounds_min,
            bounds_max,
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

pub fn color_params_from_config(
    line: &LineColorConfig,
    total_segments: u32,
    max_topological_depth: u32,
) -> ColorParams {
    match *line {
        LineColorConfig::Solid { color: c } => ColorParams {
            mode: ColorMode::Solid,
            total_segments,
            max_topological_depth: 0,
            color_start: rgb_to_rgba(c),
            ..Default::default()
        },
        LineColorConfig::Gradient { start, end } => ColorParams {
            mode: ColorMode::Gradient,
            total_segments,
            max_topological_depth: 0,
            color_start: rgb_to_rgba(start),
            color_end: rgb_to_rgba(end),
            ..Default::default()
        },
        LineColorConfig::HueCycle { initial } => {
            let (hue_start, saturation, value) = rgb_to_hsv(initial);
            ColorParams {
                mode: ColorMode::HueCycle,
                total_segments,
                max_topological_depth: 0,
                hue_start,
                saturation,
                value,
                ..Default::default()
            }
        }
        LineColorConfig::DepthGradient { start, end } => ColorParams {
            mode: ColorMode::DepthGradient,
            total_segments,
            max_topological_depth,
            color_start: rgb_to_rgba(start),
            color_end: rgb_to_rgba(end),
            ..Default::default()
        },
    }
}

fn rgb_to_rgba(rgb: Rgb) -> [f32; 4] {
    let [r, g, b] = rgb.to_array();
    [r, g, b, 1.0]
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
        scale: [sx, sy],
        offset: [(-cx + pan[0]) * sx, (-cy + pan[1]) * sy],
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

    fn rgb(components: [f32; 3]) -> Rgb {
        Rgb::try_from(components).unwrap()
    }

    #[test]
    fn solid_maps_to_mode_solid_with_color() {
        let params = color_params_from_config(
            &LineColorConfig::Solid {
                color: rgb([0.2, 0.4, 0.6]),
            },
            10,
            0,
        );

        assert_eq!(params.mode, ColorMode::Solid);
        assert_eq!(params.total_segments, 10);
        assert_eq!(params.color_start, [0.2, 0.4, 0.6, 1.0]);
        assert_eq!(params.max_topological_depth, 0);
    }

    #[test]
    fn gradient_maps_to_mode_gradient_with_start_and_end_colors() {
        let params = color_params_from_config(
            &LineColorConfig::Gradient {
                start: rgb([0.1, 0.2, 0.3]),
                end: rgb([0.7, 0.8, 0.9]),
            },
            7,
            0,
        );

        assert_eq!(params.mode, ColorMode::Gradient);
        assert_eq!(params.total_segments, 7);
        assert_eq!(params.color_start, [0.1, 0.2, 0.3, 1.0]);
        assert_eq!(params.color_end, [0.7, 0.8, 0.9, 1.0]);
        assert_eq!(params.max_topological_depth, 0);
    }

    #[test]
    fn hue_cycle_initial_rgb_maps_to_hsv_uniforms() {
        let params = color_params_from_config(
            &LineColorConfig::HueCycle {
                initial: rgb([0.25, 0.5, 0.5]),
            },
            9,
            0,
        );

        assert_eq!(params.mode, ColorMode::HueCycle);
        assert_eq!(params.total_segments, 9);
        assert!(close(params.hue_start, 180.0));
        assert!(close(params.saturation, 0.5));
        assert!(close(params.value, 0.5));
    }

    #[test]
    fn depth_gradient_maps_to_mode_three_with_max_topological_depth() {
        let params = color_params_from_config(
            &LineColorConfig::DepthGradient {
                start: rgb([0.1, 0.2, 0.3]),
                end: rgb([0.7, 0.8, 0.9]),
            },
            5,
            3,
        );

        assert_eq!(params.mode, ColorMode::DepthGradient);
        assert_eq!(params.total_segments, 5);
        assert_eq!(params.max_topological_depth, 3);
        assert_eq!(params.color_start, [0.1, 0.2, 0.3, 1.0]);
        assert_eq!(params.color_end, [0.7, 0.8, 0.9, 1.0]);
    }

    #[test]
    fn depth_gradient_preserves_zero_max_topological_depth() {
        let params = color_params_from_config(
            &LineColorConfig::DepthGradient {
                start: rgb([0.1, 0.2, 0.3]),
                end: rgb([0.7, 0.8, 0.9]),
            },
            1,
            0,
        );

        assert_eq!(params.mode, ColorMode::DepthGradient);
        assert_eq!(params.max_topological_depth, 0);
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
    fn empty_geometry_uses_fallback_bounds() {
        let SegmentData {
            segments,
            bounds_min,
            bounds_max,
        } = geometry_to_segments(generate(&cfg("A")));
        assert!(segments.is_empty());
        assert!(close(bounds_min[0], -1.0) && close(bounds_min[1], -1.0));
        assert!(close(bounds_max[0], 1.0) && close(bounds_max[1], 1.0));
    }

    #[test]
    fn empty_depth_geometry_uses_fallback_bounds_and_zero_max_depth() {
        let data =
            geometry_to_depth_segments(lsystem_core::generate_with_topological_depth(&cfg("A")));

        assert!(data.segments.is_empty());
        assert_eq!(data.max_topological_depth(), 0);
        assert!(close(data.bounds_min[0], -1.0) && close(data.bounds_min[1], -1.0));
        assert!(close(data.bounds_max[0], 1.0) && close(data.bounds_max[1], 1.0));
    }

    #[test]
    fn empty_depth_geometry_3d_uses_fallback_bounds_and_zero_max_depth() {
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
        assert_eq!(data.bounds_min, [-1.0, -1.0, -1.0]);
        assert_eq!(data.bounds_max, [1.0, 1.0, 1.0]);
    }

    #[test]
    fn single_segment_produces_one_segment_record_and_tight_bounds() {
        let SegmentData {
            segments,
            bounds_min,
            bounds_max,
        } = geometry_to_segments(generate(&cfg("F")));
        assert_eq!(segments.len(), 1);
        assert!(close(segments[0].start[0], 0.0) && close(segments[0].start[1], 0.0));
        assert!(close(segments[0].end[0], 1.0) && close(segments[0].end[1], 0.0));
        assert!(close(bounds_min[0], 0.0) && close(bounds_min[1], 0.0));
        assert!(close(bounds_max[0], 1.0) && close(bounds_max[1], 0.0));
    }

    #[test]
    fn bounds_are_tight_over_all_segments() {
        let SegmentData {
            segments,
            bounds_min,
            bounds_max,
        } = geometry_to_segments(generate(&cfg("F+F-F")));
        assert_eq!(segments.len(), 3);
        assert!(close(bounds_min[0], 0.0) && close(bounds_min[1], 0.0));
        assert!(close(bounds_max[0], 2.0) && close(bounds_max[1], 1.0));
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
        assert!(close(data.bounds_min[0], 0.0) && close(data.bounds_min[1], 0.0));
        assert!(close(data.bounds_max[0], 2.0) && close(data.bounds_max[1], 1.0));
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
        assert!(close(data.bounds_min[0], 0.0) && close(data.bounds_min[1], 0.0));
        assert!(close(data.bounds_max[0], 2.0) && close(data.bounds_max[1], 1.0));
        assert!(close(data.bounds_min[2], 0.0) && close(data.bounds_max[2], 0.0));
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
