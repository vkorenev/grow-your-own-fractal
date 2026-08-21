use glam::{Vec2, Vec4};
use lsystem_core::{BoundingCylinder3D, Bounds2D, LineColorConfig, Rgb};

use crate::line_renderer::{
    ColorParams, Segment2D, Segment3D, TopologicalDepthSegment2D, TopologicalDepthSegment3D,
    Transform,
};

/// Segment records collected for a native scene, with their bounds.
///
/// `bounds` contains every point in `segments`; for empty geometry it falls
/// back to the renderer's unit bounds for the matching dimension.
pub struct CollectedSegmentData<R, B> {
    pub segments: Vec<R>,
    pub bounds: B,
}

/// Like `CollectedSegmentData`, plus the maximum topological depth seen
/// across `segments`.
///
/// `bounds` contains every point in `segments`; for empty geometry it falls
/// back to the renderer's unit bounds for the matching dimension.
pub struct CollectedDepthSegmentData<R, B> {
    pub segments: Vec<R>,
    pub bounds: B,
    pub max_topological_depth: u32,
}

pub type SegmentData2D = CollectedSegmentData<Segment2D, Bounds2D>;
pub type SegmentData3D = CollectedSegmentData<Segment3D, BoundingCylinder3D>;
pub type TopologicalDepthSegmentData2D =
    CollectedDepthSegmentData<TopologicalDepthSegment2D, Bounds2D>;
pub type TopologicalDepthSegmentData3D =
    CollectedDepthSegmentData<TopologicalDepthSegment3D, BoundingCylinder3D>;

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

/// Converts to an opaque `wgpu::Color` for use as a render-pass clear color.
pub fn rgb_to_wgpu_color(color: Rgb) -> wgpu::Color {
    let [r, g, b] = color.to_array();
    wgpu::Color {
        r: r as f64,
        g: g as f64,
        b: b as f64,
        a: 1.0,
    }
}

/// Fraction of the viewport the fitted geometry fills along its
/// tightest-constrained axis, leaving the rest as a visual margin so the
/// fractal doesn't touch the canvas edge. Shared by the 2D and 3D fits.
pub(crate) const VIEWPORT_FILL_FRACTION: f32 = 0.9;

pub fn fitted_pixels_per_unit(bounds: Bounds2D, width: u32, height: u32) -> f32 {
    let geom_w = (bounds.max.x - bounds.min.x).max(1.0);
    let geom_h = (bounds.max.y - bounds.min.y).max(1.0);
    (width as f32 / geom_w).min(height as f32 / geom_h) * VIEWPORT_FILL_FRACTION
}

pub fn viewport_transform(
    bounds: Bounds2D,
    width: u32,
    height: u32,
    pan: Vec2,
    zoom: f32,
) -> Transform {
    let center = (bounds.min + bounds.max) * 0.5;
    let ppu = fitted_pixels_per_unit(bounds, width, height) * zoom;
    let sx = ppu * 2.0 / width as f32;
    let sy = ppu * 2.0 / height as f32;
    Transform {
        scale: Vec2::new(sx, sy),
        offset: Vec2::new((-center.x + pan[0]) * sx, (-center.y + pan[1]) * sy),
    }
}

#[cfg(test)]
mod tests {
    use glam::Vec3;
    use lsystem_core::{
        AnyCompiledGeneration, CompiledGeneration2D, CompiledGeneration3D, D2, D3,
        DepthSegmentStream, Dimensions, GenerationConfig, PreparedGeneration, Rgb, SegmentStream,
        TemplateDimension,
    };
    use std::collections::BTreeMap;
    use std::ops::Index;

    use super::*;
    use crate::line_renderer::RenderDimension;

    const EPS: f32 = 1e-5;

    fn collect_plain_segments<D: RenderDimension + TemplateDimension>(
        stream: SegmentStream<'_, D>,
    ) -> CollectedSegmentData<D::PlainRecord, D::Bounds> {
        let mut segments = Vec::new();
        let bounds = stream.drain(|[a, b]| segments.push(D::plain_record(a, b)));
        CollectedSegmentData {
            segments,
            bounds: bounds.unwrap_or_else(D::empty_scene_bounds),
        }
    }

    fn collect_depth_segments<D: RenderDimension + TemplateDimension>(
        stream: DepthSegmentStream<'_, D>,
    ) -> CollectedDepthSegmentData<D::DepthRecord, D::Bounds> {
        let mut segments = Vec::new();
        let summary = stream.drain(|segment| {
            let [a, b] = segment.points;
            segments.push(D::depth_record(a, b, segment.topological_depth));
        });
        CollectedDepthSegmentData {
            segments,
            bounds: summary.bounds.unwrap_or_else(D::empty_scene_bounds),
            max_topological_depth: summary.max_topological_depth,
        }
    }

    fn compile_2d(config: &GenerationConfig) -> CompiledGeneration2D {
        let AnyCompiledGeneration::TwoD(generation) = config.compile() else {
            panic!("expected a 2D generation config")
        };
        generation
    }

    fn compile_3d(config: &GenerationConfig) -> CompiledGeneration3D {
        let AnyCompiledGeneration::ThreeD(generation) = config.compile() else {
            panic!("expected a 3D generation config")
        };
        generation
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < EPS
    }

    trait TestBounds {
        fn close_to(self, other: Self, eps: f32) -> bool;
    }

    impl TestBounds for Bounds2D {
        fn close_to(self, other: Self, eps: f32) -> bool {
            self.min.abs_diff_eq(other.min, eps) && self.max.abs_diff_eq(other.max, eps)
        }
    }

    impl TestBounds for BoundingCylinder3D {
        fn close_to(self, other: Self, eps: f32) -> bool {
            self.center_xz.abs_diff_eq(other.center_xz, eps)
                && (self.radius - other.radius).abs() < eps
                && (self.min_y - other.min_y).abs() < eps
                && (self.max_y - other.max_y).abs() < eps
        }
    }

    fn cylinder_contains(bounds: BoundingCylinder3D, point: Vec3) -> bool {
        Vec2::new(point.x, point.z).distance(bounds.center_xz) <= bounds.radius
            && point.y >= bounds.min_y
            && point.y <= bounds.max_y
    }

    fn assert_plain_3d_endpoints_are_contained(data: &SegmentData3D) {
        assert!(
            data.segments
                .iter()
                .flat_map(|segment| [segment.start, segment.end])
                .all(|point| cylinder_contains(data.bounds, point))
        );
    }

    fn assert_depth_3d_endpoints_are_contained(data: &TopologicalDepthSegmentData3D) {
        assert!(
            data.segments
                .iter()
                .flat_map(|segment| [segment.start, segment.end])
                .all(|point| cylinder_contains(data.bounds, point))
        );
    }

    /// Euclidean distance over the first `axis_count` components of a
    /// generic `D::Point` value.
    fn axis_distance<P: Index<usize, Output = f32>>(a: P, b: P, axis_count: usize) -> f32 {
        (0..axis_count)
            .map(|axis| (a[axis] - b[axis]).powi(2))
            .sum::<f32>()
            .sqrt()
    }

    fn hex_rgba(hex: Rgb) -> Vec4 {
        let [r, g, b] = hex.to_array();
        Vec4::new(r, g, b, 1.0)
    }

    #[test]
    fn stamped_plain_segments_match_interpreted() {
        let config = GenerationConfig::new(
            Dimensions::TwoD,
            "F++F++F".to_string(),
            3,
            60.0,
            1.0,
            0.0,
            BTreeMap::from([('F', "F-F++F-F".to_string())]),
        )
        .expect("balanced config");
        let stamped = PreparedGeneration::Stamped(
            compile_2d(&config).build_templates(2).expect("set builds"),
        );
        let interpreted = PreparedGeneration::Interpreted(compile_2d(&config));

        let stamped = collect_plain_segments::<D2>(stamped.segments());
        let interpreted = collect_plain_segments::<D2>(interpreted.segments());

        assert_eq!(stamped.segments.len(), interpreted.segments.len());
        for (s, i) in stamped.segments.iter().zip(&interpreted.segments) {
            assert!(s.start.distance(i.start) < 1e-3);
            assert!(s.end.distance(i.end) < 1e-3);
        }

        let config = GenerationConfig::new(
            Dimensions::ThreeD,
            "X".to_string(),
            3,
            90.0,
            1.0,
            0.0,
            BTreeMap::from([('X', r"^\XF^\XFX-F^//XFX&F+//XFX-F/X-/".to_string())]),
        )
        .expect("balanced config");
        let stamped = PreparedGeneration::Stamped(
            compile_3d(&config).build_templates(2).expect("set builds"),
        );
        let interpreted = PreparedGeneration::Interpreted(compile_3d(&config));

        let stamped = collect_plain_segments::<D3>(stamped.segments());
        let interpreted = collect_plain_segments::<D3>(interpreted.segments());

        assert_plain_3d_endpoints_are_contained(&stamped);
        assert_plain_3d_endpoints_are_contained(&interpreted);
        assert_eq!(stamped.segments.len(), interpreted.segments.len());
        for (s, i) in stamped.segments.iter().zip(&interpreted.segments) {
            assert!(s.start.distance(i.start) < 1e-3);
            assert!(s.end.distance(i.end) < 1e-3);
        }
    }

    #[test]
    fn empty_stamped_iterators_report_zero_and_collect_with_fallback_bounds() {
        let config = GenerationConfig::new(
            Dimensions::TwoD,
            "A".to_string(),
            1,
            90.0,
            1.0,
            0.0,
            BTreeMap::new(),
        )
        .expect("balanced config");
        let prepared = PreparedGeneration::Stamped(
            compile_2d(&config).build_templates(1).expect("set builds"),
        );

        assert_eq!(prepared.segments().count(), 0);

        let data = collect_plain_segments::<D2>(prepared.segments());
        assert!(data.segments.is_empty());
        assert_eq!(data.bounds, D2::empty_scene_bounds());

        let depth_data = collect_depth_segments::<D2>(prepared.depth_segments());
        assert!(depth_data.segments.is_empty());
        assert_eq!(depth_data.max_topological_depth, 0);
        assert_eq!(depth_data.bounds, D2::empty_scene_bounds());
    }

    #[allow(clippy::too_many_arguments)]
    fn check_stamped_depth_matches_interpreted<D>(
        stamped: DepthSegmentStream<'_, D>,
        interpreted: DepthSegmentStream<'_, D>,
        axis_count: usize,
        depth_of: impl Fn(&D::DepthRecord) -> u32,
        start_of: impl Fn(&D::DepthRecord) -> D::Point,
        end_of: impl Fn(&D::DepthRecord) -> D::Point,
    ) where
        D: RenderDimension + TemplateDimension,
        D::Point: Index<usize, Output = f32>,
        D::Bounds: TestBounds,
    {
        let stamped = collect_depth_segments::<D>(stamped);
        let interpreted = collect_depth_segments::<D>(interpreted);

        assert_eq!(stamped.segments.len(), interpreted.segments.len());
        assert_eq!(
            stamped.max_topological_depth,
            interpreted.max_topological_depth
        );
        for (s, i) in stamped.segments.iter().zip(&interpreted.segments) {
            assert_eq!(depth_of(s), depth_of(i));
            assert!(axis_distance(start_of(s), start_of(i), axis_count) < 1e-3);
            assert!(axis_distance(end_of(s), end_of(i), axis_count) < 1e-3);
        }
    }

    #[test]
    fn stamped_2d_depth_segments_match_interpreted() {
        let config = GenerationConfig::new(
            Dimensions::TwoD,
            "X".to_string(),
            5,
            23.4,
            1.0,
            90.0,
            BTreeMap::from([
                ('X', "F+[[X]-X]-F[-FX]+X".to_string()),
                ('F', "FF".to_string()),
            ]),
        )
        .expect("balanced config");
        let stamped = PreparedGeneration::Stamped(
            compile_2d(&config).build_templates(2).expect("set builds"),
        );
        let interpreted = PreparedGeneration::Interpreted(compile_2d(&config));

        check_stamped_depth_matches_interpreted::<D2>(
            stamped.depth_segments(),
            interpreted.depth_segments(),
            2,
            |s| s.topological_depth,
            |s| s.start,
            |s| s.end,
        );
    }

    #[test]
    fn stamped_3d_depth_segments_match_interpreted() {
        let config = GenerationConfig::new(
            Dimensions::ThreeD,
            "A".to_string(),
            5,
            40.0,
            1.0,
            90.0,
            BTreeMap::from([
                ('A', r"F[+/A]/[-/A]F[&/A]/[^/A]".to_string()),
                ('F', "FF".to_string()),
            ]),
        )
        .expect("balanced config");
        let stamped = PreparedGeneration::Stamped(
            compile_3d(&config).build_templates(2).expect("set builds"),
        );
        let interpreted = PreparedGeneration::Interpreted(compile_3d(&config));

        check_stamped_depth_matches_interpreted::<D3>(
            stamped.depth_segments(),
            interpreted.depth_segments(),
            3,
            |s| s.topological_depth,
            |s| s.start,
            |s| s.end,
        );

        let stamped = collect_depth_segments::<D3>(stamped.depth_segments());
        let interpreted = collect_depth_segments::<D3>(interpreted.depth_segments());
        assert_depth_3d_endpoints_are_contained(&stamped);
        assert_depth_3d_endpoints_are_contained(&interpreted);
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
        GenerationConfig::new(
            Dimensions::TwoD,
            axiom.to_string(),
            0,
            90.0,
            1.0,
            0.0,
            BTreeMap::new(),
        )
        .expect("balanced config")
    }

    #[test]
    fn empty_geometry_uses_fallback_bounds() {
        let config = cfg("A");
        let prepared = PreparedGeneration::Interpreted(compile_2d(&config));
        let SegmentData2D { segments, bounds } = collect_plain_segments::<D2>(prepared.segments());
        assert!(segments.is_empty());
        assert_eq!(bounds, D2::empty_scene_bounds());
    }

    fn check_empty_depth_fallback<D: RenderDimension + TemplateDimension>(
        data: CollectedDepthSegmentData<D::DepthRecord, D::Bounds>,
        expected: D::Bounds,
    ) where
        D::Bounds: PartialEq + std::fmt::Debug,
    {
        assert!(data.segments.is_empty());
        assert_eq!(data.max_topological_depth, 0);
        assert_eq!(data.bounds, expected);
    }

    #[test]
    fn empty_depth_geometry_uses_fallback_bounds_and_zero_max_depth() {
        let config = cfg("A");
        let prepared = PreparedGeneration::Interpreted(compile_2d(&config));
        let data = collect_depth_segments::<D2>(prepared.depth_segments());
        check_empty_depth_fallback::<D2>(data, D2::empty_scene_bounds());
    }

    #[test]
    fn empty_depth_geometry_3d_uses_fallback_bounds_and_zero_max_depth() {
        let config = GenerationConfig::new(
            Dimensions::ThreeD,
            "A".to_string(),
            0,
            90.0,
            1.0,
            0.0,
            BTreeMap::new(),
        )
        .expect("balanced config");
        let prepared = PreparedGeneration::Interpreted(compile_3d(&config));
        let data = collect_depth_segments::<D3>(prepared.depth_segments());
        check_empty_depth_fallback::<D3>(data, D3::empty_scene_bounds());
    }

    #[test]
    fn single_segment_produces_one_segment_record_and_tight_bounds() {
        let config = cfg("F");
        let prepared = PreparedGeneration::Interpreted(compile_2d(&config));
        let SegmentData2D { segments, bounds } = collect_plain_segments::<D2>(prepared.segments());
        assert_eq!(segments.len(), 1);
        assert!(close(segments[0].start[0], 0.0) && close(segments[0].start[1], 0.0));
        assert!(close(segments[0].end[0], 1.0) && close(segments[0].end[1], 0.0));
        assert_eq!(bounds.min, Vec2::ZERO);
        assert_eq!(bounds.max, Vec2::X);
    }

    #[test]
    fn bounds_are_tight_over_all_segments() {
        let config = cfg("F+F-F");
        let prepared = PreparedGeneration::Interpreted(compile_2d(&config));
        let SegmentData2D { segments, bounds } = collect_plain_segments::<D2>(prepared.segments());
        assert_eq!(segments.len(), 3);
        assert_eq!(bounds.min, Vec2::ZERO);
        assert_eq!(bounds.max, Vec2::new(2.0, 1.0));
    }

    fn check_topological_depth_segments<D>(
        data: CollectedDepthSegmentData<D::DepthRecord, D::Bounds>,
        depth_of: impl Fn(&D::DepthRecord) -> u32,
        expected_depths: [u32; 3],
        expected_bounds: D::Bounds,
    ) where
        D: RenderDimension + TemplateDimension,
        D::Bounds: TestBounds,
    {
        assert_eq!(data.segments.len(), 3);
        for (segment, expected) in data.segments.iter().zip(expected_depths) {
            assert_eq!(depth_of(segment), expected);
        }
        assert_eq!(
            data.max_topological_depth,
            expected_depths.into_iter().max().unwrap()
        );
        assert!(data.bounds.close_to(expected_bounds, EPS));
    }

    #[test]
    fn topological_depth_segments_preserve_depth_and_compute_max() {
        let config = cfg("F[+F]F");
        let prepared = PreparedGeneration::Interpreted(compile_2d(&config));
        let data = collect_depth_segments::<D2>(prepared.depth_segments());
        check_topological_depth_segments::<D2>(
            data,
            |s| s.topological_depth,
            [0, 1, 1],
            Bounds2D {
                min: Vec2::ZERO,
                max: Vec2::new(2.0, 1.0),
            },
        );
    }

    #[test]
    fn topological_depth_segments_3d_preserve_depth_and_compute_max() {
        let config = GenerationConfig::new(
            Dimensions::ThreeD,
            "F[+F]F".to_string(),
            0,
            90.0,
            1.0,
            0.0,
            BTreeMap::new(),
        )
        .expect("balanced config");
        let prepared = PreparedGeneration::Interpreted(compile_3d(&config));
        let data = collect_depth_segments::<D3>(prepared.depth_segments());
        assert_eq!(data.segments.len(), 3);
        assert_eq!(
            data.segments
                .iter()
                .map(|segment| segment.topological_depth)
                .collect::<Vec<_>>(),
            vec![0, 1, 1]
        );
        assert_eq!(data.max_topological_depth, 1);
        assert_depth_3d_endpoints_are_contained(&data);
    }

    #[test]
    fn viewport_transform_fits_and_centers_bounds() {
        let t = viewport_transform(
            Bounds2D {
                min: Vec2::new(1.0, 0.0),
                max: Vec2::new(5.0, 4.0),
            },
            200,
            200,
            Vec2::ZERO,
            1.0,
        );
        assert!(close(3.0 * t.scale[0] + t.offset[0], 0.0));
        assert!(close(2.0 * t.scale[1] + t.offset[1], 0.0));
        assert!(close(4.0 * t.scale[0], 1.8));
        assert!(close(4.0 * t.scale[1], 1.8));
    }

    #[test]
    fn viewport_transform_keeps_degenerate_bounds_finite() {
        let t = viewport_transform(
            Bounds2D {
                min: Vec2::new(5.0, 3.0),
                max: Vec2::new(5.0, 3.0),
            },
            100,
            100,
            Vec2::ZERO,
            1.0,
        );
        assert!(t.scale[0].is_finite() && t.scale[0] > 0.0);
        assert!(t.scale[1].is_finite() && t.scale[1] > 0.0);
        assert!(close(5.0 * t.scale[0] + t.offset[0], 0.0));
        assert!(close(3.0 * t.scale[1] + t.offset[1], 0.0));
    }
}
