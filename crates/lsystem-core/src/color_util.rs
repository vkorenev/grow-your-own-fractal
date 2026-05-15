/// Converts an RGB color with components in `0.0..=1.0` to
/// `(hue_degrees, saturation, value)`.
pub fn rgb_to_hsv([r, g, b]: [f32; 3]) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;

    let hue = if delta == 0.0 {
        0.0
    } else if max == r {
        60.0 * ((g - b) / delta).rem_euclid(6.0)
    } else if max == g {
        60.0 * (((b - r) / delta) + 2.0)
    } else {
        60.0 * (((r - g) / delta) + 4.0)
    };

    let saturation = if max == 0.0 { 0.0 } else { delta / max };
    (hue, saturation, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-5;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < EPS
    }

    #[test]
    fn converts_rgb_to_hsv() {
        let (hue, saturation, value) = rgb_to_hsv([0.25, 0.5, 0.5]);

        assert!(close(hue, 180.0));
        assert!(close(saturation, 0.5));
        assert!(close(value, 0.5));
    }

    #[test]
    fn grayscale_has_zero_saturation_and_hue() {
        let (hue, saturation, value) = rgb_to_hsv([0.4, 0.4, 0.4]);

        assert!(close(hue, 0.0));
        assert!(close(saturation, 0.0));
        assert!(close(value, 0.4));
    }

    #[test]
    fn pure_blue_maps_to_blue_hue() {
        let (hue, saturation, value) = rgb_to_hsv([0.0, 0.0, 1.0]);

        assert!(close(hue, 240.0));
        assert!(close(saturation, 1.0));
        assert!(close(value, 1.0));
    }

    #[test]
    fn black_has_zero_saturation_and_value() {
        let (hue, saturation, value) = rgb_to_hsv([0.0, 0.0, 0.0]);

        assert!(close(hue, 0.0));
        assert!(close(saturation, 0.0));
        assert!(close(value, 0.0));
    }

    #[test]
    fn red_dominant_negative_hue_wraps() {
        let (hue, saturation, value) = rgb_to_hsv([1.0, 0.0, 0.5]);

        assert!(close(hue, 330.0));
        assert!(close(saturation, 1.0));
        assert!(close(value, 1.0));
    }
}
