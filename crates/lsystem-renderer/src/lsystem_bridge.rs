use glam::{Vec2, Vec3, Vec4};
use lsystem_core::{
    LineColorConfig, Segment2DWithTopologicalDepth, Segment3DWithTopologicalDepth, Stamp2D,
    Stamp3D, StampStats, TemplateSet2D, TemplateSet3D,
};

use crate::line_renderer::{
    ColorParams, Segment2D, Segment3D, TopologicalDepthSegment2D, TopologicalDepthSegment3D,
    Transform,
};

/// Axis-aligned bounds accumulator over 2D segment endpoints.
struct Bounds2D {
    min: Vec2,
    max: Vec2,
}

impl Bounds2D {
    fn new() -> Self {
        Self {
            min: Vec2::INFINITY,
            max: Vec2::NEG_INFINITY,
        }
    }

    fn update(&mut self, a: Vec2, b: Vec2) {
        self.min = self.min.min(a).min(b);
        self.max = self.max.max(a).max(b);
    }

    /// Falls back to the unit box when no endpoints were seen, so empty
    /// geometry still yields a usable viewport.
    fn finish(self) -> ([f32; 2], [f32; 2]) {
        if self.min.x.is_infinite() {
            ([-1.0, -1.0], [1.0, 1.0])
        } else {
            (self.min.to_array(), self.max.to_array())
        }
    }
}

/// Axis-aligned bounds accumulator over 3D segment endpoints.
struct Bounds3D {
    min: Vec3,
    max: Vec3,
}

impl Bounds3D {
    fn new() -> Self {
        Self {
            min: Vec3::INFINITY,
            max: Vec3::NEG_INFINITY,
        }
    }

    fn update(&mut self, a: Vec3, b: Vec3) {
        self.min = self.min.min(a).min(b);
        self.max = self.max.max(a).max(b);
    }

    /// Falls back to the unit box when no endpoints were seen, so empty
    /// geometry still yields a usable viewport.
    fn finish(self) -> ([f32; 3], [f32; 3]) {
        if self.min.x.is_infinite() {
            ([-1.0, -1.0, -1.0], [1.0, 1.0, 1.0])
        } else {
            (self.min.to_array(), self.max.to_array())
        }
    }
}

/// Two-phase view of a stamped scene.
///
/// Phase 1 ([`Self::collect`]) walks the boundary expansion once and keeps
/// only the stamps, so the exact segment total and maximum topological depth
/// are known before any geometry is materialized. Phase 2
/// ([`Self::segments`] / [`Self::depth_segments`]) streams transformed
/// records in traversal order — the order the gradient shaders index by —
/// yielding exactly [`Self::total_segments`] items, which lets callers size a
/// GPU buffer up front and fill it in a single pass.
pub struct StampedScene2D<'a> {
    set: &'a TemplateSet2D,
    stamps: Vec<Stamp2D>,
    stats: StampStats,
}

impl<'a> StampedScene2D<'a> {
    pub fn collect(set: &'a TemplateSet2D) -> Self {
        let mut stamps = Vec::new();
        let mut running: u64 = 0;
        let stats = set.emit_stamps(|stamp, template| {
            debug_assert_eq!(
                u64::from(stamp.order_base),
                running,
                "stamps must stream in traversal order"
            );
            running += template.segments.len() as u64;
            stamps.push(stamp);
        });
        Self { set, stamps, stats }
    }

    pub fn total_segments(&self) -> u64 {
        self.stats.total_segments
    }

    pub fn max_topological_depth(&self) -> u32 {
        self.stats.max_depth
    }

    /// World-space segments in traversal order.
    pub fn segments(&self) -> impl Iterator<Item = Segment2D> + '_ {
        let templates = self.set.templates();
        self.stamps.iter().flat_map(move |stamp| {
            templates[stamp.template as usize]
                .segments
                .iter()
                .map(move |segment| Segment2D {
                    start: stamp.pos + stamp.rot.rotate(segment.start),
                    end: stamp.pos + stamp.rot.rotate(segment.end),
                })
        })
    }

    /// World-space segments with topological depth, in traversal order.
    pub fn depth_segments(&self) -> impl Iterator<Item = TopologicalDepthSegment2D> + '_ {
        let templates = self.set.templates();
        self.stamps.iter().flat_map(move |stamp| {
            templates[stamp.template as usize]
                .segments
                .iter()
                .map(move |segment| TopologicalDepthSegment2D {
                    start: stamp.pos + stamp.rot.rotate(segment.start),
                    end: stamp.pos + stamp.rot.rotate(segment.end),
                    topological_depth: stamp.depth_base.saturating_add(segment.depth_offset),
                })
        })
    }
}

/// Two-phase view of a stamped scene; see [`StampedScene2D`].
pub struct StampedScene3D<'a> {
    set: &'a TemplateSet3D,
    stamps: Vec<Stamp3D>,
    stats: StampStats,
}

impl<'a> StampedScene3D<'a> {
    pub fn collect(set: &'a TemplateSet3D) -> Self {
        let mut stamps = Vec::new();
        let mut running: u64 = 0;
        let stats = set.emit_stamps(|stamp, template| {
            debug_assert_eq!(
                u64::from(stamp.order_base),
                running,
                "stamps must stream in traversal order"
            );
            running += template.segments.len() as u64;
            stamps.push(stamp);
        });
        Self { set, stamps, stats }
    }

    pub fn total_segments(&self) -> u64 {
        self.stats.total_segments
    }

    pub fn max_topological_depth(&self) -> u32 {
        self.stats.max_depth
    }

    /// World-space segments in traversal order.
    pub fn segments(&self) -> impl Iterator<Item = Segment3D> + '_ {
        let templates = self.set.templates();
        self.stamps.iter().flat_map(move |stamp| {
            templates[stamp.template as usize]
                .segments
                .iter()
                .map(move |segment| Segment3D {
                    start: stamp.pos + stamp.rot * segment.start,
                    end: stamp.pos + stamp.rot * segment.end,
                })
        })
    }

    /// World-space segments with topological depth, in traversal order.
    pub fn depth_segments(&self) -> impl Iterator<Item = TopologicalDepthSegment3D> + '_ {
        let templates = self.set.templates();
        self.stamps.iter().flat_map(move |stamp| {
            templates[stamp.template as usize]
                .segments
                .iter()
                .map(move |segment| TopologicalDepthSegment3D {
                    start: stamp.pos + stamp.rot * segment.start,
                    end: stamp.pos + stamp.rot * segment.end,
                    topological_depth: stamp.depth_base.saturating_add(segment.depth_offset),
                })
        })
    }
}

pub struct SegmentData {
    pub segments: Vec<Segment2D>,
    pub bounds_min: [f32; 2],
    pub bounds_max: [f32; 2],
}

pub struct SegmentDataBuilder {
    bounds: Bounds2D,
    segments: Vec<Segment2D>,
}

impl SegmentDataBuilder {
    pub fn new() -> Self {
        Self {
            bounds: Bounds2D::new(),
            segments: Vec::new(),
        }
    }

    pub fn push_segment(&mut self, [a, b]: [Vec2; 2]) {
        self.bounds.update(a, b);
        self.segments.push(Segment2D { start: a, end: b });
    }

    pub fn finish(self) -> SegmentData {
        let (bounds_min, bounds_max) = self.bounds.finish();
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
    segments.for_each(|segment| builder.push_segment(segment));
    builder.finish()
}

/// CPU-stamped alternative to [`geometry_to_segments`]: transforms each
/// stamp's template segments into world space in a tight per-segment loop,
/// skipping per-symbol interpretation of the template-depth iterations.
pub fn stamped_geometry_to_segments(set: &TemplateSet2D) -> SegmentData {
    let scene = StampedScene2D::collect(set);
    let mut bounds = Bounds2D::new();
    let segments = scene
        .segments()
        .inspect(|segment| bounds.update(segment.start, segment.end))
        .collect();
    let (bounds_min, bounds_max) = bounds.finish();
    SegmentData {
        segments,
        bounds_min,
        bounds_max,
    }
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
    bounds: Bounds2D,
    max_topological_depth: u32,
    segments: Vec<TopologicalDepthSegment2D>,
}

impl TopologicalDepthSegmentDataBuilder {
    pub fn new() -> Self {
        Self {
            bounds: Bounds2D::new(),
            max_topological_depth: 0,
            segments: Vec::new(),
        }
    }

    pub fn push_segment(&mut self, segment: Segment2DWithTopologicalDepth) {
        let [a, b] = segment.points;
        self.bounds.update(a, b);
        self.max_topological_depth = self.max_topological_depth.max(segment.topological_depth);
        self.segments.push(TopologicalDepthSegment2D {
            start: a,
            end: b,
            topological_depth: segment.topological_depth,
        });
    }

    pub fn finish(self) -> TopologicalDepthSegmentData {
        let (bounds_min, bounds_max) = self.bounds.finish();
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
    segments.for_each(|segment| builder.push_segment(segment));
    builder.finish()
}

/// CPU-stamped alternative to [`geometry_to_depth_segments`].
pub fn stamped_geometry_to_depth_segments(set: &TemplateSet2D) -> TopologicalDepthSegmentData {
    let scene = StampedScene2D::collect(set);
    let mut bounds = Bounds2D::new();
    let segments = scene
        .depth_segments()
        .inspect(|segment| bounds.update(segment.start, segment.end))
        .collect();
    let (bounds_min, bounds_max) = bounds.finish();
    TopologicalDepthSegmentData {
        segments,
        bounds_min,
        bounds_max,
        max_topological_depth: scene.max_topological_depth(),
    }
}

pub struct SegmentData3D {
    pub segments: Vec<Segment3D>,
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
}

pub struct SegmentDataBuilder3D {
    bounds: Bounds3D,
    segments: Vec<Segment3D>,
}

impl SegmentDataBuilder3D {
    pub fn new() -> Self {
        Self {
            bounds: Bounds3D::new(),
            segments: Vec::new(),
        }
    }

    pub fn push_segment(&mut self, [a, b]: [Vec3; 2]) {
        self.bounds.update(a, b);
        self.segments.push(Segment3D { start: a, end: b });
    }

    pub fn finish(self) -> SegmentData3D {
        let (bounds_min, bounds_max) = self.bounds.finish();
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
    segments.for_each(|segment| builder.push_segment(segment));
    builder.finish()
}

/// CPU-stamped alternative to [`geometry_to_segments_3d`].
pub fn stamped_geometry_to_segments_3d(set: &TemplateSet3D) -> SegmentData3D {
    let scene = StampedScene3D::collect(set);
    let mut bounds = Bounds3D::new();
    let segments = scene
        .segments()
        .inspect(|segment| bounds.update(segment.start, segment.end))
        .collect();
    let (bounds_min, bounds_max) = bounds.finish();
    SegmentData3D {
        segments,
        bounds_min,
        bounds_max,
    }
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
    bounds: Bounds3D,
    max_topological_depth: u32,
    segments: Vec<TopologicalDepthSegment3D>,
}

impl TopologicalDepthSegmentDataBuilder3D {
    pub fn new() -> Self {
        Self {
            bounds: Bounds3D::new(),
            max_topological_depth: 0,
            segments: Vec::new(),
        }
    }

    pub fn push_segment(&mut self, segment: Segment3DWithTopologicalDepth) {
        let [a, b] = segment.points;
        self.bounds.update(a, b);
        self.max_topological_depth = self.max_topological_depth.max(segment.topological_depth);
        self.segments.push(TopologicalDepthSegment3D {
            start: a,
            end: b,
            topological_depth: segment.topological_depth,
        });
    }

    pub fn finish(self) -> TopologicalDepthSegmentData3D {
        let (bounds_min, bounds_max) = self.bounds.finish();
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
    segments.for_each(|segment| builder.push_segment(segment));
    builder.finish()
}

/// CPU-stamped alternative to [`geometry_to_depth_segments_3d`].
pub fn stamped_geometry_to_depth_segments_3d(set: &TemplateSet3D) -> TopologicalDepthSegmentData3D {
    let scene = StampedScene3D::collect(set);
    let mut bounds = Bounds3D::new();
    let segments = scene
        .depth_segments()
        .inspect(|segment| bounds.update(segment.start, segment.end))
        .collect();
    let (bounds_min, bounds_max) = bounds.finish();
    TopologicalDepthSegmentData3D {
        segments,
        bounds_min,
        bounds_max,
        max_topological_depth: scene.max_topological_depth(),
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
        CompiledGeneration, CompiledGeneration2D, CompiledGeneration3D, Dimensions,
        GenerationConfig, Rgb,
    };
    use std::collections::BTreeMap;

    use super::*;

    const EPS: f32 = 1e-5;

    fn compile_2d(config: &GenerationConfig) -> CompiledGeneration2D {
        let CompiledGeneration::TwoD(generation) = config.compile() else {
            panic!("expected a 2D generation config")
        };
        generation
    }

    fn compile_3d(config: &GenerationConfig) -> CompiledGeneration3D {
        let CompiledGeneration::ThreeD(generation) = config.compile() else {
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
        let set = TemplateSet2D::build(compile_2d(&config), 2).expect("set builds");

        let stamped = stamped_geometry_to_segments(&set);
        let interpreted = geometry_to_segments(compile_2d(&config).segments());

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
        let set = TemplateSet3D::build(compile_3d(&config), 2).expect("set builds");

        let stamped = stamped_geometry_to_segments_3d(&set);
        let interpreted = geometry_to_segments_3d(compile_3d(&config).segments());

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
    fn stamped_scene_counts_match_stats() {
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
        let set = TemplateSet2D::build(compile_2d(&config), 2).expect("set builds");
        let scene = StampedScene2D::collect(&set);

        assert!(scene.total_segments() > 0);
        assert_eq!(scene.segments().count() as u64, scene.total_segments());
        assert_eq!(
            scene.depth_segments().count() as u64,
            scene.total_segments()
        );
        let per_segment_max = scene
            .depth_segments()
            .map(|segment| segment.topological_depth)
            .max()
            .unwrap_or(0);
        assert_eq!(scene.max_topological_depth(), per_segment_max);
    }

    #[test]
    fn stamped_scene_3d_counts_match_stats() {
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
        let set = TemplateSet3D::build(compile_3d(&config), 2).expect("set builds");
        let scene = StampedScene3D::collect(&set);

        assert!(scene.total_segments() > 0);
        assert_eq!(scene.segments().count() as u64, scene.total_segments());
        assert_eq!(
            scene.depth_segments().count() as u64,
            scene.total_segments()
        );
        let per_segment_max = scene
            .depth_segments()
            .map(|segment| segment.topological_depth)
            .max()
            .unwrap_or(0);
        assert_eq!(scene.max_topological_depth(), per_segment_max);
    }

    #[test]
    fn empty_stamped_scene_reports_zero_segments_and_fallback_bounds() {
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
        let set = TemplateSet2D::build(compile_2d(&config), 1).expect("set builds");

        let scene = StampedScene2D::collect(&set);
        assert_eq!(scene.total_segments(), 0);
        assert_eq!(scene.max_topological_depth(), 0);
        assert_eq!(scene.segments().count(), 0);

        let data = stamped_geometry_to_segments(&set);
        assert!(data.segments.is_empty());
        assert_eq!(data.bounds_min, [-1.0, -1.0]);
        assert_eq!(data.bounds_max, [1.0, 1.0]);

        let depth_data = stamped_geometry_to_depth_segments(&set);
        assert!(depth_data.segments.is_empty());
        assert_eq!(depth_data.max_topological_depth(), 0);
        assert_eq!(depth_data.bounds_min, [-1.0, -1.0]);
        assert_eq!(depth_data.bounds_max, [1.0, 1.0]);
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
        let set = TemplateSet2D::build(compile_2d(&config), 2).expect("set builds");

        let stamped = stamped_geometry_to_depth_segments(&set);
        let interpreted = geometry_to_depth_segments(compile_2d(&config).depth_segments());

        assert_eq!(stamped.segments.len(), interpreted.segments.len());
        assert_eq!(
            stamped.max_topological_depth(),
            interpreted.max_topological_depth()
        );
        for axis in 0..2 {
            assert!((stamped.bounds_min[axis] - interpreted.bounds_min[axis]).abs() < 1e-3);
            assert!((stamped.bounds_max[axis] - interpreted.bounds_max[axis]).abs() < 1e-3);
        }
        for (s, i) in stamped.segments.iter().zip(&interpreted.segments) {
            assert_eq!(s.topological_depth, i.topological_depth);
            assert!(s.start.distance(i.start) < 1e-3);
            assert!(s.end.distance(i.end) < 1e-3);
        }
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
        let set = TemplateSet3D::build(compile_3d(&config), 2).expect("set builds");

        let stamped = stamped_geometry_to_depth_segments_3d(&set);
        let interpreted = geometry_to_depth_segments_3d(compile_3d(&config).depth_segments());

        assert_eq!(stamped.segments.len(), interpreted.segments.len());
        assert_eq!(
            stamped.max_topological_depth(),
            interpreted.max_topological_depth()
        );
        for (s, i) in stamped.segments.iter().zip(&interpreted.segments) {
            assert_eq!(s.topological_depth, i.topological_depth);
            assert!(s.start.distance(i.start) < 1e-3);
            assert!(s.end.distance(i.end) < 1e-3);
        }
    }

    #[test]
    fn geometry_drains_use_iterator_fold() {
        let segment_2d = [Vec2::ZERO, Vec2::X];
        let segment_3d = [Vec3::ZERO, Vec3::X];

        let plain_2d = geometry_to_segments(FoldOnly::new(vec![segment_2d]));
        let depth_2d =
            geometry_to_depth_segments(FoldOnly::new(vec![Segment2DWithTopologicalDepth {
                points: segment_2d,
                topological_depth: 0,
            }]));
        let plain_3d = geometry_to_segments_3d(FoldOnly::new(vec![segment_3d]));
        let depth_3d =
            geometry_to_depth_segments_3d(FoldOnly::new(vec![Segment3DWithTopologicalDepth {
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
        let SegmentData {
            segments,
            bounds_min,
            bounds_max,
        } = geometry_to_segments(compile_2d(&config).segments());
        assert!(segments.is_empty());
        assert!(close(bounds_min[0], -1.0) && close(bounds_min[1], -1.0));
        assert!(close(bounds_max[0], 1.0) && close(bounds_max[1], 1.0));
    }

    #[test]
    fn empty_depth_geometry_uses_fallback_bounds_and_zero_max_depth() {
        let config = cfg("A");
        let data = geometry_to_depth_segments(compile_2d(&config).depth_segments());

        assert!(data.segments.is_empty());
        assert_eq!(data.max_topological_depth(), 0);
        assert!(close(data.bounds_min[0], -1.0) && close(data.bounds_min[1], -1.0));
        assert!(close(data.bounds_max[0], 1.0) && close(data.bounds_max[1], 1.0));
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
        let data = geometry_to_depth_segments_3d(compile_3d(&config).depth_segments());

        assert!(data.segments.is_empty());
        assert_eq!(data.max_topological_depth(), 0);
        assert_eq!(data.bounds_min, [-1.0, -1.0, -1.0]);
        assert_eq!(data.bounds_max, [1.0, 1.0, 1.0]);
    }

    #[test]
    fn single_segment_produces_one_segment_record_and_tight_bounds() {
        let config = cfg("F");
        let SegmentData {
            segments,
            bounds_min,
            bounds_max,
        } = geometry_to_segments(compile_2d(&config).segments());
        assert_eq!(segments.len(), 1);
        assert!(close(segments[0].start[0], 0.0) && close(segments[0].start[1], 0.0));
        assert!(close(segments[0].end[0], 1.0) && close(segments[0].end[1], 0.0));
        assert!(close(bounds_min[0], 0.0) && close(bounds_min[1], 0.0));
        assert!(close(bounds_max[0], 1.0) && close(bounds_max[1], 0.0));
    }

    #[test]
    fn bounds_are_tight_over_all_segments() {
        let config = cfg("F+F-F");
        let SegmentData {
            segments,
            bounds_min,
            bounds_max,
        } = geometry_to_segments(compile_2d(&config).segments());
        assert_eq!(segments.len(), 3);
        assert!(close(bounds_min[0], 0.0) && close(bounds_min[1], 0.0));
        assert!(close(bounds_max[0], 2.0) && close(bounds_max[1], 1.0));
    }

    #[test]
    fn topological_depth_segments_preserve_depth_and_compute_max() {
        let config = cfg("F[+F]F");
        let data = geometry_to_depth_segments(compile_2d(&config).depth_segments());

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
        let data = geometry_to_depth_segments_3d(compile_3d(&config).depth_segments());

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
