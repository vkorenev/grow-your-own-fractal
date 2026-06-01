use lsystem_core::LineColorConfig;

pub(crate) const HSV_MOVEMENT_DEFAULT_SPEED_DEGREES_PER_SECOND: f32 = 15.0;
pub const HSV_MOVEMENT_MIN_SPEED_DEGREES_PER_SECOND: f32 = 1.0;
pub const HSV_MOVEMENT_MAX_SPEED_DEGREES_PER_SECOND: f32 = 60.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HsvMovementDirection {
    Forward,
    Reverse,
}

impl HsvMovementDirection {
    pub const ALL: &'static [Self] = &[Self::Forward, Self::Reverse];

    pub(crate) fn sign(self) -> f32 {
        match self {
            Self::Forward => 1.0,
            Self::Reverse => -1.0,
        }
    }
}

impl std::fmt::Display for HsvMovementDirection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Forward => "Forward",
            Self::Reverse => "Reverse",
        })
    }
}

/// User-driven configuration for HSV hue-cycle animation.
/// The phase accumulator is stored separately by each GUI:
/// Leptos uses `StoredValue<f32>`, Iced uses a plain `f32` field on `FractalApp`.
#[derive(Clone, Copy, Debug)]
pub struct HsvMovement {
    enabled: bool,
    speed_degrees_per_second: f32,
    direction: HsvMovementDirection,
}

impl HsvMovement {
    pub fn is_enabled(self) -> bool {
        self.enabled
    }

    pub fn speed_degrees_per_second(self) -> f32 {
        self.speed_degrees_per_second
    }

    pub fn direction(self) -> HsvMovementDirection {
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
            HSV_MOVEMENT_MIN_SPEED_DEGREES_PER_SECOND,
            HSV_MOVEMENT_MAX_SPEED_DEGREES_PER_SECOND,
        );
    }

    pub fn set_direction(&mut self, direction: HsvMovementDirection) {
        self.direction = direction;
    }

    pub fn is_active(self, line_color: &LineColorConfig) -> bool {
        self.enabled && matches!(line_color, LineColorConfig::HueCycle { .. })
    }
}

impl Default for HsvMovement {
    fn default() -> Self {
        Self {
            enabled: false,
            speed_degrees_per_second: HSV_MOVEMENT_DEFAULT_SPEED_DEGREES_PER_SECOND,
            direction: HsvMovementDirection::Forward,
        }
    }
}

/// Pure phase-advance computation. Kept as a free function so it remains testable
/// without constructing signal infrastructure.
pub fn advance_hsv_phase_degrees(
    phase_degrees: f32,
    speed_degrees_per_second: f32,
    dt_seconds: f32,
    direction: HsvMovementDirection,
) -> f32 {
    (phase_degrees + direction.sign() * speed_degrees_per_second * dt_seconds).rem_euclid(360.0)
}

#[cfg(test)]
mod tests {
    use super::{
        HSV_MOVEMENT_MAX_SPEED_DEGREES_PER_SECOND, HSV_MOVEMENT_MIN_SPEED_DEGREES_PER_SECOND,
        HsvMovement, HsvMovementDirection, advance_hsv_phase_degrees,
    };
    use lsystem_core::LineColorConfig;

    #[test]
    fn set_speed_clamps_to_valid_range() {
        let mut m = HsvMovement::default();
        m.set_speed(0.0);
        assert_eq!(
            m.speed_degrees_per_second(),
            HSV_MOVEMENT_MIN_SPEED_DEGREES_PER_SECOND
        );
        m.set_speed(1000.0);
        assert_eq!(
            m.speed_degrees_per_second(),
            HSV_MOVEMENT_MAX_SPEED_DEGREES_PER_SECOND
        );
        m.set_speed(30.0);
        assert_eq!(m.speed_degrees_per_second(), 30.0);
    }

    #[test]
    fn is_active_requires_enabled_and_hue_cycle_mode() {
        let mut m = HsvMovement::default();
        assert!(!m.is_active(&LineColorConfig::DEFAULT_HUE_CYCLE));
        m.start();
        assert!(m.is_active(&LineColorConfig::DEFAULT_HUE_CYCLE));
        assert!(!m.is_active(&LineColorConfig::DEFAULT_SOLID));
    }

    #[test]
    fn advance_wraps_forward() {
        assert_eq!(
            advance_hsv_phase_degrees(350.0, 20.0, 1.0, HsvMovementDirection::Forward),
            10.0
        );
    }

    #[test]
    fn advance_wraps_reverse() {
        assert_eq!(
            advance_hsv_phase_degrees(10.0, 20.0, 1.0, HsvMovementDirection::Reverse),
            350.0
        );
    }
}
