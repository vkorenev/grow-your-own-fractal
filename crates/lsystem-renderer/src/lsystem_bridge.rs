use glam::{Vec2, Vec3, Vec4};
use lsystem_core::{LineColorConfig, SegmentWithTopologicalDepth, TemplateDimension, TemplateSet};

use crate::line_renderer::{
    ColorParams, RenderDimension, Segment2D, Segment3D, TopologicalDepthSegment2D,
    TopologicalDepthSegment3D, Transform,
};

/// Point operations used by renderer-local bounds accumulation.
///
/// Public only so `RenderDimension` can bound `Dimension::Point` with it;
/// not intended for direct use outside the renderer.
#[doc(hidden)]
pub trait BoundsPoint: Copy + std::ops::Add<Output = Self> {
    const INFINITY: Self;
    const NEG_INFINITY: Self;

    fn min(self, other: Self) -> Self;
    fn max(self, other: Self) -> Self;
    /// Checking a single component suffices: `Bounds` seeds every component
    /// with infinity and any update makes them all finite together.
    fn is_unbounded(self) -> bool;
    fn fallback() -> (Self, Self);
}

impl BoundsPoint for Vec2 {
    const INFINITY: Self = Vec2::INFINITY;
    const NEG_INFINITY: Self = Vec2::NEG_INFINITY;

    fn min(self, other: Self) -> Self {
        Vec2::min(self, other)
    }

    fn max(self, other: Self) -> Self {
        Vec2::max(self, other)
    }

    fn is_unbounded(self) -> bool {
        self.x.is_infinite()
    }

    fn fallback() -> (Self, Self) {
        (Vec2::splat(-1.0), Vec2::splat(1.0))
    }
}

impl BoundsPoint for Vec3 {
    const INFINITY: Self = Vec3::INFINITY;
    const NEG_INFINITY: Self = Vec3::NEG_INFINITY;

    fn min(self, other: Self) -> Self {
        Vec3::min(self, other)
    }

    fn max(self, other: Self) -> Self {
        Vec3::max(self, other)
    }

    fn is_unbounded(self) -> bool {
        self.x.is_infinite()
    }

    fn fallback() -> (Self, Self) {
        (Vec3::splat(-1.0), Vec3::splat(1.0))
    }
}

/// Axis-aligned bounds accumulator over segment endpoints.
pub(crate) struct Bounds<P: BoundsPoint> {
    min: P,
    max: P,
}

impl<P: BoundsPoint> Bounds<P> {
    pub(crate) fn new() -> Self {
        Self {
            min: P::INFINITY,
            max: P::NEG_INFINITY,
        }
    }

    pub(crate) fn update(&mut self, a: P, b: P) {
        self.min = self.min.min(a).min(b);
        self.max = self.max.max(a).max(b);
    }

    /// Falls back to the unit box when no endpoints were seen, so empty
    /// geometry still yields a usable viewport.
    pub(crate) fn finish(self) -> (P, P) {
        if self.min.is_unbounded() {
            P::fallback()
        } else {
            (self.min, self.max)
        }
    }
}

/// Segment records collected for a stamped scene, with their bounds.
///
/// `bounds_min`/`bounds_max` bound every point in `segments`; for empty
/// geometry they fall back to the unit box.
pub struct CollectedSegmentData<R, P> {
    pub segments: Vec<R>,
    pub bounds_min: P,
    pub bounds_max: P,
}

/// Like `CollectedSegmentData`, plus the maximum topological depth seen
/// across `segments`.
///
/// `bounds_min`/`bounds_max` bound every point in `segments`; for empty
/// geometry they fall back to the unit box.
pub struct CollectedDepthSegmentData<R, P> {
    pub segments: Vec<R>,
    pub bounds_min: P,
    pub bounds_max: P,
    max_topological_depth: u32,
}

impl<R, P> CollectedDepthSegmentData<R, P> {
    pub fn max_topological_depth(&self) -> u32 {
        self.max_topological_depth
    }
}

pub type SegmentData2D = CollectedSegmentData<Segment2D, Vec2>;
pub type SegmentData3D = CollectedSegmentData<Segment3D, Vec3>;
pub type TopologicalDepthSegmentData2D = CollectedDepthSegmentData<TopologicalDepthSegment2D, Vec2>;
pub type TopologicalDepthSegmentData3D = CollectedDepthSegmentData<TopologicalDepthSegment3D, Vec3>;

struct SegmentCollector<R, P: BoundsPoint> {
    bounds: Bounds<P>,
    max_topological_depth: u32,
    segments: Vec<R>,
}

impl<R, P: BoundsPoint> SegmentCollector<R, P> {
    fn new() -> Self {
        Self {
            bounds: Bounds::new(),
            max_topological_depth: 0,
            segments: Vec::new(),
        }
    }

    fn push(&mut self, [a, b]: [P; 2], topological_depth: u32, record: R) {
        self.bounds.update(a, b);
        self.max_topological_depth = self.max_topological_depth.max(topological_depth);
        self.segments.push(record);
    }

    fn finish(self) -> (Vec<R>, P, P, u32) {
        let (bounds_min, bounds_max) = self.bounds.finish();
        (
            self.segments,
            bounds_min,
            bounds_max,
            self.max_topological_depth,
        )
    }
}

/// Stamped counterpart of the interpreted collection paths: transforms each
/// stamp's template segments into world space in a tight per-segment loop,
/// skipping per-symbol interpretation of the template-depth iterations.
pub fn collect_stamped_segments<D>(
    set: &TemplateSet<D>,
) -> CollectedSegmentData<D::PlainRecord, D::Point>
where
    D: RenderDimension + TemplateDimension,
{
    let mut collector = SegmentCollector::new();
    set.emit_segments(|points @ [a, b]| {
        collector.push(points, 0, D::plain_record(a, b));
    });
    let (segments, bounds_min, bounds_max, _) = collector.finish();
    CollectedSegmentData {
        segments,
        bounds_min,
        bounds_max,
    }
}

/// Stamped counterpart of the interpreted depth collection path; see
/// [`collect_stamped_segments`].
pub fn collect_stamped_depth_segments<D>(
    set: &TemplateSet<D>,
) -> CollectedDepthSegmentData<D::DepthRecord, D::Point>
where
    D: RenderDimension + TemplateDimension,
{
    let mut collector = SegmentCollector::new();
    set.emit_depth_segments(|segment| {
        let [a, b] = segment.points;
        collector.push(
            segment.points,
            segment.topological_depth,
            D::depth_record(a, b, segment.topological_depth),
        );
    });
    let (segments, bounds_min, bounds_max, max_topological_depth) = collector.finish();
    CollectedDepthSegmentData {
        segments,
        bounds_min,
        bounds_max,
        max_topological_depth,
    }
}

pub(crate) fn collect_plain_segments<D: RenderDimension>(
    segments: impl Iterator<Item = [D::Point; 2]>,
) -> CollectedSegmentData<D::PlainRecord, D::Point> {
    let mut builder = PlainSegmentDataBuilder::<D>::new();
    segments.for_each(|segment| builder.push_segment(segment));
    builder.finish()
}

pub(crate) fn collect_depth_segments<D: RenderDimension>(
    segments: impl Iterator<Item = SegmentWithTopologicalDepth<D>>,
) -> CollectedDepthSegmentData<D::DepthRecord, D::Point> {
    let mut builder = DepthSegmentDataBuilder::<D>::new();
    segments.for_each(|segment| builder.push_segment(segment));
    builder.finish()
}

/// Incremental generic counterpart of `collect_plain_segments` for callers
/// that need work between pushes (e.g. cancellation checks).
pub struct PlainSegmentDataBuilder<D: RenderDimension> {
    collector: SegmentCollector<D::PlainRecord, D::Point>,
}

impl<D: RenderDimension> PlainSegmentDataBuilder<D> {
    pub fn new() -> Self {
        Self {
            collector: SegmentCollector::new(),
        }
    }

    pub fn push_segment(&mut self, points @ [a, b]: [D::Point; 2]) {
        self.collector.push(points, 0, D::plain_record(a, b));
    }

    pub fn finish(self) -> CollectedSegmentData<D::PlainRecord, D::Point> {
        let (segments, bounds_min, bounds_max, _) = self.collector.finish();
        CollectedSegmentData {
            segments,
            bounds_min,
            bounds_max,
        }
    }
}

impl<D: RenderDimension> Default for PlainSegmentDataBuilder<D> {
    fn default() -> Self {
        Self::new()
    }
}

/// Incremental generic counterpart of `collect_depth_segments`.
pub struct DepthSegmentDataBuilder<D: RenderDimension> {
    collector: SegmentCollector<D::DepthRecord, D::Point>,
}

impl<D: RenderDimension> DepthSegmentDataBuilder<D> {
    pub fn new() -> Self {
        Self {
            collector: SegmentCollector::new(),
        }
    }

    pub fn push_segment(&mut self, segment: SegmentWithTopologicalDepth<D>) {
        let [a, b] = segment.points;
        self.collector.push(
            segment.points,
            segment.topological_depth,
            D::depth_record(a, b, segment.topological_depth),
        );
    }

    pub fn finish(self) -> CollectedDepthSegmentData<D::DepthRecord, D::Point> {
        let (segments, bounds_min, bounds_max, max_topological_depth) = self.collector.finish();
        CollectedDepthSegmentData {
            segments,
            bounds_min,
            bounds_max,
            max_topological_depth,
        }
    }
}

impl<D: RenderDimension> Default for DepthSegmentDataBuilder<D> {
    fn default() -> Self {
        Self::new()
    }
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
    use lsystem_core::{
        AnyCompiledGeneration, CompiledGeneration, CompiledGeneration2D, CompiledGeneration3D, D2,
        D3, Dimensions, GenerationConfig, GenerationDimension, Rgb, Segment2DWithTopologicalDepth,
        Segment3DWithTopologicalDepth,
    };
    use std::collections::BTreeMap;
    use std::ops::Index;

    use super::*;

    const EPS: f32 = 1e-5;

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

    struct FoldOnly<T> {
        items: std::vec::IntoIter<T>,
    }

    impl<T> FoldOnly<T> {
        fn new(items: Vec<T>) -> Self {
            Self {
                items: items.into_iter(),
            }
        }
    }

    impl<T> Iterator for FoldOnly<T> {
        type Item = T;

        fn next(&mut self) -> Option<Self::Item> {
            panic!("renderer bridge must drain geometry through fold")
        }

        fn fold<B, F>(self, init: B, f: F) -> B
        where
            Self: Sized,
            F: FnMut(B, Self::Item) -> B,
        {
            self.items.fold(init, f)
        }
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < EPS
    }

    /// Per-axis approximate equality over the first `axis_count` components,
    /// mirroring [`close`] for generic `D::Point` values.
    fn axis_close<P: Index<usize, Output = f32>>(a: P, b: P, axis_count: usize, eps: f32) -> bool {
        (0..axis_count).all(|axis| (a[axis] - b[axis]).abs() < eps)
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
        let set = compile_2d(&config).build_templates(2).expect("set builds");

        let stamped = collect_stamped_segments::<D2>(&set);
        let interpreted = collect_plain_segments::<D2>(compile_2d(&config).segments());

        assert_eq!(stamped.segments.len(), interpreted.segments.len());
        for axis in 0..2 {
            assert!((stamped.bounds_min[axis] - interpreted.bounds_min[axis]).abs() < 1e-3);
            assert!((stamped.bounds_max[axis] - interpreted.bounds_max[axis]).abs() < 1e-3);
        }
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
        let set = compile_3d(&config).build_templates(2).expect("set builds");

        let stamped = collect_stamped_segments::<D3>(&set);
        let interpreted = collect_plain_segments::<D3>(compile_3d(&config).segments());

        assert_eq!(stamped.segments.len(), interpreted.segments.len());
        for axis in 0..3 {
            assert!((stamped.bounds_min[axis] - interpreted.bounds_min[axis]).abs() < 1e-3);
            assert!((stamped.bounds_max[axis] - interpreted.bounds_max[axis]).abs() < 1e-3);
        }
        for (s, i) in stamped.segments.iter().zip(&interpreted.segments) {
            assert!(s.start.distance(i.start) < 1e-3);
            assert!(s.end.distance(i.end) < 1e-3);
        }
    }

    #[test]
    fn empty_stamped_segments_report_zero_and_collect_with_fallback_bounds() {
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
        let set = compile_2d(&config).build_templates(1).expect("set builds");

        let segments = set.stamped_segments();
        assert_eq!(segments.total_segments(), 0);
        assert_eq!(segments.max_topological_depth(), 0);
        assert_eq!(segments.segments().count(), 0);

        let data = collect_stamped_segments::<D2>(&set);
        assert!(data.segments.is_empty());
        assert_eq!(data.bounds_min, Vec2::splat(-1.0));
        assert_eq!(data.bounds_max, Vec2::splat(1.0));

        let depth_data = collect_stamped_depth_segments::<D2>(&set);
        assert!(depth_data.segments.is_empty());
        assert_eq!(depth_data.max_topological_depth(), 0);
        assert_eq!(depth_data.bounds_min, Vec2::splat(-1.0));
        assert_eq!(depth_data.bounds_max, Vec2::splat(1.0));
    }

    #[allow(clippy::too_many_arguments)]
    fn check_stamped_depth_matches_interpreted<D>(
        set: &TemplateSet<D>,
        generation: &CompiledGeneration<D>,
        axis_count: usize,
        depth_of: impl Fn(&D::DepthRecord) -> u32,
        start_of: impl Fn(&D::DepthRecord) -> D::Point,
        end_of: impl Fn(&D::DepthRecord) -> D::Point,
    ) where
        D: RenderDimension + TemplateDimension + GenerationDimension,
        D::Point: Index<usize, Output = f32>,
    {
        let stamped = collect_stamped_depth_segments::<D>(set);
        let interpreted = collect_depth_segments::<D>(generation.depth_segments());

        assert_eq!(stamped.segments.len(), interpreted.segments.len());
        assert_eq!(
            stamped.max_topological_depth(),
            interpreted.max_topological_depth()
        );
        assert!(axis_close(
            stamped.bounds_min,
            interpreted.bounds_min,
            axis_count,
            1e-3
        ));
        assert!(axis_close(
            stamped.bounds_max,
            interpreted.bounds_max,
            axis_count,
            1e-3
        ));
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
        let set = compile_2d(&config).build_templates(2).expect("set builds");
        let generation = compile_2d(&config);

        check_stamped_depth_matches_interpreted::<D2>(
            &set,
            &generation,
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
        let set = compile_3d(&config).build_templates(2).expect("set builds");
        let generation = compile_3d(&config);

        check_stamped_depth_matches_interpreted::<D3>(
            &set,
            &generation,
            3,
            |s| s.topological_depth,
            |s| s.start,
            |s| s.end,
        );
    }

    #[test]
    fn geometry_drains_use_iterator_fold() {
        let segment_2d = [Vec2::ZERO, Vec2::X];
        let segment_3d = [Vec3::ZERO, Vec3::X];

        let plain_2d = collect_plain_segments::<D2>(FoldOnly::new(vec![segment_2d]));
        let depth_2d =
            collect_depth_segments::<D2>(FoldOnly::new(vec![Segment2DWithTopologicalDepth {
                points: segment_2d,
                topological_depth: 0,
            }]));
        let plain_3d = collect_plain_segments::<D3>(FoldOnly::new(vec![segment_3d]));
        let depth_3d =
            collect_depth_segments::<D3>(FoldOnly::new(vec![Segment3DWithTopologicalDepth {
                points: segment_3d,
                topological_depth: 0,
            }]));

        assert_eq!(plain_2d.segments.len(), 1);
        assert_eq!(depth_2d.segments.len(), 1);
        assert_eq!(plain_3d.segments.len(), 1);
        assert_eq!(depth_3d.segments.len(), 1);
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
        let SegmentData2D {
            segments,
            bounds_min,
            bounds_max,
        } = collect_plain_segments::<D2>(compile_2d(&config).segments());
        assert!(segments.is_empty());
        assert!(close(bounds_min[0], -1.0) && close(bounds_min[1], -1.0));
        assert!(close(bounds_max[0], 1.0) && close(bounds_max[1], 1.0));
    }

    fn check_empty_depth_fallback<D: RenderDimension>(
        data: CollectedDepthSegmentData<D::DepthRecord, D::Point>,
        expected_min: D::Point,
        expected_max: D::Point,
    ) {
        assert!(data.segments.is_empty());
        assert_eq!(data.max_topological_depth(), 0);
        assert_eq!(data.bounds_min, expected_min);
        assert_eq!(data.bounds_max, expected_max);
    }

    #[test]
    fn empty_depth_geometry_uses_fallback_bounds_and_zero_max_depth() {
        let config = cfg("A");
        let data = collect_depth_segments::<D2>(compile_2d(&config).depth_segments());
        check_empty_depth_fallback::<D2>(data, Vec2::splat(-1.0), Vec2::splat(1.0));
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
        let data = collect_depth_segments::<D3>(compile_3d(&config).depth_segments());
        check_empty_depth_fallback::<D3>(data, Vec3::splat(-1.0), Vec3::splat(1.0));
    }

    #[test]
    fn single_segment_produces_one_segment_record_and_tight_bounds() {
        let config = cfg("F");
        let SegmentData2D {
            segments,
            bounds_min,
            bounds_max,
        } = collect_plain_segments::<D2>(compile_2d(&config).segments());
        assert_eq!(segments.len(), 1);
        assert!(close(segments[0].start[0], 0.0) && close(segments[0].start[1], 0.0));
        assert!(close(segments[0].end[0], 1.0) && close(segments[0].end[1], 0.0));
        assert!(close(bounds_min[0], 0.0) && close(bounds_min[1], 0.0));
        assert!(close(bounds_max[0], 1.0) && close(bounds_max[1], 0.0));
    }

    #[test]
    fn bounds_are_tight_over_all_segments() {
        let config = cfg("F+F-F");
        let SegmentData2D {
            segments,
            bounds_min,
            bounds_max,
        } = collect_plain_segments::<D2>(compile_2d(&config).segments());
        assert_eq!(segments.len(), 3);
        assert!(close(bounds_min[0], 0.0) && close(bounds_min[1], 0.0));
        assert!(close(bounds_max[0], 2.0) && close(bounds_max[1], 1.0));
    }

    fn check_topological_depth_segments<D>(
        data: CollectedDepthSegmentData<D::DepthRecord, D::Point>,
        axis_count: usize,
        depth_of: impl Fn(&D::DepthRecord) -> u32,
        expected_depths: [u32; 3],
        expected_min: D::Point,
        expected_max: D::Point,
    ) where
        D: RenderDimension,
        D::Point: Index<usize, Output = f32>,
    {
        assert_eq!(data.segments.len(), 3);
        for (segment, expected) in data.segments.iter().zip(expected_depths) {
            assert_eq!(depth_of(segment), expected);
        }
        assert_eq!(
            data.max_topological_depth(),
            expected_depths.into_iter().max().unwrap()
        );
        assert!(axis_close(data.bounds_min, expected_min, axis_count, EPS));
        assert!(axis_close(data.bounds_max, expected_max, axis_count, EPS));
    }

    #[test]
    fn topological_depth_segments_preserve_depth_and_compute_max() {
        let config = cfg("F[+F]F");
        let data = collect_depth_segments::<D2>(compile_2d(&config).depth_segments());
        check_topological_depth_segments::<D2>(
            data,
            2,
            |s| s.topological_depth,
            [0, 1, 1],
            Vec2::new(0.0, 0.0),
            Vec2::new(2.0, 1.0),
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
        let data = collect_depth_segments::<D3>(compile_3d(&config).depth_segments());
        check_topological_depth_segments::<D3>(
            data,
            3,
            |s| s.topological_depth,
            [0, 1, 1],
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(2.0, 1.0, 0.0),
        );
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
