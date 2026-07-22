use glam::{Quat, Vec2, Vec3};
use lsystem_core::{BoundingCylinder3D, Bounds2D};

use crate::line_renderer::{Mvp, Transform};
use crate::lsystem_bridge::{fitted_pixels_per_unit, viewport_transform};

const ORBIT_DEG_PER_PIXEL: f32 = 0.3;
const FOV_Y_RAD: f32 = std::f32::consts::FRAC_PI_4; // 45°
const NEAR_CLIP_RATIO: f32 = 0.001;

#[derive(Clone, Debug)]
pub struct Camera {
    pan: Vec2,
    zoom: f32,
    pub azimuth: f32,
    pub elevation: f32,
    pub roll: f32,
    /// Elevation at the time of the last `reset_position` call; determines the
    /// framing distance for the bounding cylinder. Azimuth is not needed
    /// because the cylinder is azimuth-symmetric.
    framing_elevation: f32,
}

impl Camera {
    pub fn new() -> Self {
        Self {
            pan: Vec2::ZERO,
            zoom: 1.0,
            azimuth: 0.0,
            elevation: 20.0,
            roll: 0.0,
            framing_elevation: 20.0,
        }
    }

    pub fn reset(&mut self) {
        self.pan = Vec2::ZERO;
        self.zoom = 1.0;
        self.azimuth = 0.0;
        self.elevation = 20.0;
        self.roll = 0.0;
        self.framing_elevation = 20.0;
    }

    /// Preserves azimuth/elevation/roll; only resets pan and zoom.
    /// Captures the current elevation as the new framing reference so the
    /// cylinder-fit framing stays stable for the current view.
    pub fn reset_position(&mut self) {
        self.pan = Vec2::ZERO;
        self.zoom = 1.0;
        self.framing_elevation = self.elevation;
    }

    fn px_per_unit(&self, bounds: Bounds2D, width: u32, height: u32) -> f32 {
        fitted_pixels_per_unit(bounds, width, height) * self.zoom
    }

    pub fn compute_transform(&self, bounds: Bounds2D, width: u32, height: u32) -> Transform {
        viewport_transform(bounds, width, height, self.pan, self.zoom)
    }

    pub fn pan_by_pixels(&mut self, screen_delta: Vec2, bounds: Bounds2D, width: u32, height: u32) {
        let ppu = self.px_per_unit(bounds, width, height);
        // screen Y increases downward, world Y increases upward
        self.pan += screen_delta * Vec2::new(1.0, -1.0) / ppu;
    }

    /// Zoom by `factor` keeping the world point under `cursor_px` fixed in screen space.
    pub fn zoom_toward_cursor(
        &mut self,
        factor: f32,
        cursor_px: Vec2,
        bounds: Bounds2D,
        width: u32,
        height: u32,
    ) {
        if factor <= 0.0 {
            return;
        }
        let ppu = self.px_per_unit(bounds, width, height);
        let sx = ppu * 2.0 / width as f32;
        let sy = ppu * 2.0 / height as f32;
        let ndc = Vec2::new(
            cursor_px.x / width as f32 * 2.0 - 1.0,
            1.0 - cursor_px.y / height as f32 * 2.0,
        );
        let scale = Vec2::new(sx, sy);
        // Derived from: world_under_cursor = ndc / scale_old + center - pan.
        // After zoom: ndc = (world_under_cursor - center + pan_new) * scale_new.
        let clamped = (self.zoom * factor).clamp(1e-4, 1e4);
        let actual = clamped / self.zoom;
        self.pan -= ndc / scale * (1.0 - 1.0 / actual);
        self.zoom = clamped;
    }

    /// Orbits in response to a screen-space drag, making the model follow the pointer.
    pub fn orbit_by_pixels(&mut self, screen_delta: Vec2) {
        self.orbit_by(
            -screen_delta.x * ORBIT_DEG_PER_PIXEL,
            screen_delta.y * ORBIT_DEG_PER_PIXEL,
        );
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

    pub fn compute_mvp_3d(&self, bounds: BoundingCylinder3D, width: u32, height: u32) -> Mvp {
        // `center_xz.y` is world Z, not world Y.
        let center = Vec3::new(
            bounds.center_xz.x,
            (bounds.min_y + bounds.max_y) * 0.5,
            bounds.center_xz.y,
        );
        let half_y = (bounds.max_y - bounds.min_y) * 0.5;

        // A cylinder is azimuth-symmetric, so evaluate at faz=0 (right = X,
        // eye toward +Z) using only the captured elevation.
        let fel = self.framing_elevation.to_radians();
        let (fel_sin, fel_cos) = fel.sin_cos();
        let framing_eye_dir = Vec3::new(0.0, fel_sin, fel_cos);
        let framing_roll = Quat::from_axis_angle(framing_eye_dir, self.roll.to_radians());
        let framing_right = framing_roll * Vec3::X;
        let framing_up = framing_roll * Vec3::new(0.0, fel_cos, -fel_sin);

        let aspect = (width as f32 / height as f32).max(0.001);
        let half_fov_y_tan = (FOV_Y_RAD * 0.5).tan();
        let half_fov_x_tan = half_fov_y_tan * aspect;

        let support = |v: Vec3| bounds.radius * v.x.hypot(v.z) + half_y * v.y.abs();

        // Exact support against both side planes for a centered world-Y
        // cylinder. Both signs are needed because the eye and view axes are
        // not generally symmetric, particularly for the vertical planes.
        let fit_dist = |axis: Vec3, tan_fov: f32| -> f32 {
            let vp = axis / tan_fov + framing_eye_dir;
            let vn = -axis / tan_fov + framing_eye_dir;
            support(vp).max(support(vn))
        };

        let base_distance =
            fit_dist(framing_right, half_fov_x_tan).max(fit_dist(framing_up, half_fov_y_tan));
        let distance = (base_distance / self.zoom)
            .max(bounds.radius.max(half_y) * NEAR_CLIP_RATIO)
            .max(1e-4);

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

        let view = glam::camera::rh::view::look_at_mat4(eye, center, up);
        let z_near = distance * NEAR_CLIP_RATIO;
        let z_far = distance * 10.0;
        let proj = glam::camera::rh::proj::directx::perspective(FOV_Y_RAD, aspect, z_near, z_far);

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
    use glam::Vec2;

    const EPS: f32 = 1e-5;
    // Tall plant-like bounds used across several framing tests.
    const TALL_MIN: Vec3 = Vec3::new(-10.0, 0.0, -10.0);
    const TALL_MAX: Vec3 = Vec3::new(10.0, 60.0, 10.0);

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < EPS
    }

    fn bounds_2d(min: Vec2, max: Vec2) -> Bounds2D {
        Bounds2D { min, max }
    }

    fn cylinder_from_aabb(min: Vec3, max: Vec3) -> BoundingCylinder3D {
        BoundingCylinder3D {
            center_xz: Vec2::new((min.x + max.x) * 0.5, (min.z + max.z) * 0.5),
            radius: ((max.x - min.x) * 0.5).hypot((max.z - min.z) * 0.5),
            min_y: min.y,
            max_y: max.y,
        }
    }

    #[test]
    fn transform_square_geo_square_viewport() {
        let t = Camera::new().compute_transform(bounds_2d(Vec2::splat(-1.0), Vec2::ONE), 100, 100);
        assert!(close(t.scale[0], 0.9), "scale x = {}", t.scale[0]);
        assert!(close(t.scale[1], 0.9), "scale y = {}", t.scale[1]);
        assert!(close(t.offset[0], 0.0));
        assert!(close(t.offset[1], 0.0));
    }

    #[test]
    fn transform_center_maps_to_ndc_origin() {
        let t = Camera::new().compute_transform(
            bounds_2d(Vec2::new(1.0, 0.0), Vec2::new(5.0, 4.0)),
            200,
            200,
        );
        assert!(close(3.0 * t.scale[0] + t.offset[0], 0.0));
        assert!(close(2.0 * t.scale[1] + t.offset[1], 0.0));
    }

    #[test]
    fn transform_width_constrained_fills_horizontal() {
        let t =
            Camera::new().compute_transform(bounds_2d(Vec2::ZERO, Vec2::new(4.0, 1.0)), 100, 100);
        assert!(close(4.0 * t.scale[0], 1.8));
        assert!(1.0 * t.scale[1] < 0.9);
    }

    #[test]
    fn transform_height_constrained_fills_vertical() {
        let t =
            Camera::new().compute_transform(bounds_2d(Vec2::ZERO, Vec2::new(1.0, 4.0)), 100, 100);
        assert!(close(4.0 * t.scale[1], 1.8));
        assert!(1.0 * t.scale[0] < 0.9);
    }

    #[test]
    fn transform_preserves_aspect_ratio_in_landscape_viewport() {
        let t = Camera::new().compute_transform(bounds_2d(Vec2::splat(-1.0), Vec2::ONE), 200, 100);
        let px_per_unit_x = t.scale[0] * 100.0;
        let px_per_unit_y = t.scale[1] * 50.0;
        assert!(close(px_per_unit_x, px_per_unit_y));
    }

    #[test]
    fn transform_degenerate_point_geometry_stays_finite() {
        let t = Camera::new().compute_transform(
            bounds_2d(Vec2::new(5.0, 3.0), Vec2::new(5.0, 3.0)),
            100,
            100,
        );
        assert!(t.scale[0].is_finite() && t.scale[0] > 0.0);
        assert!(t.scale[1].is_finite() && t.scale[1] > 0.0);
        assert!(close(5.0 * t.scale[0] + t.offset[0], 0.0));
        assert!(close(3.0 * t.scale[1] + t.offset[1], 0.0));
    }

    #[test]
    fn zoom_doubles_scale() {
        let bounds = bounds_2d(Vec2::splat(-1.0), Vec2::ONE);
        let (w, h) = (100u32, 100u32);
        let mut cam = Camera::new();
        let t_before = cam.compute_transform(bounds, w, h);
        cam.zoom_toward_cursor(2.0, Vec2::splat(50.0), bounds, w, h);
        let t_after = cam.compute_transform(bounds, w, h);
        assert!(close(t_after.scale[0], t_before.scale[0] * 2.0));
        assert!(close(t_after.scale[1], t_before.scale[1] * 2.0));
    }

    #[test]
    fn zoom_preserves_world_point_under_cursor() {
        let bounds = bounds_2d(Vec2::splat(-1.0), Vec2::ONE);
        let (w, h) = (200u32, 200u32);
        // Cursor at top-right: pixel (150, 50) -> NDC (0.5, 0.5)
        let cursor = Vec2::new(150.0, 50.0);
        let ndc = Vec2::new(
            cursor.x / w as f32 * 2.0 - 1.0,
            1.0 - cursor.y / h as f32 * 2.0,
        );

        let mut cam = Camera::new();
        let t_before = cam.compute_transform(bounds, w, h);
        let wp = (ndc - t_before.offset) / t_before.scale;

        cam.zoom_toward_cursor(2.0, cursor, bounds, w, h);
        let t_after = cam.compute_transform(bounds, w, h);

        let ndc_after = wp * t_after.scale + t_after.offset;
        assert!(close(ndc_after.x, ndc.x), "x: {} != {}", ndc_after.x, ndc.x);
        assert!(close(ndc_after.y, ndc.y), "y: {} != {}", ndc_after.y, ndc.y);
    }

    #[test]
    fn pan_right_moves_geometry_right() {
        let bounds = bounds_2d(Vec2::splat(-1.0), Vec2::ONE);
        let (w, h) = (100u32, 100u32);
        let mut cam = Camera::new();
        cam.pan_by_pixels(Vec2::new(10.0, 0.0), bounds, w, h);
        let t = cam.compute_transform(bounds, w, h);
        let ndc_center = t.offset[0];
        assert!(ndc_center > 0.0, "ndc center x = {ndc_center}");
    }

    #[test]
    fn mvp_3d_produces_finite_matrix() {
        let cam = Camera::new();
        let mvp = cam.compute_mvp_3d(
            cylinder_from_aabb(Vec3::splat(-5.0), Vec3::splat(5.0)),
            800,
            600,
        );
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

    #[test]
    fn pixel_orbit_makes_model_follow_drag() {
        let mut cam = Camera::new();

        cam.orbit_by_pixels(Vec2::new(10.0, 5.0));

        assert!((cam.azimuth - -3.0).abs() < 1e-6);
        assert!((cam.elevation - 21.5).abs() < 1e-6);
    }

    fn bounding_box_corners(min: Vec3, max: Vec3) -> [Vec3; 8] {
        [
            Vec3::new(min.x, min.y, min.z),
            Vec3::new(min.x, min.y, max.z),
            Vec3::new(min.x, max.y, min.z),
            Vec3::new(min.x, max.y, max.z),
            Vec3::new(max.x, min.y, min.z),
            Vec3::new(max.x, min.y, max.z),
            Vec3::new(max.x, max.y, min.z),
            Vec3::new(max.x, max.y, max.z),
        ]
    }

    fn all_corners_fit_in_frustum(mvp: &Mvp, min: Vec3, max: Vec3) -> bool {
        let tolerance = 1.001;
        for corner in bounding_box_corners(min, max) {
            let clip = mvp.matrix * corner.extend(1.0);
            let ndc_x = clip.x / clip.w;
            let ndc_y = clip.y / clip.w;
            if ndc_x.abs() > tolerance || ndc_y.abs() > tolerance {
                return false;
            }
        }
        true
    }

    fn cylinder_surface_fits(mvp: &Mvp, bounds: BoundingCylinder3D) -> bool {
        let tolerance = 1.001;
        for y in [bounds.min_y, bounds.max_y] {
            for step in 0..64 {
                let angle = step as f32 * std::f32::consts::TAU / 64.0;
                let point = glam::Vec4::new(
                    bounds.center_xz.x + bounds.radius * angle.cos(),
                    y,
                    bounds.center_xz.y + bounds.radius * angle.sin(),
                    1.0,
                );
                let clip = mvp.matrix * point;
                let ndc_x = clip.x / clip.w;
                let ndc_y = clip.y / clip.w;
                if ndc_x.abs() > tolerance || ndc_y.abs() > tolerance {
                    return false;
                }
            }
        }
        true
    }

    #[test]
    fn mvp_3d_fits_off_center_cylinder_surface_and_retargets_axis_midpoint() {
        let bounds = BoundingCylinder3D {
            center_xz: Vec2::new(17.0, -23.0),
            radius: 8.0,
            min_y: -4.0,
            max_y: 31.0,
        };
        let cam = Camera::new();
        let mvp = cam.compute_mvp_3d(bounds, 700, 900);
        assert!(cylinder_surface_fits(&mvp, bounds));

        let center = glam::Vec4::new(
            bounds.center_xz.x,
            (bounds.min_y + bounds.max_y) * 0.5,
            bounds.center_xz.y,
            1.0,
        );
        let clip = mvp.matrix * center;
        assert!(close(clip.x / clip.w, 0.0));
        assert!(close(clip.y / clip.w, 0.0));
    }

    #[test]
    fn mvp_3d_cylinder_fit_reaches_a_constraining_side_plane() {
        let bounds = BoundingCylinder3D {
            center_xz: Vec2::ZERO,
            radius: 12.0,
            min_y: -2.0,
            max_y: 2.0,
        };
        let mvp = Camera::new().compute_mvp_3d(bounds, 500, 900);
        let max_abs_ndc = [bounds.min_y, bounds.max_y]
            .into_iter()
            .flat_map(|y| {
                (0..256).map(move |step| {
                    let angle = step as f32 * std::f32::consts::TAU / 256.0;
                    let clip = mvp.matrix
                        * glam::Vec4::new(
                            bounds.radius * angle.cos(),
                            y,
                            bounds.radius * angle.sin(),
                            1.0,
                        );
                    (clip.x / clip.w).abs().max((clip.y / clip.w).abs())
                })
            })
            .fold(0.0_f32, f32::max);

        assert!(
            (0.98..=1.001).contains(&max_abs_ndc),
            "cylinder reaches only {max_abs_ndc:.3} NDC"
        );
    }

    #[test]
    fn mvp_3d_roll_uses_rolled_side_planes_for_fit() {
        let bounds = BoundingCylinder3D {
            center_xz: Vec2::ZERO,
            radius: 2.0,
            min_y: -30.0,
            max_y: 30.0,
        };
        let mut cam = Camera::new();
        cam.roll_by(73.0);
        let mvp = cam.compute_mvp_3d(bounds, 1000, 400);
        assert!(cylinder_surface_fits(&mvp, bounds));
    }

    #[test]
    fn mvp_3d_rolled_fit_is_stable_and_contains_across_azimuth() {
        let bounds = BoundingCylinder3D {
            center_xz: Vec2::new(7.0, -11.0),
            radius: 4.0,
            min_y: -25.0,
            max_y: 35.0,
        };
        let center = glam::Vec4::new(
            bounds.center_xz.x,
            (bounds.min_y + bounds.max_y) * 0.5,
            bounds.center_xz.y,
            1.0,
        );
        let mut cam = Camera::new();
        cam.roll_by(73.0);
        let mut reference_distance: Option<f32> = None;

        for az_step in 0..16 {
            cam.azimuth = az_step as f32 * 22.5;
            let mvp = cam.compute_mvp_3d(bounds, 1000, 400);
            assert!(
                cylinder_surface_fits(&mvp, bounds),
                "rolled cylinder outside frustum at azimuth={} deg",
                cam.azimuth
            );

            let distance = (mvp.matrix * center).w;
            if let Some(reference) = reference_distance {
                assert!(
                    (distance - reference).abs() < 0.01,
                    "rolled framing distance changed at azimuth={} deg: {reference:.3} vs {distance:.3}",
                    cam.azimuth
                );
            } else {
                reference_distance = Some(distance);
            }
        }
    }

    #[test]
    fn mvp_3d_degenerate_cylinders_stay_finite() {
        for bounds in [
            BoundingCylinder3D {
                center_xz: Vec2::new(2.0, -3.0),
                radius: 0.0,
                min_y: 5.0,
                max_y: 5.0,
            },
            BoundingCylinder3D {
                center_xz: Vec2::ZERO,
                radius: 7.0,
                min_y: 1.0,
                max_y: 1.0,
            },
            BoundingCylinder3D {
                center_xz: Vec2::ZERO,
                radius: 0.0,
                min_y: -9.0,
                max_y: 9.0,
            },
        ] {
            let mvp = Camera::new().compute_mvp_3d(bounds, 800, 600);
            assert!(mvp.matrix.is_finite());
        }
    }

    #[test]
    fn mvp_3d_portrait_viewport_fits_wide_geometry() {
        // Wide box on a portrait (aspect=0.5) screen: horizontal FOV is narrower,
        // so projected-fit must use the horizontal constraint.
        let cam = Camera::new();
        let min = Vec3::new(-10.0, -3.0, -3.0);
        let max = Vec3::new(10.0, 3.0, 3.0);
        let mvp = cam.compute_mvp_3d(cylinder_from_aabb(min, max), 400, 800);
        assert!(
            all_corners_fit_in_frustum(&mvp, min, max),
            "portrait viewport: corners outside frustum"
        );
    }

    #[test]
    fn mvp_3d_landscape_viewport_fits_geometry() {
        let cam = Camera::new();
        let min = Vec3::splat(-5.0);
        let max = Vec3::splat(5.0);
        let mvp = cam.compute_mvp_3d(cylinder_from_aabb(min, max), 800, 600);
        assert!(
            all_corners_fit_in_frustum(&mvp, min, max),
            "landscape viewport: corners outside frustum"
        );
    }

    #[test]
    fn mvp_3d_fills_viewport_at_reset_orientation() {
        // At the framing elevation (el=20°) the tight axis should reach ≥ 80% NDC.
        // The cylinder framing uses r_cyl ≥ half.x so the scene is slightly farther
        // than the exact AABB fit, but non-cubic geometry must not be over-scaled.
        let cam = Camera::new();
        let mvp = cam.compute_mvp_3d(cylinder_from_aabb(TALL_MIN, TALL_MAX), 800, 600);

        let max_ndc_y = bounding_box_corners(TALL_MIN, TALL_MAX)
            .iter()
            .map(|&c| {
                let clip = mvp.matrix * c.extend(1.0);
                (clip.y / clip.w).abs()
            })
            .fold(0.0f32, f32::max);

        assert!(
            max_ndc_y >= 0.80,
            "geometry fills only {max_ndc_y:.3} of viewport height, expected ≥ 0.80"
        );
    }

    #[test]
    fn mvp_3d_distance_stable_under_orbit() {
        // Orbiting must not change the framing distance; otherwise the scene
        // appears to pulse in size during auto-rotation.
        // clip_w of the bounding-box center equals the camera-to-center distance.
        let mut cam = Camera::new();
        let dist0 = center_clip_w(&cam, TALL_MIN, TALL_MAX);
        cam.orbit_by(45.0, 0.0);
        let dist45 = center_clip_w(&cam, TALL_MIN, TALL_MAX);
        assert!(
            (dist0 - dist45).abs() < 0.01,
            "distance changed after orbit: {dist0:.3} vs {dist45:.3}"
        );
    }

    fn center_clip_w(cam: &Camera, min: Vec3, max: Vec3) -> f32 {
        let center = (min + max) * 0.5;
        (cam.compute_mvp_3d(cylinder_from_aabb(min, max), 800, 600)
            .matrix
            * center.extend(1.0))
        .w
    }

    #[test]
    fn mvp_3d_zoom_halves_framing_distance() {
        let mut cam = Camera::new();
        let dist1 = center_clip_w(&cam, TALL_MIN, TALL_MAX);
        cam.zoom_3d(2.0);
        let dist2 = center_clip_w(&cam, TALL_MIN, TALL_MAX);
        assert!(
            (dist2 - dist1 * 0.5).abs() < 0.01,
            "2× zoom should halve distance: {dist1:.3} → {dist2:.3}"
        );
    }

    #[test]
    fn mvp_3d_framing_distance_stable_under_elevation_orbit() {
        // framing_elevation is fixed at reset; orbiting elevation must not
        // reprice the framing distance (which would cause size pulsing).
        let mut cam = Camera::new();
        let dist0 = center_clip_w(&cam, TALL_MIN, TALL_MAX);
        cam.orbit_by(0.0, 50.0);
        let dist50 = center_clip_w(&cam, TALL_MIN, TALL_MAX);
        assert!(
            (dist0 - dist50).abs() < 0.01,
            "distance changed after elevation orbit: {dist0:.3} vs {dist50:.3}"
        );
    }

    #[test]
    fn mvp_3d_reset_restores_framing_elevation() {
        // reset() must restore framing_elevation to 20° so the framing distance
        // is identical to a freshly constructed Camera.
        let dist_fresh = center_clip_w(&Camera::new(), TALL_MIN, TALL_MAX);
        let mut cam = Camera::new();
        cam.orbit_by(0.0, 60.0); // move to a different elevation
        cam.reset();
        let dist_after_reset = center_clip_w(&cam, TALL_MIN, TALL_MAX);
        assert!(
            (dist_fresh - dist_after_reset).abs() < 0.01,
            "reset() should restore framing: fresh={dist_fresh:.3}, after reset={dist_after_reset:.3}"
        );
    }

    #[test]
    fn mvp_3d_reset_position_captures_elevation() {
        // reset_position() must update framing_elevation to the live elevation.
        // After calling it at 60°, the framing distance must differ from the
        // default 20° framing distance (i.e. the capture actually took effect).
        let dist_default = center_clip_w(&Camera::new(), TALL_MIN, TALL_MAX);
        let mut cam = Camera::new();
        cam.orbit_by(0.0, 60.0);
        cam.reset_position();
        let dist_captured = center_clip_w(&cam, TALL_MIN, TALL_MAX);
        assert!(
            (dist_captured - dist_default).abs() > 1.0,
            "reset_position() should have updated framing_elevation: \
             default={dist_default:.3}, captured={dist_captured:.3}"
        );
    }

    #[test]
    fn mvp_3d_cylinder_fits_corners_at_all_azimuths() {
        // The bounding cylinder framing must keep all AABB corners inside the
        // frustum at every azimuth during auto-rotation.
        let min = Vec3::new(-8.0, 0.0, -6.0);
        let max = Vec3::new(8.0, 50.0, 6.0); // asymmetric XZ — worst case for AABB-only framing
        let mut cam = Camera::new();
        for az_step in 0..8 {
            cam.azimuth = az_step as f32 * 45.0;
            let mvp = cam.compute_mvp_3d(cylinder_from_aabb(min, max), 800, 600);
            assert!(
                all_corners_fit_in_frustum(&mvp, min, max),
                "corners outside frustum at azimuth={}°",
                cam.azimuth
            );
        }
    }
}
