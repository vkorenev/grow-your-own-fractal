use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub struct Transform {
    pub scale: [f32; 2],
    pub offset: [f32; 2],
}

pub struct Camera {
    pan: [f32; 2],
    zoom: f32,
}

impl Camera {
    pub fn new() -> Self {
        Self {
            pan: [0.0, 0.0],
            zoom: 1.0,
        }
    }

    pub fn reset(&mut self) {
        self.pan = [0.0, 0.0];
        self.zoom = 1.0;
    }

    fn px_per_unit(
        &self,
        bounds_min: [f32; 2],
        bounds_max: [f32; 2],
        width: u32,
        height: u32,
    ) -> f32 {
        let geom_w = (bounds_max[0] - bounds_min[0]).max(1.0);
        let geom_h = (bounds_max[1] - bounds_min[1]).max(1.0);
        let base = (width as f32 / geom_w).min(height as f32 / geom_h) * 0.9;
        base * self.zoom
    }

    pub fn compute_transform(
        &self,
        bounds_min: [f32; 2],
        bounds_max: [f32; 2],
        width: u32,
        height: u32,
    ) -> Transform {
        let cx = (bounds_min[0] + bounds_max[0]) * 0.5;
        let cy = (bounds_min[1] + bounds_max[1]) * 0.5;
        let ppu = self.px_per_unit(bounds_min, bounds_max, width, height);
        let sx = ppu * 2.0 / width as f32;
        let sy = ppu * 2.0 / height as f32;
        Transform {
            scale: [sx, sy],
            offset: [(-cx + self.pan[0]) * sx, (-cy + self.pan[1]) * sy],
        }
    }

    pub fn pan_by_pixels(
        &mut self,
        screen_dx: f32,
        screen_dy: f32,
        bounds_min: [f32; 2],
        bounds_max: [f32; 2],
        width: u32,
        height: u32,
    ) {
        let ppu = self.px_per_unit(bounds_min, bounds_max, width, height);
        self.pan[0] += screen_dx / ppu;
        // screen Y increases downward, world Y increases upward
        self.pan[1] -= screen_dy / ppu;
    }

    /// Zoom by `factor` keeping the world point under `cursor_px` fixed in screen space.
    pub fn zoom_toward_cursor(
        &mut self,
        factor: f32,
        cursor_px: [f32; 2],
        bounds_min: [f32; 2],
        bounds_max: [f32; 2],
        width: u32,
        height: u32,
    ) {
        if factor <= 0.0 {
            return;
        }
        let ppu = self.px_per_unit(bounds_min, bounds_max, width, height);
        let sx = ppu * 2.0 / width as f32;
        let sy = ppu * 2.0 / height as f32;
        let ndc_x = cursor_px[0] / width as f32 * 2.0 - 1.0;
        let ndc_y = 1.0 - cursor_px[1] / height as f32 * 2.0;
        // Derived from: world_under_cursor = ndc / scale_old + center - pan
        // After zoom: ndc = (world_under_cursor - center + pan_new) * scale_new
        // → pan_new = pan_old - ndc / scale_old * (1 - 1/factor)
        let clamped = (self.zoom * factor).clamp(1e-4, 1e4);
        let actual = clamped / self.zoom;
        self.pan[0] -= ndc_x / sx * (1.0 - 1.0 / actual);
        self.pan[1] -= ndc_y / sy * (1.0 - 1.0 / actual);
        self.zoom = clamped;
    }
}

impl Default for Camera {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-5;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < EPS
    }

    #[test]
    fn transform_square_geo_square_viewport() {
        let t = Camera::new().compute_transform([-1.0, -1.0], [1.0, 1.0], 100, 100);
        assert!(close(t.scale[0], 0.9), "scale x = {}", t.scale[0]);
        assert!(close(t.scale[1], 0.9), "scale y = {}", t.scale[1]);
        assert!(close(t.offset[0], 0.0));
        assert!(close(t.offset[1], 0.0));
    }

    #[test]
    fn transform_center_maps_to_ndc_origin() {
        let t = Camera::new().compute_transform([1.0, 0.0], [5.0, 4.0], 200, 200);
        assert!(close(3.0 * t.scale[0] + t.offset[0], 0.0));
        assert!(close(2.0 * t.scale[1] + t.offset[1], 0.0));
    }

    #[test]
    fn transform_width_constrained_fills_horizontal() {
        let t = Camera::new().compute_transform([0.0, 0.0], [4.0, 1.0], 100, 100);
        assert!(close(4.0 * t.scale[0], 1.8));
        assert!(1.0 * t.scale[1] < 0.9);
    }

    #[test]
    fn transform_height_constrained_fills_vertical() {
        let t = Camera::new().compute_transform([0.0, 0.0], [1.0, 4.0], 100, 100);
        assert!(close(4.0 * t.scale[1], 1.8));
        assert!(1.0 * t.scale[0] < 0.9);
    }

    #[test]
    fn transform_preserves_aspect_ratio_in_landscape_viewport() {
        let t = Camera::new().compute_transform([-1.0, -1.0], [1.0, 1.0], 200, 100);
        let px_per_unit_x = t.scale[0] * 100.0;
        let px_per_unit_y = t.scale[1] * 50.0;
        assert!(close(px_per_unit_x, px_per_unit_y));
    }

    #[test]
    fn transform_degenerate_point_geometry_stays_finite() {
        let t = Camera::new().compute_transform([5.0, 3.0], [5.0, 3.0], 100, 100);
        assert!(t.scale[0].is_finite() && t.scale[0] > 0.0);
        assert!(t.scale[1].is_finite() && t.scale[1] > 0.0);
        assert!(close(5.0 * t.scale[0] + t.offset[0], 0.0));
        assert!(close(3.0 * t.scale[1] + t.offset[1], 0.0));
    }

    #[test]
    fn zoom_doubles_scale() {
        let bounds_min = [-1.0f32, -1.0];
        let bounds_max = [1.0f32, 1.0];
        let (w, h) = (100u32, 100u32);
        let mut cam = Camera::new();
        let t_before = cam.compute_transform(bounds_min, bounds_max, w, h);
        cam.zoom_toward_cursor(2.0, [50.0, 50.0], bounds_min, bounds_max, w, h);
        let t_after = cam.compute_transform(bounds_min, bounds_max, w, h);
        assert!(close(t_after.scale[0], t_before.scale[0] * 2.0));
        assert!(close(t_after.scale[1], t_before.scale[1] * 2.0));
    }

    #[test]
    fn zoom_preserves_world_point_under_cursor() {
        let bounds_min = [-1.0f32, -1.0];
        let bounds_max = [1.0f32, 1.0];
        let (w, h) = (200u32, 200u32);
        // Cursor at top-right: pixel (150, 50) → NDC (0.5, 0.5)
        let cursor = [150.0f32, 50.0];
        let ndc_x = cursor[0] / w as f32 * 2.0 - 1.0;
        let ndc_y = 1.0 - cursor[1] / h as f32 * 2.0;

        let mut cam = Camera::new();
        let t_before = cam.compute_transform(bounds_min, bounds_max, w, h);
        // World point currently under cursor
        let wp_x = (ndc_x - t_before.offset[0]) / t_before.scale[0];
        let wp_y = (ndc_y - t_before.offset[1]) / t_before.scale[1];

        cam.zoom_toward_cursor(2.0, cursor, bounds_min, bounds_max, w, h);
        let t_after = cam.compute_transform(bounds_min, bounds_max, w, h);

        // Same world point should map to the same cursor NDC
        let ndc_x_after = wp_x * t_after.scale[0] + t_after.offset[0];
        let ndc_y_after = wp_y * t_after.scale[1] + t_after.offset[1];
        assert!(close(ndc_x_after, ndc_x), "x: {ndc_x_after} != {ndc_x}");
        assert!(close(ndc_y_after, ndc_y), "y: {ndc_y_after} != {ndc_y}");
    }

    #[test]
    fn pan_right_moves_geometry_right() {
        let bounds_min = [-1.0f32, -1.0];
        let bounds_max = [1.0f32, 1.0];
        let (w, h) = (100u32, 100u32);
        let mut cam = Camera::new();
        cam.pan_by_pixels(10.0, 0.0, bounds_min, bounds_max, w, h);
        let t = cam.compute_transform(bounds_min, bounds_max, w, h);
        // Geometry center (world 0,0) maps to positive NDC x (shifted right)
        let ndc_center = 0.0 * t.scale[0] + t.offset[0];
        assert!(ndc_center > 0.0, "ndc center x = {ndc_center}");
    }
}
