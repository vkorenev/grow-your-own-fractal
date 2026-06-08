use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer};
use thiserror::Error;

use lsystem_core::{ConfigError, LineColorConfig, Rgb};

/// Error type for TOML parse and validation failures.
///
/// This wraps TOML-layer errors (parse, deserialization) as well as domain-validation
/// errors from `lsystem-core`. It is the error type returned by `ConfigSource::parse`
/// and `TryFrom<ConfigSource> for ConfigDocument`.
#[derive(Debug, Error)]
pub enum ParseConfigError {
    #[error("TOML parse error: {0}")]
    TomlParse(#[from] toml_edit::TomlError),

    #[error("TOML deserialization error: {0}")]
    TomlDeserialize(#[from] toml_edit::de::Error),

    #[error(transparent)]
    Validation(#[from] ConfigError),
}

const DEFAULTS_TOML: &str = include_str!("defaults.toml");

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ConfigDefaults {
    pub turtle: TurtleDefaults,
    pub colors: ColorDefaults,
}

impl ConfigDefaults {
    pub fn embedded() -> &'static Self {
        static DEFAULTS: std::sync::OnceLock<ConfigDefaults> = std::sync::OnceLock::new();
        DEFAULTS.get_or_init(|| {
            Self::parse(DEFAULTS_TOML).expect("embedded config defaults should validate")
        })
    }

    fn parse(toml_str: &str) -> Result<Self, ParseConfigError> {
        let raw = toml_edit::de::from_str::<RawDefaults>(toml_str)?;
        Ok(raw.try_into()?)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TurtleDefaults {
    step: f32,
    initial_heading: f32,
}

impl TurtleDefaults {
    pub fn try_new(step: f32, initial_heading: f32) -> Result<Self, ConfigError> {
        Ok(Self {
            step: validate_step(step)?,
            initial_heading: validate_initial_heading(initial_heading)?,
        })
    }

    pub fn step(self) -> f32 {
        self.step
    }

    pub fn initial_heading(self) -> f32 {
        self.initial_heading
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorDefaults {
    pub background: Rgb,
    pub line: LineColorDefaults,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineColorDefaults {
    pub default: DefaultLineColorMode,
    pub solid: Rgb,
    pub gradient: GradientDefaults,
    pub hue_cycle: HueCycleDefaults,
}

impl LineColorDefaults {
    pub fn default_line_color(self) -> LineColorConfig {
        match self.default {
            DefaultLineColorMode::Solid => LineColorConfig::Solid(self.solid),
            DefaultLineColorMode::Gradient => LineColorConfig::Gradient {
                start: self.gradient.start,
                end: self.gradient.end,
                topological_depth: self.gradient.topological_depth,
            },
            DefaultLineColorMode::HueCycle => LineColorConfig::HueCycle {
                initial: self.hue_cycle.initial,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefaultLineColorMode {
    Solid,
    Gradient,
    HueCycle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GradientDefaults {
    pub start: Rgb,
    pub end: Rgb,
    pub topological_depth: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HueCycleDefaults {
    pub initial: Rgb,
}

// --- Raw deserialization types ---

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDefaults {
    turtle: RawTurtleDefaults,
    colors: RawColorDefaults,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTurtleDefaults {
    #[serde(deserialize_with = "deserialize_number")]
    step: f64,
    #[serde(deserialize_with = "deserialize_number")]
    initial_heading: f64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawColorDefaults {
    background: String,
    line: RawLineColorDefaults,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLineColorDefaults {
    #[serde(rename = "default")]
    default_mode: RawDefaultLineColorMode,
    solid: String,
    gradient: RawGradientDefaults,
    hue_cycle: RawHueCycleDefaults,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RawDefaultLineColorMode {
    Solid,
    Gradient,
    HueCycle,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGradientDefaults {
    start: String,
    end: String,
    topological_depth: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawHueCycleDefaults {
    initial: String,
}

// --- TryFrom impls ---

impl TryFrom<RawDefaults> for ConfigDefaults {
    type Error = ConfigError;

    fn try_from(raw: RawDefaults) -> Result<Self, Self::Error> {
        Ok(Self {
            turtle: TurtleDefaults::try_new(
                raw.turtle.step as f32,
                raw.turtle.initial_heading as f32,
            )?,
            colors: ColorDefaults {
                background: parse_rgb(raw.colors.background, "colors.background")?,
                line: LineColorDefaults {
                    default: raw.colors.line.default_mode.into(),
                    solid: parse_rgb(raw.colors.line.solid, "colors.line.solid")?,
                    gradient: GradientDefaults {
                        start: parse_rgb(
                            raw.colors.line.gradient.start,
                            "colors.line.gradient.start",
                        )?,
                        end: parse_rgb(raw.colors.line.gradient.end, "colors.line.gradient.end")?,
                        topological_depth: raw.colors.line.gradient.topological_depth,
                    },
                    hue_cycle: HueCycleDefaults {
                        initial: parse_rgb(
                            raw.colors.line.hue_cycle.initial,
                            "colors.line.hue_cycle.initial",
                        )?,
                    },
                },
            },
        })
    }
}

impl From<RawDefaultLineColorMode> for DefaultLineColorMode {
    fn from(mode: RawDefaultLineColorMode) -> Self {
        match mode {
            RawDefaultLineColorMode::Solid => Self::Solid,
            RawDefaultLineColorMode::Gradient => Self::Gradient,
            RawDefaultLineColorMode::HueCycle => Self::HueCycle,
        }
    }
}

// --- Shared validation helpers (used by editor_config.rs as well) ---

pub(crate) fn validate_step(step: f32) -> Result<f32, ConfigError> {
    if !step.is_finite() || step <= 0.0 {
        return Err(ConfigError::InvalidStep(step));
    }
    Ok(step)
}

pub(crate) fn validate_initial_heading(initial_heading: f32) -> Result<f32, ConfigError> {
    if !initial_heading.is_finite() {
        return Err(ConfigError::InvalidInitialHeading(initial_heading));
    }
    Ok(initial_heading)
}

// --- Shared helper ---

fn parse_rgb(s: String, field: &str) -> Result<Rgb, ConfigError> {
    s.parse::<Rgb>().map_err(|_| ConfigError::InvalidRgb {
        field: field.into(),
        value: s,
    })
}

// --- Custom number deserializer (needed for f64/integer support in TOML) ---

fn deserialize_number<'de, D>(deserializer: D) -> Result<f64, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(TomlNumber::deserialize(deserializer)?.0)
}

#[derive(Debug, Clone, Copy)]
struct TomlNumber(f64);

impl<'de> Deserialize<'de> for TomlNumber {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(NumberVisitor)
    }
}

struct NumberVisitor;

impl Visitor<'_> for NumberVisitor {
    type Value = TomlNumber;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a TOML integer or float")
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(TomlNumber(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(TomlNumber(value as f64))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(TomlNumber(value as f64))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Rgb {
        Rgb::try_from(s).unwrap()
    }

    fn custom_defaults() -> ConfigDefaults {
        ConfigDefaults {
            turtle: TurtleDefaults::try_new(2.5, 15.0).unwrap(),
            colors: ColorDefaults {
                background: hex("#112233"),
                line: LineColorDefaults {
                    default: DefaultLineColorMode::Solid,
                    solid: hex("#445566"),
                    gradient: GradientDefaults {
                        start: hex("#123456"),
                        end: hex("#abcdef"),
                        topological_depth: true,
                    },
                    hue_cycle: HueCycleDefaults {
                        initial: hex("#fedcba"),
                    },
                },
            },
        }
    }

    #[test]
    fn embedded_defaults_toml_parses_and_preserves_current_values() {
        let defaults = ConfigDefaults::embedded();

        assert_eq!(defaults.turtle.step(), 1.0);
        assert_eq!(defaults.turtle.initial_heading(), 0.0);
        assert_eq!(defaults.colors.background, hex("#000000"));
        assert_eq!(defaults.colors.line.default, DefaultLineColorMode::Solid);
        assert_eq!(
            defaults.colors.line.default_line_color(),
            LineColorConfig::Solid(hex("#00e680"))
        );
        assert_eq!(defaults.colors.line.solid, hex("#00e680"));
        assert_eq!(
            defaults.colors.line.gradient,
            GradientDefaults {
                start: hex("#0d590d"),
                end: hex("#99e61a"),
                topological_depth: false,
            }
        );
        assert_eq!(
            defaults.colors.line.hue_cycle,
            HueCycleDefaults {
                initial: hex("#e60000"),
            }
        );
    }

    #[test]
    fn parses_custom_default_line_color_modes_from_toml() {
        let gradient_toml = DEFAULTS_TOML
            .replace("default = \"solid\"", "default = \"gradient\"")
            .replace("start = \"#0d590d\"", "start = \"#112233\"")
            .replace("end = \"#99e61a\"", "end = \"#445566\"")
            .replace("topological_depth = false", "topological_depth = true");
        let defaults = ConfigDefaults::parse(&gradient_toml).unwrap();
        assert_eq!(
            defaults.colors.line.default_line_color(),
            LineColorConfig::Gradient {
                start: hex("#112233"),
                end: hex("#445566"),
                topological_depth: true,
            }
        );

        let hue_cycle_toml = DEFAULTS_TOML
            .replace("default = \"solid\"", "default = \"hue_cycle\"")
            .replace("initial = \"#e60000\"", "initial = \"#abcdef\"");
        let defaults = ConfigDefaults::parse(&hue_cycle_toml).unwrap();
        assert_eq!(
            defaults.colors.line.default_line_color(),
            LineColorConfig::HueCycle {
                initial: hex("#abcdef"),
            }
        );
    }

    #[test]
    fn turtle_defaults_reject_invalid_values() {
        assert!(matches!(
            TurtleDefaults::try_new(0.0, 0.0),
            Err(ConfigError::InvalidStep(0.0))
        ));
        assert!(matches!(
            TurtleDefaults::try_new(-1.0, 0.0),
            Err(ConfigError::InvalidStep(-1.0))
        ));
        assert!(matches!(
            TurtleDefaults::try_new(f32::NAN, 0.0),
            Err(ConfigError::InvalidStep(step)) if step.is_nan()
        ));
        assert!(matches!(
            TurtleDefaults::try_new(1.0, f32::NAN),
            Err(ConfigError::InvalidInitialHeading(initial_heading)) if initial_heading.is_nan()
        ));
    }

    #[test]
    fn raw_defaults_reject_unknown_keys() {
        let cases = [
            format!("{DEFAULTS_TOML}\n[unknown]\nvalue = true\n"),
            DEFAULTS_TOML.replace("step = 1.0", "step = 1.0\nunknown = true"),
            DEFAULTS_TOML.replace(
                "background = \"#000000\"",
                "background = \"#000000\"\nunknown = true",
            ),
            DEFAULTS_TOML.replace("solid = \"#00e680\"", "solid = \"#00e680\"\nunknown = true"),
            DEFAULTS_TOML.replace(
                "topological_depth = false",
                "topological_depth = false\nunknown = true",
            ),
            DEFAULTS_TOML.replace(
                "initial = \"#e60000\"",
                "initial = \"#e60000\"\nunknown = true",
            ),
        ];

        for toml in cases {
            let err = ConfigDefaults::parse(&toml).unwrap_err();

            assert!(
                matches!(err, ParseConfigError::TomlDeserialize(_)),
                "unexpected error: {err}"
            );
        }
    }

    #[test]
    fn custom_defaults_are_used_by_gradient_resolve() {
        let defaults = custom_defaults();
        let resolved = crate::editor_config::EditorLineColorConfig::Gradient {
            start: None,
            end: None,
            topological_depth: None,
        }
        .resolve(&defaults.colors.line);

        assert_eq!(
            resolved,
            LineColorConfig::Gradient {
                start: hex("#123456"),
                end: hex("#abcdef"),
                topological_depth: true,
            }
        );
    }
}
