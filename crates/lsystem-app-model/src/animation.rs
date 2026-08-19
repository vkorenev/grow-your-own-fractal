use lsystem_core::LineColorConfig;

pub const CAMERA_AUTO_ROTATION_DEFAULT_SPEED_DEGREES_PER_SECOND: f32 = 20.0;
pub const CAMERA_AUTO_ROTATION_MIN_SPEED_DEGREES_PER_SECOND: f32 = 5.0;
pub const CAMERA_AUTO_ROTATION_MAX_SPEED_DEGREES_PER_SECOND: f32 = 360.0;
pub const CAMERA_AUTO_ROTATION_SPEED_STEP_DEGREES_PER_SECOND: f32 = 5.0;

pub(crate) const HUE_ROTATION_DEFAULT_SPEED_DEGREES_PER_SECOND: f32 = 15.0;
pub const HUE_ROTATION_MIN_SPEED_DEGREES_PER_SECOND: f32 = 1.0;
pub const HUE_ROTATION_MAX_SPEED_DEGREES_PER_SECOND: f32 = 60.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HueRotationDirection {
    Forward,
    Reverse,
}

impl HueRotationDirection {
    pub const ALL: &'static [Self] = &[Self::Forward, Self::Reverse];

    pub(crate) fn sign(self) -> f32 {
        match self {
            Self::Forward => 1.0,
            Self::Reverse => -1.0,
        }
    }
}

impl std::fmt::Display for HueRotationDirection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Forward => "Forward",
            Self::Reverse => "Reverse",
        })
    }
}

/// User-driven configuration for hue rotation animation.
/// The phase accumulator is stored separately by each GUI:
/// Leptos uses `StoredValue<f32>`, Iced uses a plain `f32` field on `FractalApp`.
#[derive(Clone, Copy, Debug)]
pub struct HueRotation {
    enabled: bool,
    speed_degrees_per_second: f32,
    direction: HueRotationDirection,
}

impl HueRotation {
    pub fn is_enabled(self) -> bool {
        self.enabled
    }

    pub fn speed_degrees_per_second(self) -> f32 {
        self.speed_degrees_per_second
    }

    pub fn direction(self) -> HueRotationDirection {
        self.direction
    }

    pub fn stop(&mut self) {
        self.enabled = false;
    }

    pub fn start(&mut self) {
        self.enabled = true;
    }

    pub fn set_speed(&mut self, speed: f32) {
        self.speed_degrees_per_second = speed.clamp(
            HUE_ROTATION_MIN_SPEED_DEGREES_PER_SECOND,
            HUE_ROTATION_MAX_SPEED_DEGREES_PER_SECOND,
        );
    }

    pub fn set_direction(&mut self, direction: HueRotationDirection) {
        self.direction = direction;
    }

    pub fn is_active(self, line_color: &LineColorConfig) -> bool {
        self.enabled && matches!(line_color, LineColorConfig::HueCycle { .. })
    }
}

impl Default for HueRotation {
    fn default() -> Self {
        Self {
            enabled: false,
            speed_degrees_per_second: HUE_ROTATION_DEFAULT_SPEED_DEGREES_PER_SECOND,
            direction: HueRotationDirection::Forward,
        }
    }
}

/// Pure phase-advance computation. Kept as a free function so it remains testable
/// without constructing signal infrastructure.
pub fn advance_hue_rotation_phase_degrees(
    phase_degrees: f32,
    speed_degrees_per_second: f32,
    dt_seconds: f32,
    direction: HueRotationDirection,
) -> f32 {
    (phase_degrees + direction.sign() * speed_degrees_per_second * dt_seconds).rem_euclid(360.0)
}

#[cfg(test)]
mod tests {
    use super::{
        HUE_ROTATION_MAX_SPEED_DEGREES_PER_SECOND, HUE_ROTATION_MIN_SPEED_DEGREES_PER_SECOND,
        HueRotation, HueRotationDirection, advance_hue_rotation_phase_degrees,
    };
    use lsystem_core::{LineColorConfig, Rgb};

    #[test]
    fn set_speed_clamps_to_valid_range() {
        let mut m = HueRotation::default();
        m.set_speed(0.0);
        assert_eq!(
            m.speed_degrees_per_second(),
            HUE_ROTATION_MIN_SPEED_DEGREES_PER_SECOND
        );
        m.set_speed(1000.0);
        assert_eq!(
            m.speed_degrees_per_second(),
            HUE_ROTATION_MAX_SPEED_DEGREES_PER_SECOND
        );
        m.set_speed(30.0);
        assert_eq!(m.speed_degrees_per_second(), 30.0);
    }

    #[test]
    fn is_active_requires_enabled_and_hue_cycle_mode() {
        let mut m = HueRotation::default();
        let hue_cycle = LineColorConfig::HueCycle {
            initial: Rgb::new(0xe5, 0x1a, 0x33),
        };
        let solid = LineColorConfig::Solid(Rgb::new(0x1a, 0x33, 0x4d));

        assert!(!m.is_active(&hue_cycle));
        m.start();
        assert!(m.is_active(&hue_cycle));
        assert!(!m.is_active(&solid));
    }

    #[test]
    fn advance_wraps_forward() {
        assert_eq!(
            advance_hue_rotation_phase_degrees(350.0, 20.0, 1.0, HueRotationDirection::Forward),
            10.0
        );
    }

    #[test]
    fn advance_wraps_reverse() {
        assert_eq!(
            advance_hue_rotation_phase_degrees(10.0, 20.0, 1.0, HueRotationDirection::Reverse),
            350.0
        );
    }
}
