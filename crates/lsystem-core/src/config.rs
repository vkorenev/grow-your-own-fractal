use std::collections::HashMap;

use serde::Deserialize;
use thiserror::Error;

use crate::alphabet::{validate_bracket_balance, validate_symbols};

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("TOML parse error: {0}")]
    TomlParse(#[from] toml::de::Error),

    #[error("rule key {key:?} must be a single ASCII letter")]
    InvalidRuleKey { key: String },

    #[error("invalid symbol {ch:?} at position {position} in `{field}`")]
    InvalidSymbol {
        ch: char,
        field: String,
        position: usize,
    },

    #[error("unsupported dimensions value {0} (must be 2)")]
    InvalidDimensions(u8),

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
}

/// Color mode for the fractal lines.
#[derive(Debug, Clone)]
pub enum LineColorConfig {
    Solid([f32; 3]),
    Gradient {
        start: [f32; 3],
        end: [f32; 3],
    },
    HueCycle {
        start_hue: f32,
        saturation: f32,
        value: f32,
    },
}

impl Default for LineColorConfig {
    fn default() -> Self {
        Self::Solid([0.0, 0.9, 0.5])
    }
}

/// Visual color settings for background and fractal lines.
#[derive(Debug, Clone)]
pub struct ColorConfig {
    pub background: [f32; 3],
    pub line: LineColorConfig,
}

impl Default for ColorConfig {
    fn default() -> Self {
        Self {
            background: [0.0, 0.0, 0.0],
            line: LineColorConfig::default(),
        }
    }
}

/// Parsed and validated L-System configuration.
#[derive(Debug, Clone)]
pub struct Config {
    pub name: String,
    pub axiom: String,
    pub iterations: u32,
    /// Turn angle in degrees.
    pub angle: f32,
    /// Length of each forward step.
    pub step: f32,
    /// Turtle heading at the start, in degrees (0 = +X, counter-clockwise positive).
    pub initial_heading: f32,
    /// Production rules: single ASCII letter → replacement string.
    pub rules: HashMap<char, String>,
    pub colors: ColorConfig,
}

impl Config {
    pub fn parse(toml_str: &str) -> Result<Self, ConfigError> {
        let raw: RawConfig = toml::from_str(toml_str)?;

        if raw.dimensions != 2 {
            return Err(ConfigError::InvalidDimensions(raw.dimensions));
        }
        if !raw.step.is_finite() || raw.step <= 0.0 {
            return Err(ConfigError::InvalidStep(raw.step));
        }
        if !raw.angle.is_finite() {
            return Err(ConfigError::InvalidAngle(raw.angle));
        }
        if !raw.initial_heading.is_finite() {
            return Err(ConfigError::InvalidInitialHeading(raw.initial_heading));
        }

        // Strip whitespace from axiom and rule RHS, then validate symbols.
        let axiom: String = raw.axiom.chars().filter(|c| !c.is_whitespace()).collect();
        validate_symbols(&axiom, "axiom")?;
        validate_bracket_balance(&axiom, "axiom")?;

        let mut rules = HashMap::with_capacity(raw.rules.len());
        for (key_str, rhs_raw) in raw.rules {
            let mut key_chars = key_str.chars();
            let key = key_chars
                .next()
                .filter(|c| c.is_ascii_alphabetic())
                .ok_or_else(|| ConfigError::InvalidRuleKey {
                    key: key_str.clone(),
                })?;
            if key_chars.next().is_some() {
                return Err(ConfigError::InvalidRuleKey { key: key_str });
            }

            let rhs: String = rhs_raw.chars().filter(|c| !c.is_whitespace()).collect();
            validate_symbols(&rhs, &format!("rules.{key}"))?;
            validate_bracket_balance(&rhs, &format!("rules.{key}"))?;
            rules.insert(key, rhs);
        }

        let colors = ColorConfig {
            background: raw.background_color.unwrap_or_default(),
            line: match raw.line_color {
                None => LineColorConfig::default(),
                Some(RawLineColor::Solid { color }) => LineColorConfig::Solid(color),
                Some(RawLineColor::Gradient { start, end }) => {
                    LineColorConfig::Gradient { start, end }
                }
                Some(RawLineColor::HueCycle {
                    start_hue,
                    saturation,
                    value,
                }) => LineColorConfig::HueCycle {
                    start_hue,
                    saturation,
                    value,
                },
            },
        };

        Ok(Config {
            name: raw.name,
            axiom,
            iterations: raw.iterations,
            angle: raw.angle,
            step: raw.step,
            initial_heading: raw.initial_heading,
            rules,
            colors,
        })
    }
}

#[derive(Deserialize)]
struct RawConfig {
    name: String,
    #[serde(default = "default_dimensions")]
    dimensions: u8,
    axiom: String,
    iterations: u32,
    angle: f32,
    step: f32,
    #[serde(default)]
    initial_heading: f32,
    #[serde(default)]
    rules: HashMap<String, String>,
    #[serde(default)]
    background_color: Option<[f32; 3]>,
    #[serde(default)]
    line_color: Option<RawLineColor>,
}

#[derive(Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
enum RawLineColor {
    Solid {
        #[serde(default = "default_line_color")]
        color: [f32; 3],
    },
    Gradient {
        start: [f32; 3],
        end: [f32; 3],
    },
    HueCycle {
        #[serde(default)]
        start_hue: f32,
        #[serde(default = "default_saturation")]
        saturation: f32,
        #[serde(default = "default_value")]
        value: f32,
    },
}

fn default_dimensions() -> u8 {
    2
}

fn default_line_color() -> [f32; 3] {
    [0.0, 0.9, 0.5]
}

fn default_saturation() -> f32 {
    1.0
}

fn default_value() -> f32 {
    0.9
}

#[cfg(test)]
mod tests {
    use super::*;

    const KOCH_TOML: &str = r#"
name = "Koch Snowflake"
dimensions = 2
axiom = "F++F++F"
iterations = 4
angle = 60.0
step = 1.0

[rules]
F = "F-F++F-F"
"#;

    #[test]
    fn parses_valid_config() {
        let cfg = Config::parse(KOCH_TOML).unwrap();
        assert_eq!(cfg.axiom, "F++F++F");
        assert_eq!(cfg.angle, 60.0);
        assert_eq!(cfg.iterations, 4);
        assert_eq!(cfg.rules[&'F'], "F-F++F-F");
    }

    #[test]
    fn strips_whitespace_from_axiom() {
        let toml = r#"
name = "test"
axiom = "F + + F"
iterations = 1
angle = 90.0
step = 1.0

[rules]
F = "F - F"
"#;
        let cfg = Config::parse(toml).unwrap();
        assert_eq!(cfg.axiom, "F++F");
        assert_eq!(cfg.rules[&'F'], "F-F");
    }

    #[test]
    fn rejects_digit_in_axiom() {
        let toml = r#"
name = "bad"
axiom = "F+1"
iterations = 1
angle = 90.0
step = 1.0
"#;
        let err = Config::parse(toml).unwrap_err();
        assert!(
            matches!(err, ConfigError::InvalidSymbol { ch: '1', .. }),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_unsupported_symbol() {
        let toml = "name=\"bad\"\naxiom=\"F&F\"\niterations=1\nangle=90.0\nstep=1.0";
        let err = Config::parse(toml).unwrap_err();
        assert!(
            matches!(err, ConfigError::InvalidSymbol { ch: '&', .. }),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_multi_char_rule_key() {
        let toml = r#"
name = "bad"
axiom = "F"
iterations = 1
angle = 90.0
step = 1.0

[rules]
FF = "FFF"
"#;
        let err = Config::parse(toml).unwrap_err();
        assert!(
            matches!(err, ConfigError::InvalidRuleKey { .. }),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_invalid_dimensions() {
        for bad_dim in [3u8, 4] {
            let toml = format!(
                "name=\"bad\"\ndimensions={bad_dim}\naxiom=\"F\"\niterations=1\nangle=90.0\nstep=1.0"
            );
            let err = Config::parse(&toml).unwrap_err();
            assert!(
                matches!(err, ConfigError::InvalidDimensions(d) if d == bad_dim),
                "dim={bad_dim}: unexpected error: {err}"
            );
        }
    }

    #[test]
    fn rejects_unmatched_close_bracket_in_axiom() {
        let toml = "name=\"bad\"\naxiom=\"F]F\"\niterations=1\nangle=90.0\nstep=1.0";
        let err = Config::parse(toml).unwrap_err();
        assert!(
            matches!(err, ConfigError::UnmatchedClose { position: 1, .. }),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_unclosed_open_bracket_in_axiom() {
        let toml = "name=\"bad\"\naxiom=\"F[F\"\niterations=1\nangle=90.0\nstep=1.0";
        let err = Config::parse(toml).unwrap_err();
        assert!(
            matches!(err, ConfigError::UnmatchedOpen { position: 1, .. }),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn reports_first_unclosed_bracket_not_last() {
        // "F[F[F": two unclosed brackets at positions 1 and 3; error must point to 1.
        let toml = "name=\"bad\"\naxiom=\"F[F[F\"\niterations=1\nangle=90.0\nstep=1.0";
        let err = Config::parse(toml).unwrap_err();
        assert!(
            matches!(err, ConfigError::UnmatchedOpen { position: 1, .. }),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_unbalanced_brackets_in_rule() {
        let toml =
            "name=\"bad\"\naxiom=\"F\"\niterations=1\nangle=90.0\nstep=1.0\n[rules]\nF=\"F[+F\"";
        let err = Config::parse(toml).unwrap_err();
        assert!(
            matches!(err, ConfigError::UnmatchedOpen { .. }),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_non_positive_step() {
        for bad_step in ["0.0", "-1.0"] {
            let toml =
                format!("name=\"bad\"\naxiom=\"F\"\niterations=1\nangle=90.0\nstep={bad_step}");
            let err = Config::parse(&toml).unwrap_err();
            assert!(
                matches!(err, ConfigError::InvalidStep(_)),
                "step={bad_step}: unexpected error: {err}"
            );
        }
    }

    #[test]
    fn rejects_non_finite_step() {
        let toml = "name=\"bad\"\naxiom=\"F\"\niterations=1\nangle=90.0\nstep=inf";
        let err = Config::parse(toml).unwrap_err();
        assert!(
            matches!(err, ConfigError::InvalidStep(_)),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_non_finite_angle() {
        let toml = "name=\"bad\"\naxiom=\"F\"\niterations=1\nangle=nan\nstep=1.0";
        let err = Config::parse(toml).unwrap_err();
        assert!(
            matches!(err, ConfigError::InvalidAngle(_)),
            "unexpected error: {err}"
        );
    }
}
