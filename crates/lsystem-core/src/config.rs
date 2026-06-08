use std::collections::BTreeMap;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("rule key {key:?} must be a single ASCII letter")]
    InvalidRuleKey { key: String },

    #[error("invalid symbol {ch:?} at position {position} in `{field}`")]
    InvalidSymbol {
        ch: char,
        field: String,
        position: usize,
    },

    #[error("unmatched `]` at position {position} in `{field}`")]
    UnmatchedClose { field: String, position: usize },

    #[error("`[` at position {position} in `{field}` has no matching `]`")]
    UnmatchedOpen { field: String, position: usize },

    #[error("step must be finite and positive, got {0}")]
    InvalidStep(f32),

    #[error("angle must be finite, got {0}")]
    InvalidAngle(f32),

    #[error("initial_heading must be finite, got {0}")]
    InvalidInitialHeading(f32),

    #[error("invalid RGB color in `{field}`: expected `#rrggbb`, got {value:?}")]
    InvalidRgb { field: String, value: String },
}

/// Spatial dimensions of an L-system: 2D or 3D.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dimensions {
    TwoD,
    ThreeD,
}

/// A 24-bit RGB color, stored as `[R, G, B]` bytes.
///
/// This type is `Copy` and `const`-constructible. The `FromStr`/`Display` impls
/// use `#rrggbb` hex format. Parsing requires a leading `#` followed by exactly
/// six ASCII hex digits. Shorthand (`#rgb`) and alpha (`#rrggbbaa`) forms are not accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgb {
    bytes: [u8; 3],
}

/// Error returned when a string cannot be parsed as an `Rgb` value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("expected `#rrggbb` hex color")]
pub struct RgbError;

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { bytes: [r, g, b] }
    }

    pub const fn r(self) -> u8 {
        self.bytes[0]
    }

    pub const fn g(self) -> u8 {
        self.bytes[1]
    }

    pub const fn b(self) -> u8 {
        self.bytes[2]
    }

    /// Returns normalized `[r, g, b]` components in `0.0..=1.0` for rendering.
    pub fn to_array(self) -> [f32; 3] {
        self.bytes.map(|b| b as f32 / 255.0)
    }

    /// Converts to `(hue_degrees, saturation, value)` in HSV color space.
    pub fn to_hsv(self) -> (f32, f32, f32) {
        let [r, g, b] = self.to_array();
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
}

impl TryFrom<&str> for Rgb {
    type Error = RgbError;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        let hex = s.strip_prefix('#').ok_or(RgbError)?;
        // len() is byte count; multi-byte UTF-8 chars can pass len == 6 but would panic on byte-range slicing below
        if hex.len() != 6 || !hex.is_ascii() {
            return Err(RgbError);
        }
        let r = u8::from_str_radix(&hex[0..2], 16).map_err(|_| RgbError)?;
        let g = u8::from_str_radix(&hex[2..4], 16).map_err(|_| RgbError)?;
        let b = u8::from_str_radix(&hex[4..6], 16).map_err(|_| RgbError)?;
        Ok(Self::new(r, g, b))
    }
}

impl TryFrom<String> for Rgb {
    type Error = RgbError;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

impl std::fmt::Display for Rgb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "#{:02x}{:02x}{:02x}",
            self.bytes[0], self.bytes[1], self.bytes[2]
        )
    }
}

impl std::str::FromStr for Rgb {
    type Err = RgbError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::try_from(s)
    }
}

/// Color mode for the fractal lines.
///
/// `Solid` renders every segment with one RGB color. `Gradient` interpolates
/// from `start` to `end` either by traversal order or, when `topological_depth`
/// is enabled, by turtle stack depth. `HueCycle` converts `initial` to HSV when
/// building renderer color parameters and cycles through hue values from there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineColorConfig {
    Solid(Rgb),
    Gradient {
        start: Rgb,
        end: Rgb,
        topological_depth: bool,
    },
    HueCycle {
        initial: Rgb,
    },
}

impl LineColorConfig {
    pub fn needs_topological_depth(&self) -> bool {
        matches!(
            self,
            Self::Gradient {
                topological_depth: true,
                ..
            }
        )
    }
}

/// Visual color settings for background and fractal lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorConfig {
    pub background: Rgb,
    pub line: LineColorConfig,
}

/// Parsed and validated L-System configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub name: String,
    pub generation: GenerationConfig,
    pub colors: ColorConfig,
}

/// Validated inputs needed to expand an L-system and run the turtle.
#[derive(Debug, Clone, PartialEq)]
pub struct GenerationConfig {
    pub dimensions: Dimensions,
    pub axiom: String,
    pub iterations: u32,
    /// Turn angle in degrees.
    pub angle: f32,
    /// Length of each forward step.
    pub step: f32,
    /// Turtle heading at the start, in degrees (0 = +X, counter-clockwise positive).
    /// In 3D this is the initial yaw in the XY plane.
    pub initial_heading: f32,
    /// Production rules: single ASCII letter → replacement string.
    pub rules: BTreeMap<char, String>,
}

impl GenerationConfig {
    /// Returns `true` if the axiom or any rule RHS contains a `[` push directive.
    ///
    /// Only `[` needs checking; bracket balance is validated, so `]` cannot appear alone.
    pub fn has_stack_directives(&self) -> bool {
        has_stack_directives(&self.axiom, &self.rules)
    }
}

pub(crate) fn has_stack_directives(axiom: &str, rules: &BTreeMap<char, String>) -> bool {
    axiom.contains('[') || rules.values().any(|rhs| rhs.contains('['))
}

#[cfg(test)]
mod hex_color {
    use super::*;

    // --- Parsing ---

    #[test]
    fn try_from_accepts_lowercase_hex() {
        let hex = Rgb::try_from("#00e680").unwrap();
        assert_eq!(hex.r(), 0x00);
        assert_eq!(hex.g(), 0xe6);
        assert_eq!(hex.b(), 0x80);
    }

    #[test]
    fn try_from_accepts_uppercase_hex() {
        let hex = Rgb::try_from("#00E680").unwrap();
        assert_eq!(hex.r(), 0x00);
        assert_eq!(hex.g(), 0xe6);
        assert_eq!(hex.b(), 0x80);
    }

    #[test]
    fn try_from_string_accepts_lowercase_hex() {
        let hex = Rgb::try_from(String::from("#00e680")).unwrap();
        assert_eq!(hex, Rgb::new(0x00, 0xe6, 0x80));
    }

    // --- Display ---

    #[test]
    fn display_emits_canonical_lowercase() {
        assert_eq!(Rgb::new(0x00, 0xe6, 0x80).to_string(), "#00e680");
        assert_eq!(Rgb::new(0xAB, 0xCD, 0xEF).to_string(), "#abcdef");
        assert_eq!(Rgb::new(0x00, 0x00, 0x00).to_string(), "#000000");
    }

    // --- Rejection cases ---

    #[test]
    fn try_from_rejects_missing_leading_hash() {
        assert_eq!(Rgb::try_from("00e680"), Err(RgbError));
    }

    #[test]
    fn try_from_rejects_too_short() {
        // 5 hex chars after #
        assert_eq!(Rgb::try_from("#0e680"), Err(RgbError));
    }

    #[test]
    fn try_from_rejects_too_long() {
        // 7 hex chars after #
        assert_eq!(Rgb::try_from("#00e6800"), Err(RgbError));
    }

    #[test]
    fn try_from_rejects_non_hex_character() {
        assert_eq!(Rgb::try_from("#00g680"), Err(RgbError));
        assert_eq!(Rgb::try_from("#zzzzzz"), Err(RgbError));
    }

    #[test]
    fn try_from_rejects_shorthand() {
        // #rgb — 3 chars
        assert_eq!(Rgb::try_from("#0e6"), Err(RgbError));
    }

    #[test]
    fn try_from_rejects_alpha_form() {
        // #rrggbbaa — 8 chars
        assert_eq!(Rgb::try_from("#00e680ff"), Err(RgbError));
    }

    #[test]
    fn try_from_rejects_full_width_lookalikes() {
        // Full-width '#' (U+FF03) followed by 6 ASCII hex chars
        assert_eq!(Rgb::try_from("\u{FF03}00e680"), Err(RgbError));
        // Full-width digits
        assert_eq!(Rgb::try_from("#\u{FF10}\u{FF10}e680"), Err(RgbError));
    }

    #[test]
    fn try_from_rejects_non_ascii_same_byte_length() {
        // Two 3-byte UTF-8 chars → 6 bytes total, would panic without is_ascii() guard
        assert_eq!(Rgb::try_from("#\u{0800}\u{0800}"), Err(RgbError));
    }

    #[test]
    fn try_from_rejects_empty_string() {
        assert_eq!(Rgb::try_from(""), Err(RgbError));
    }

    // --- to_array ---

    #[test]
    fn to_array_maps_bytes_correctly() {
        let hex = Rgb::new(0x00, 0xff, 0x80);
        let [r, g, b] = hex.to_array();
        assert_eq!(r, 0.0);
        assert_eq!(g, 1.0);
        assert!((b - 128.0 / 255.0).abs() < f32::EPSILON);
    }

    #[test]
    fn to_array_black() {
        assert_eq!(Rgb::new(0x00, 0x00, 0x00).to_array(), [0.0, 0.0, 0.0]);
    }

    #[test]
    fn to_array_white() {
        assert_eq!(Rgb::new(0xff, 0xff, 0xff).to_array(), [1.0, 1.0, 1.0]);
    }

    // --- to_hsv ---

    const EPS: f32 = 1e-5;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < EPS
    }

    #[test]
    fn to_hsv_converts_correctly() {
        // 0x40=64, 0x80=128 → r=64/255≈0.251, g=b=128/255≈0.502 → hue=180°, sat≈0.5, val≈0.502
        let (hue, saturation, value) = Rgb::new(0x40, 0x80, 0x80).to_hsv();
        assert!(close(hue, 180.0));
        assert!(close(saturation, 0.5));
        assert!(close(value, 128.0 / 255.0));
    }

    #[test]
    fn to_hsv_grayscale_has_zero_saturation_and_hue() {
        let (hue, saturation, value) = Rgb::new(0x66, 0x66, 0x66).to_hsv();
        assert!(close(hue, 0.0));
        assert!(close(saturation, 0.0));
        assert!(close(value, 0x66 as f32 / 255.0));
    }

    #[test]
    fn to_hsv_pure_blue_maps_to_blue_hue() {
        let (hue, saturation, value) = Rgb::new(0x00, 0x00, 0xff).to_hsv();
        assert!(close(hue, 240.0));
        assert!(close(saturation, 1.0));
        assert!(close(value, 1.0));
    }

    #[test]
    fn to_hsv_black_has_zero_saturation_and_value() {
        let (hue, saturation, value) = Rgb::new(0x00, 0x00, 0x00).to_hsv();
        assert!(close(hue, 0.0));
        assert!(close(saturation, 0.0));
        assert!(close(value, 0.0));
    }

    #[test]
    fn to_hsv_red_dominant_negative_hue_wraps() {
        // #ff0080: r=1.0, g=0.0, b=128/255≈0.502. Raw (g-b)/delta = -0.502, which rem_euclid(6)
        // wraps to 5.498. hue = 60*5.498 ≈ 329.88°. Verifies that rem_euclid keeps hue positive.
        let (hue, saturation, value) = Rgb::new(0xff, 0x00, 0x80).to_hsv();
        assert!(close(hue, 60.0 * (-(128.0_f32 / 255.0)).rem_euclid(6.0)));
        assert!(close(saturation, 1.0));
        assert!(close(value, 1.0));
    }
}
