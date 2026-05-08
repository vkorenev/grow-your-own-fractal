use glam::Vec2;
use lsystem_core::LineColorConfig;

use crate::line_renderer::{ColorParams, Transform, Vertex};

pub struct VertexData {
    pub vertices: Vec<Vertex>,
    pub bounds_min: [f32; 2],
    pub bounds_max: [f32; 2],
}

pub struct VertexDataBuilder {
    min_x: f32,
    min_y: f32,
    max_x: f32,
    max_y: f32,
    vertices: Vec<Vertex>,
}

impl VertexDataBuilder {
    pub fn new() -> Self {
        Self {
            min_x: f32::INFINITY,
            min_y: f32::INFINITY,
            max_x: f32::NEG_INFINITY,
            max_y: f32::NEG_INFINITY,
            vertices: Vec::new(),
        }
    }

    pub fn push_segment(&mut self, [a, b]: [Vec2; 2]) {
        self.min_x = self.min_x.min(a.x).min(b.x);
        self.min_y = self.min_y.min(a.y).min(b.y);
        self.max_x = self.max_x.max(a.x).max(b.x);
        self.max_y = self.max_y.max(a.y).max(b.y);
        self.vertices.push(Vertex {
            position: [a.x, a.y],
        });
        self.vertices.push(Vertex {
            position: [b.x, b.y],
        });
    }

    pub fn finish(self) -> VertexData {
        let (bounds_min, bounds_max) = if self.min_x.is_infinite() {
            ([-1.0, -1.0], [1.0, 1.0])
        } else {
            ([self.min_x, self.min_y], [self.max_x, self.max_y])
        };

        VertexData {
            vertices: self.vertices,
            bounds_min,
            bounds_max,
        }
    }
}

impl Default for VertexDataBuilder {
    fn default() -> Self {
        Self::new()
    }
}

pub fn geometry_to_vertices(segments: impl Iterator<Item = [Vec2; 2]>) -> VertexData {
    let mut builder = VertexDataBuilder::new();
    for segment in segments {
        builder.push_segment(segment);
    }
    builder.finish()
}

pub fn color_params_from_config(line: &LineColorConfig, total_segments: u32) -> ColorParams {
    match *line {
        LineColorConfig::Solid(c) => ColorParams {
            mode: 0,
            total_segments,
            color_start: [c[0], c[1], c[2], 1.0],
            ..Default::default()
        },
        LineColorConfig::Gradient { start, end } => ColorParams {
            mode: 1,
            total_segments,
            color_start: [start[0], start[1], start[2], 1.0],
            color_end: [end[0], end[1], end[2], 1.0],
            ..Default::default()
        },
        LineColorConfig::HueCycle {
            start_hue,
            saturation,
            value,
        } => ColorParams {
            mode: 2,
            total_segments,
            hue_start: start_hue,
            saturation,
            value,
            ..Default::default()
        },
    }
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
    use lsystem_core::{Config, generate};

    use super::*;

    const EPS: f32 = 1e-5;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < EPS
    }

    fn cfg(toml: &str) -> Config {
        Config::parse(toml).unwrap()
    }

    #[test]
    fn empty_geometry_uses_fallback_bounds() {
        let VertexData {
            vertices,
            bounds_min,
            bounds_max,
        } = geometry_to_vertices(generate(&cfg(
            "name=\"t\"\naxiom=\"A\"\niterations=0\nangle=90.0\nstep=1.0",
        )));
        assert!(vertices.is_empty());
        assert!(close(bounds_min[0], -1.0) && close(bounds_min[1], -1.0));
        assert!(close(bounds_max[0], 1.0) && close(bounds_max[1], 1.0));
    }

    #[test]
    fn single_segment_produces_two_vertices_and_tight_bounds() {
        let VertexData {
            vertices,
            bounds_min,
            bounds_max,
        } = geometry_to_vertices(generate(&cfg(
            "name=\"t\"\naxiom=\"F\"\niterations=0\nangle=90.0\nstep=1.0",
        )));
        assert_eq!(vertices.len(), 2);
        assert!(close(vertices[0].position[0], 0.0) && close(vertices[0].position[1], 0.0));
        assert!(close(vertices[1].position[0], 1.0) && close(vertices[1].position[1], 0.0));
        assert!(close(bounds_min[0], 0.0) && close(bounds_min[1], 0.0));
        assert!(close(bounds_max[0], 1.0) && close(bounds_max[1], 0.0));
    }

    #[test]
    fn bounds_are_tight_over_all_segments() {
        let VertexData {
            vertices,
            bounds_min,
            bounds_max,
        } = geometry_to_vertices(generate(&cfg(
            "name=\"t\"\naxiom=\"F+F-F\"\niterations=0\nangle=90.0\nstep=1.0",
        )));
        assert_eq!(vertices.len(), 6);
        assert!(close(bounds_min[0], 0.0) && close(bounds_min[1], 0.0));
        assert!(close(bounds_max[0], 2.0) && close(bounds_max[1], 1.0));
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
