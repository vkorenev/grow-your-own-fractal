use glam::{Mat4, Quat, Vec3};

use crate::line_renderer::{Mvp, Transform};
use crate::lsystem_bridge::{fitted_pixels_per_unit, viewport_transform};

const ORBIT_DEG_PER_PIXEL: f32 = 0.3;
const FOV_Y_RAD: f32 = std::f32::consts::FRAC_PI_4; // 45°

#[derive(Clone, Debug)]
pub struct Camera {
    pan: [f32; 2],
    zoom: f32,
    pub azimuth: f32,
    pub elevation: f32,
    pub roll: f32,
}

impl Camera {
    pub fn new() -> Self {
        Self {
            pan: [0.0, 0.0],
            zoom: 1.0,
            azimuth: 0.0,
            elevation: 20.0,
            roll: 0.0,
        }
    }

    pub fn reset(&mut self) {
        self.pan = [0.0, 0.0];
        self.zoom = 1.0;
        self.azimuth = 0.0;
        self.elevation = 20.0;
        self.roll = 0.0;
    }

    /// Preserves azimuth/elevation/roll; only resets pan and zoom.
    pub fn reset_position(&mut self) {
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
        fitted_pixels_per_unit(bounds_min, bounds_max, width, height) * self.zoom
    }

    pub fn compute_transform(
        &self,
        bounds_min: [f32; 2],
        bounds_max: [f32; 2],
        width: u32,
        height: u32,
    ) -> Transform {
        viewport_transform(bounds_min, bounds_max, width, height, self.pan, self.zoom)
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
        // Derived from: world_under_cursor = ndc / scale_old + center - pan.
        // After zoom: ndc = (world_under_cursor - center + pan_new) * scale_new.
        let clamped = (self.zoom * factor).clamp(1e-4, 1e4);
        let actual = clamped / self.zoom;
        self.pan[0] -= ndc_x / sx * (1.0 - 1.0 / actual);
        self.pan[1] -= ndc_y / sy * (1.0 - 1.0 / actual);
        self.zoom = clamped;
    }

    pub fn orbit_by_pixels(&mut self, dx: f32, dy: f32) {
        self.orbit_by(dx * ORBIT_DEG_PER_PIXEL, -dy * ORBIT_DEG_PER_PIXEL);
    }

    pub fn orbit_by(&mut self, d_azimuth: f32, d_elevation: f32) {
        self.azimuth += d_azimuth;
        self.elevation = (self.elevation + d_elevation).clamp(-89.0, 89.0);
    }

    /// Roll the camera view by degrees (positive = counter-clockwise).
    pub fn roll_by(&mut self, degrees: f32) {
        self.roll += degrees;
    }

    pub fn auto_rotate_by(&mut self, degrees: f32) {
        self.azimuth += degrees;
    }

    /// Zoom in/out for 3D (same zoom field, affects view distance).
    pub fn zoom_3d(&mut self, factor: f32) {
        if factor > 0.0 {
            self.zoom = (self.zoom * factor).clamp(1e-4, 1e4);
        }
    }

    pub fn compute_mvp_3d(
        &self,
        bounds_min: [f32; 3],
        bounds_max: [f32; 3],
        width: u32,
        height: u32,
    ) -> Mvp {
        let min = Vec3::from(bounds_min);
        let max = Vec3::from(bounds_max);
        let center = (min + max) * 0.5;
        let extent = max - min;
        let radius = (extent.length() * 0.5).max(1.0);

        // Base distance to frame the geometry at 45° FOV, then apply zoom.
        let base_distance = radius / (FOV_Y_RAD * 0.5).tan();
        let distance = (base_distance / self.zoom).max(radius * 0.01);

        let az = self.azimuth.to_radians();
        let el = self.elevation.to_radians();
        let eye_dir = Vec3::new(el.cos() * az.sin(), el.sin(), el.cos() * az.cos());
        let eye = center + eye_dir * distance;

        // Up vector: world Y unless camera is near zenith/nadir.
        let world_up = if (self.elevation.abs() - 89.0).abs() < 1.0 {
            // Near pole: use a stable fallback derived from azimuth.
            Vec3::new(-az.sin(), 0.0, -az.cos())
        } else {
            Vec3::Y
        };
        let roll_quat = Quat::from_axis_angle(eye_dir, self.roll.to_radians());
        let up = roll_quat * world_up;

        let view = Mat4::look_at_rh(eye, center, up);
        let aspect = (width as f32 / height as f32).max(0.001);
        let z_near = distance * 0.001;
        let z_far = distance * 10.0;
        let proj = Mat4::perspective_rh(FOV_Y_RAD, aspect, z_near, z_far);

        Mvp {
            matrix: proj * view,
        }
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
        // Cursor at top-right: pixel (150, 50) -> NDC (0.5, 0.5)
        let cursor = [150.0f32, 50.0];
        let ndc_x = cursor[0] / w as f32 * 2.0 - 1.0;
        let ndc_y = 1.0 - cursor[1] / h as f32 * 2.0;

        let mut cam = Camera::new();
        let t_before = cam.compute_transform(bounds_min, bounds_max, w, h);
        let wp_x = (ndc_x - t_before.offset[0]) / t_before.scale[0];
        let wp_y = (ndc_y - t_before.offset[1]) / t_before.scale[1];

        cam.zoom_toward_cursor(2.0, cursor, bounds_min, bounds_max, w, h);
        let t_after = cam.compute_transform(bounds_min, bounds_max, w, h);

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
        let ndc_center = t.offset[0];
        assert!(ndc_center > 0.0, "ndc center x = {ndc_center}");
    }

    #[test]
    fn mvp_3d_produces_finite_matrix() {
        let cam = Camera::new();
        let mvp = cam.compute_mvp_3d([-5.0, -5.0, -5.0], [5.0, 5.0, 5.0], 800, 600);
        for column in mvp.matrix.to_cols_array_2d() {
            for v in column {
                assert!(v.is_finite(), "MVP contains non-finite value: {v}");
            }
        }
    }

    #[test]
    fn orbit_clamps_elevation() {
        let mut cam = Camera::new();
        cam.orbit_by(0.0, 999.0);
        assert!(cam.elevation <= 89.0, "elevation={}", cam.elevation);
        cam.orbit_by(0.0, -999.0);
        assert!(cam.elevation >= -89.0, "elevation={}", cam.elevation);
    }
}
