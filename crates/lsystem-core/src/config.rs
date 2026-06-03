use std::collections::BTreeMap;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer};
use thiserror::Error;
use toml_edit::{DocumentMut, Item, Table, Value, value};

use crate::alphabet::{validate_bracket_balance, validate_symbols};

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("TOML parse error: {0}")]
    TomlParse(#[from] toml_edit::TomlError),

    #[error("TOML deserialization error: {0}")]
    TomlDeserialize(#[from] toml_edit::de::Error),

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

    #[error("invalid hex color in `{field}`: expected `#rrggbb`, got {value:?}")]
    InvalidHexColor { field: String, value: String },
}

/// Spatial dimensions of an L-system: 2D or 3D.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dimensions {
    TwoD,
    ThreeD,
}

/// A 24-bit CSS hex color (`#rrggbb`), stored as `[R, G, B]` bytes.
///
/// This type is `Copy` and `const`-constructible. Parsing requires a leading `#`
/// followed by exactly six ASCII hex digits. Shorthand (`#rgb`) and alpha (`#rrggbbaa`)
/// forms are not accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HexColor {
    bytes: [u8; 3],
}

/// Error returned when a string cannot be parsed as a `#rrggbb` hex color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("expected `#rrggbb` hex color")]
pub struct HexColorError;

impl HexColor {
    pub const BLACK: Self = Self::new(0x00, 0x00, 0x00);
    pub const DEFAULT_SOLID_LINE: Self = Self::new(0x00, 0xe6, 0x80);
    pub const DEFAULT_GRADIENT_START: Self = Self::new(0x0d, 0x59, 0x0d);
    pub const DEFAULT_GRADIENT_END: Self = Self::new(0x99, 0xe6, 0x1a);
    pub const DEFAULT_HUE_CYCLE_INITIAL: Self = Self::new(0xe6, 0x00, 0x00);

    /// Constructs a `HexColor` from raw R, G, B byte values.
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

    /// Returns a canonical lowercase `#rrggbb` CSS hex string.
    pub fn to_css_hex(self) -> String {
        format!(
            "#{:02x}{:02x}{:02x}",
            self.bytes[0], self.bytes[1], self.bytes[2]
        )
    }

    /// Returns normalized `[r, g, b]` components in `0.0..=1.0` for rendering.
    pub fn to_f32_array(self) -> [f32; 3] {
        self.bytes.map(|b| b as f32 / 255.0)
    }

    /// Converts to `(hue_degrees, saturation, value)` in HSV color space.
    pub fn to_hsv(self) -> (f32, f32, f32) {
        let [r, g, b] = self.to_f32_array();
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

impl TryFrom<&str> for HexColor {
    type Error = HexColorError;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        let hex = s.strip_prefix('#').ok_or(HexColorError)?;
        if hex.len() != 6 || !hex.is_ascii() {
            return Err(HexColorError);
        }
        let r = u8::from_str_radix(&hex[0..2], 16).map_err(|_| HexColorError)?;
        let g = u8::from_str_radix(&hex[2..4], 16).map_err(|_| HexColorError)?;
        let b = u8::from_str_radix(&hex[4..6], 16).map_err(|_| HexColorError)?;
        Ok(Self::new(r, g, b))
    }
}

impl TryFrom<String> for HexColor {
    type Error = HexColorError;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::try_from(s.as_str())
    }
}

/// Color mode for the fractal lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineColorConfig {
    Solid { color: HexColor },
    Gradient { start: HexColor, end: HexColor },
    HueCycle { initial: HexColor },
    DepthGradient { start: HexColor, end: HexColor },
}

impl LineColorConfig {
    pub const DEFAULT_SOLID: Self = Self::Solid {
        color: HexColor::DEFAULT_SOLID_LINE,
    };
    pub const DEFAULT_GRADIENT: Self = Self::Gradient {
        start: HexColor::DEFAULT_GRADIENT_START,
        end: HexColor::DEFAULT_GRADIENT_END,
    };
    pub const DEFAULT_HUE_CYCLE: Self = Self::HueCycle {
        initial: HexColor::DEFAULT_HUE_CYCLE_INITIAL,
    };
    pub const DEFAULT_DEPTH_GRADIENT: Self = Self::DepthGradient {
        start: HexColor::DEFAULT_GRADIENT_START,
        end: HexColor::DEFAULT_GRADIENT_END,
    };

    fn mode_key(&self) -> &'static str {
        match self {
            Self::Solid { .. } => "solid",
            Self::Gradient { .. } => "gradient",
            Self::HueCycle { .. } => "hue_cycle",
            Self::DepthGradient { .. } => "depth_gradient",
        }
    }

    pub fn needs_topological_depth(&self) -> bool {
        matches!(self, Self::DepthGradient { .. })
    }
}

impl Default for LineColorConfig {
    fn default() -> Self {
        Self::DEFAULT_SOLID
    }
}

/// Visual color settings for background and fractal lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ColorConfig {
    pub background: Option<HexColor>,
    pub line: LineColorConfig,
}

impl ColorConfig {
    pub const DEFAULT_BACKGROUND: HexColor = HexColor::BLACK;

    pub fn effective_background(&self) -> HexColor {
        self.background.unwrap_or(Self::DEFAULT_BACKGROUND)
    }
}

/// Parsed and validated L-System configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub name: String,
    pub generation: GenerationConfig,
    pub colors: ColorConfig,
}

impl Config {
    /// Returns the effective colors for rendering.
    ///
    /// For bracketless fractals, `DepthGradient` is treated as `Gradient` because
    /// topological depth equals segment index, making the two modes identical. This
    /// keeps export segment selection consistent with the canvas iteration cap.
    pub fn effective_colors(&self) -> ColorConfig {
        if !self.generation.has_stack_directives()
            && let LineColorConfig::DepthGradient { start, end } = self.colors.line
        {
            return ColorConfig {
                line: LineColorConfig::Gradient { start, end },
                background: self.colors.background,
            };
        }
        self.colors
    }
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
        self.axiom.contains('[') || self.rules.values().any(|rhs| rhs.contains('['))
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    metadata: RawMetadata,
    #[serde(rename = "l-system")]
    l_system: RawLSystem,
    turtle: RawTurtle,
    colors: RawColors,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMetadata {
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLSystem {
    dimensions: RawDimensions,
    axiom: String,
    iterations: u32,
    rules: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
enum RawDimensions {
    #[serde(rename = "2D", alias = "2d")]
    TwoD,
    #[serde(rename = "3D", alias = "3d")]
    ThreeD,
}

impl From<RawDimensions> for Dimensions {
    fn from(dimensions: RawDimensions) -> Self {
        match dimensions {
            RawDimensions::TwoD => Self::TwoD,
            RawDimensions::ThreeD => Self::ThreeD,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTurtle {
    #[serde(deserialize_with = "deserialize_number")]
    angle: f64,
    #[serde(default = "default_step", deserialize_with = "deserialize_number")]
    step: f64,
    #[serde(default, deserialize_with = "deserialize_number")]
    initial_heading: f64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawColors {
    #[serde(default)]
    background: Option<String>,
    line: RawLineColor,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
enum RawLineColor {
    Solid(RawSolidColor),
    Gradient(RawGradientColors),
    HueCycle(RawHueCycleColor),
    DepthGradient(RawGradientColors),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSolidColor {
    color: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGradientColors {
    start: String,
    end: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawHueCycleColor {
    initial: String,
}

impl TryFrom<RawConfig> for Config {
    type Error = ConfigError;

    fn try_from(raw: RawConfig) -> Result<Self, Self::Error> {
        let dimensions = Dimensions::from(raw.l_system.dimensions);

        let angle = raw.turtle.angle as f32;
        let step = raw.turtle.step as f32;
        let initial_heading = raw.turtle.initial_heading as f32;
        if !step.is_finite() || step <= 0.0 {
            return Err(ConfigError::InvalidStep(step));
        }
        if !angle.is_finite() {
            return Err(ConfigError::InvalidAngle(angle));
        }
        if !initial_heading.is_finite() {
            return Err(ConfigError::InvalidInitialHeading(initial_heading));
        }

        let axiom: String = raw
            .l_system
            .axiom
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        validate_symbols(&axiom, "axiom", dimensions)?;
        validate_bracket_balance(&axiom, "axiom")?;

        let mut rules = BTreeMap::new();
        for (key, rhs_raw) in raw.l_system.rules {
            let mut key_chars = key.chars();
            let rule_key = key_chars
                .next()
                .filter(|c| c.is_ascii_alphabetic())
                .ok_or_else(|| ConfigError::InvalidRuleKey { key: key.clone() })?;
            if key_chars.next().is_some() {
                return Err(ConfigError::InvalidRuleKey { key });
            }

            let field = format!("l-system.rules.{rule_key}");
            let rhs: String = rhs_raw.chars().filter(|c| !c.is_whitespace()).collect();
            validate_symbols(&rhs, &field, dimensions)?;
            validate_bracket_balance(&rhs, &field)?;
            rules.insert(rule_key, rhs);
        }

        Ok(Self {
            name: raw.metadata.name,
            generation: GenerationConfig {
                dimensions,
                axiom,
                iterations: raw.l_system.iterations,
                angle,
                step,
                initial_heading,
                rules,
            },
            colors: ColorConfig {
                background: raw
                    .colors
                    .background
                    .map(|s| parse_hex(s, "colors.background"))
                    .transpose()?,
                line: raw.colors.line.try_into()?,
            },
        })
    }
}

impl TryFrom<RawLineColor> for LineColorConfig {
    type Error = ConfigError;

    fn try_from(raw: RawLineColor) -> Result<Self, Self::Error> {
        Ok(match raw {
            RawLineColor::Solid(raw) => Self::Solid {
                color: parse_hex(raw.color, "colors.line.color")?,
            },
            RawLineColor::Gradient(raw) => Self::Gradient {
                start: parse_hex(raw.start, "colors.line.start")?,
                end: parse_hex(raw.end, "colors.line.end")?,
            },
            RawLineColor::HueCycle(raw) => Self::HueCycle {
                initial: parse_hex(raw.initial, "colors.line.initial")?,
            },
            RawLineColor::DepthGradient(raw) => Self::DepthGradient {
                start: parse_hex(raw.start, "colors.line.start")?,
                end: parse_hex(raw.end, "colors.line.end")?,
            },
        })
    }
}

fn default_step() -> f64 {
    1.0
}

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

fn parse_hex(s: String, field: &str) -> Result<HexColor, ConfigError> {
    let result = HexColor::try_from(s.as_str());
    result.map_err(|_| ConfigError::InvalidHexColor {
        field: field.into(),
        value: s,
    })
}

/// Format-preserving TOML document for an L-system configuration.
#[derive(Debug, Clone)]
pub struct ConfigSource {
    document: DocumentMut,
}

impl ConfigSource {
    pub fn parse(toml_str: &str) -> Result<Self, ConfigError> {
        Ok(Self {
            document: toml_str.parse()?,
        })
    }

    pub fn to_toml_string(&self) -> String {
        self.document.to_string()
    }

    pub fn set_name(&mut self, name: &str) {
        if self
            .document
            .get("metadata")
            .is_none_or(|item| !item.is_table())
        {
            self.document["metadata"] = Item::Table(Table::new());
        }
        set_value_preserving_decor(&mut self.document["metadata"]["name"], Value::from(name));
    }

    pub fn set_iterations(&mut self, iterations: u32) {
        set_value_preserving_decor(
            &mut self.document["l-system"]["iterations"],
            Value::from(i64::from(iterations)),
        );
    }

    pub fn set_angle(&mut self, angle: f32) {
        set_value_preserving_decor(
            &mut self.document["turtle"]["angle"],
            Value::from(f64::from(angle)),
        );
    }

    pub fn set_initial_heading(&mut self, initial_heading: f32) {
        set_value_preserving_decor(
            &mut self.document["turtle"]["initial_heading"],
            Value::from(f64::from(initial_heading)),
        );
    }

    pub fn set_dimensions(&mut self, dimensions: Dimensions) {
        set_value_preserving_decor(
            &mut self.document["l-system"]["dimensions"],
            Value::from(match dimensions {
                Dimensions::TwoD => "2D",
                Dimensions::ThreeD => "3D",
            }),
        );
    }

    pub fn set_grammar(&mut self, axiom: &str, rules: &[(char, String)]) {
        set_value_preserving_decor(&mut self.document["l-system"]["axiom"], Value::from(axiom));
        // The rules table is replaced wholesale rather than patched in place.
        // This intentionally discards per-rule TOML comments and whitespace —
        // the grammar editor constructs a new set of rules, not incremental edits.
        let mut rules_table = Table::new();
        for (symbol, rhs) in rules {
            rules_table[&symbol.to_string()] = value(rhs.as_str());
        }
        self.document["l-system"]["rules"] = Item::Table(rules_table);
    }

    pub fn set_background(&mut self, background: Option<HexColor>) {
        match background {
            Some(hex) => {
                if self
                    .document
                    .get("colors")
                    .is_none_or(|item| !item.is_table())
                {
                    self.document["colors"] = Item::Table(Table::new());
                }
                set_value_preserving_decor(
                    &mut self.document["colors"]["background"],
                    Value::from(hex.to_css_hex()),
                );
            }
            None => {
                if let Some(colors) = self.document["colors"].as_table_mut() {
                    colors.remove("background");
                }
            }
        }
    }

    pub fn set_line_color(&mut self, line_color: &LineColorConfig) {
        let line = line_table_mut(&mut self.document);
        set_value_preserving_decor(&mut line["mode"], Value::from(line_color.mode_key()));
        match line_color {
            LineColorConfig::Solid { color } => {
                remove_inactive_line_color_keys(line, &["color"]);
                set_value_preserving_decor(&mut line["color"], Value::from(color.to_css_hex()));
            }
            LineColorConfig::Gradient { start, end } => {
                remove_inactive_line_color_keys(line, &["start", "end"]);
                set_value_preserving_decor(&mut line["start"], Value::from(start.to_css_hex()));
                set_value_preserving_decor(&mut line["end"], Value::from(end.to_css_hex()));
            }
            LineColorConfig::HueCycle { initial } => {
                remove_inactive_line_color_keys(line, &["initial"]);
                set_value_preserving_decor(&mut line["initial"], Value::from(initial.to_css_hex()));
            }
            LineColorConfig::DepthGradient { start, end } => {
                remove_inactive_line_color_keys(line, &["start", "end"]);
                set_value_preserving_decor(&mut line["start"], Value::from(start.to_css_hex()));
                set_value_preserving_decor(&mut line["end"], Value::from(end.to_css_hex()));
            }
        }
    }
}

impl std::fmt::Display for ConfigSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_toml_string())
    }
}

/// A [`ConfigSource`] paired with the validated [`Config`] it produces.
///
/// The only constructor is `TryFrom<ConfigSource>`, which validates the document and rejects
/// invalid ones. Holding a `ConfigDocument` is a runtime invariant: the document was validated
/// at construction time, so `config()` always returns the validated value derived then.
#[derive(Debug, Clone)]
pub struct ConfigDocument {
    source: ConfigSource,
    config: Config,
}

impl TryFrom<ConfigSource> for ConfigDocument {
    type Error = ConfigError;

    fn try_from(source: ConfigSource) -> Result<Self, ConfigError> {
        let raw = toml_edit::de::from_document::<RawConfig>(source.document.clone())?;
        let config = raw.try_into()?;
        Ok(Self { source, config })
    }
}

impl From<ConfigDocument> for Config {
    fn from(doc: ConfigDocument) -> Self {
        doc.config
    }
}

impl ConfigDocument {
    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn source(&self) -> &ConfigSource {
        &self.source
    }

    pub fn name(&self) -> &str {
        &self.config.name
    }

    pub fn to_toml_string(&self) -> String {
        self.source.to_toml_string()
    }
}

fn set_value_preserving_decor(item: &mut Item, mut next_value: Value) {
    debug_assert!(
        item.as_value().is_some() || item.is_none(),
        "expected scalar Value or absent item; decor will be lost for table items"
    );
    if let Some(current_value) = item.as_value() {
        *next_value.decor_mut() = current_value.decor().clone();
    }
    *item = Item::Value(next_value);
}

const LINE_COLOR_VALUE_KEYS: &[&str] = &["color", "start", "end", "initial"];

fn remove_inactive_line_color_keys(line: &mut Table, active_keys: &[&str]) {
    for key in LINE_COLOR_VALUE_KEYS {
        if !active_keys.contains(key) {
            line.remove(key);
        }
    }
}

fn line_table_mut(document: &mut DocumentMut) -> &mut Table {
    if document.get("colors").is_none_or(|item| !item.is_table()) {
        document["colors"] = Item::Table(Table::new());
    }
    let colors = document["colors"]
        .as_table_mut()
        .expect("colors table was just ensured");
    if colors.get("line").is_none_or(|item| !item.is_table()) {
        colors["line"] = Item::Table(Table::new());
    }
    colors["line"]
        .as_table_mut()
        .expect("colors.line table was just ensured")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn parse_config(toml_str: &str) -> Result<Config, ConfigError> {
        Ok(ConfigDocument::try_from(ConfigSource::parse(toml_str)?)?.into())
    }

    fn hex(s: &str) -> HexColor {
        HexColor::try_from(s).unwrap()
    }

    fn assert_toml_deserialize_error_contains(err: ConfigError, fragments: &[&str]) -> String {
        assert!(
            matches!(err, ConfigError::TomlDeserialize(_)),
            "unexpected error: {err}"
        );
        let message = err.to_string();
        for fragment in fragments {
            assert!(
                message.contains(fragment),
                "error should mention {fragment:?}, got: {message}"
            );
        }
        message
    }

    fn assert_toml_deserialize_error_mentions_path(err: ConfigError, path: &str) {
        let message = err.to_string();
        assert!(
            matches!(err, ConfigError::TomlDeserialize(_)),
            "unexpected error: {message}"
        );
        if let Some((parent, field)) = path.rsplit_once('.') {
            assert!(
                message.contains(parent),
                "error should mention parent path {parent:?}, got: {message}"
            );
            assert!(
                message.contains(field),
                "error should mention field {field:?}, got: {message}"
            );
        } else {
            assert!(
                message.contains(path),
                "error should mention path {path:?}, got: {message}"
            );
        }
    }

    const KOCH_TOML: &str = r#"
name = "Koch Snowflake"
dimensions = "2D"
axiom = "F++F++F"
iterations = 4
angle = 60.0
step = 1.0

[rules]
F = "F-F++F-F"
"#;

    const NESTED_KOCH_TOML: &str = r##"
[metadata]
name = "Koch Snowflake"

[l-system]
dimensions = "2D"
axiom = "F++F++F"
iterations = 4

[l-system.rules]
F = "F-F++F-F"

[turtle]
angle = 60.0
step = 1.0
initial_heading = 0.0

[colors]
background = "#000000"

[colors.line]
mode = "hue_cycle"
initial = "#408080"
"##;

    fn test_toml(
        dimensions: Dimensions,
        axiom: &str,
        iterations: u32,
        angle: &str,
        step: &str,
        initial_heading: &str,
        rules: &str,
    ) -> String {
        let dimensions = match dimensions {
            Dimensions::TwoD => r#""2D""#,
            Dimensions::ThreeD => r#""3D""#,
        };
        test_toml_with_dimensions(
            dimensions,
            axiom,
            iterations,
            angle,
            step,
            initial_heading,
            rules,
        )
    }

    fn test_toml_with_dimensions(
        dimensions: &str,
        axiom: &str,
        iterations: u32,
        angle: &str,
        step: &str,
        initial_heading: &str,
        rules: &str,
    ) -> String {
        let rules_table = format!("\n[l-system.rules]\n{rules}\n");
        format!(
            r##"[metadata]
name = "test"

[l-system]
dimensions = {dimensions}
axiom = "{axiom}"
iterations = {iterations}
{rules_table}
[turtle]
angle = {angle}
step = {step}
initial_heading = {initial_heading}

[colors]
background = "#000000"

[colors.line]
mode = "solid"
color = "#00e680"
"##
        )
    }

    #[test]
    fn config_document_preserves_unmodified_toml_byte_for_byte() {
        let original = r##"# leading comment
[metadata] # keep this comment
name = "Styled"

[l-system]
dimensions = "3D"
axiom = 'F\F'
iterations = 2

[l-system.rules]
F = 'F\F'

[turtle]
angle = 22.5
step = 1.0
initial_heading = 45.0

[colors]
background = "#001a33"

[colors.line]
mode = "gradient"
start = "#1a334d"
end = "#b3cce6"
"##;

        let doc = ConfigSource::parse(original).unwrap();

        assert_eq!(doc.to_string(), original);
        assert!(ConfigDocument::try_from(doc).is_ok());
    }

    #[test]
    fn set_name_preserves_existing_value_comment() {
        let original = r##"[metadata]
name = "Old" # keep name comment

[l-system]
dimensions = "2D"
axiom = "F"
iterations = 1

[l-system.rules]
F = "FF"

[turtle]
angle = 90.0

[colors]
background = "#000000"

[colors.line]
mode = "solid"
color = "#00e680"
"##;
        let mut source = ConfigSource::parse(original).unwrap();

        source.set_name("New");

        assert!(
            source
                .to_toml_string()
                .contains(r#"name = "New" # keep name comment"#)
        );
        let config: Config = ConfigDocument::try_from(source).unwrap().into();
        assert_eq!(config.name, "New");
    }

    #[test]
    fn config_uses_generation_config_for_lsystem_and_turtle_fields() {
        let cfg = parse_config(NESTED_KOCH_TOML).unwrap();

        assert_eq!(cfg.name, "Koch Snowflake");
        assert_eq!(cfg.generation.dimensions, Dimensions::TwoD);
        assert_eq!(cfg.generation.axiom, "F++F++F");
        assert_eq!(cfg.generation.angle, 60.0);
        assert_eq!(cfg.generation.step, 1.0);
        assert_eq!(cfg.generation.initial_heading, 0.0);
        assert_eq!(cfg.generation.iterations, 4);
        assert_eq!(cfg.generation.rules[&'F'], "F-F++F-F");
    }

    #[test]
    fn parses_nested_v2_config() {
        let cfg = parse_config(NESTED_KOCH_TOML).unwrap();
        assert_eq!(cfg.name, "Koch Snowflake");
        assert_eq!(cfg.generation.dimensions, Dimensions::TwoD);
        assert_eq!(cfg.generation.axiom, "F++F++F");
        assert_eq!(cfg.generation.angle, 60.0);
        assert_eq!(cfg.generation.step, 1.0);
        assert_eq!(cfg.generation.initial_heading, 0.0);
        assert_eq!(cfg.generation.iterations, 4);
        assert_eq!(cfg.generation.rules[&'F'], "F-F++F-F");
        assert_eq!(cfg.colors.background, Some(hex("#000000")));
        match cfg.colors.line {
            LineColorConfig::HueCycle { initial } => assert_eq!(initial, hex("#408080")),
            other => panic!("expected hue cycle line color, got {other:?}"),
        }
    }

    #[test]
    fn parses_missing_background_as_none() {
        let toml = NESTED_KOCH_TOML.replace("background = \"#000000\"\n\n", "");
        let cfg = parse_config(&toml).unwrap();

        assert_eq!(cfg.colors.background, None);
    }

    #[test]
    fn parses_present_background_as_some_hex() {
        let toml = NESTED_KOCH_TOML.replace("background = \"#000000\"", "background = \"#334d66\"");
        let cfg = parse_config(&toml).unwrap();

        assert_eq!(cfg.colors.background, Some(hex("#334d66")));
    }

    #[test]
    fn rejects_old_array_background() {
        let toml =
            NESTED_KOCH_TOML.replace("background = \"#000000\"", "background = [0.0, 0.0, 0.0]");
        let err = parse_config(&toml).unwrap_err();

        assert!(
            matches!(err, ConfigError::TomlDeserialize(_)),
            "expected TomlDeserialize error for array background, got: {err}"
        );
    }

    #[test]
    fn rejects_invalid_hex_for_background() {
        let toml = NESTED_KOCH_TOML.replace("background = \"#000000\"", "background = \"notahex\"");
        let err = parse_config(&toml).unwrap_err();

        assert!(
            matches!(
                err,
                ConfigError::InvalidHexColor {
                    ref field,
                    ref value
                } if field == "colors.background" && value == "notahex"
            ),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn accepts_uppercase_hex_background() {
        let toml = NESTED_KOCH_TOML.replace("background = \"#000000\"", "background = \"#AABBCC\"");
        let cfg = parse_config(&toml).unwrap();

        assert_eq!(cfg.colors.background, Some(hex("#aabbcc")));
    }

    #[test]
    fn line_color_config_exposes_mode_defaults() {
        assert_eq!(
            LineColorConfig::DEFAULT_SOLID,
            LineColorConfig::Solid {
                color: hex("#00e680"),
            }
        );
        assert_eq!(LineColorConfig::default(), LineColorConfig::DEFAULT_SOLID);
        assert_eq!(
            LineColorConfig::DEFAULT_GRADIENT,
            LineColorConfig::Gradient {
                start: hex("#0d590d"),
                end: hex("#99e61a"),
            }
        );
        assert_eq!(
            LineColorConfig::DEFAULT_HUE_CYCLE,
            LineColorConfig::HueCycle {
                initial: hex("#e60000"),
            }
        );
        assert_eq!(
            LineColorConfig::DEFAULT_DEPTH_GRADIENT,
            LineColorConfig::DepthGradient {
                start: hex("#0d590d"),
                end: hex("#99e61a"),
            }
        );
    }

    #[test]
    fn parses_solid_line_color() {
        let cfg = parse_config(&test_toml(
            Dimensions::TwoD,
            "F",
            1,
            "90.0",
            "1.0",
            "0.0",
            "",
        ))
        .unwrap();

        assert_eq!(
            cfg.colors.line,
            LineColorConfig::Solid {
                color: hex("#00e680"),
            }
        );
    }

    #[test]
    fn parses_gradient_line_color() {
        let toml = test_toml(Dimensions::TwoD, "F", 1, "90.0", "1.0", "0.0", "").replace(
            "mode = \"solid\"\ncolor = \"#00e680\"",
            "mode = \"gradient\"\nstart = \"#1a334d\"\nend = \"#b3cce6\"",
        );

        let cfg = parse_config(&toml).unwrap();

        assert_eq!(
            cfg.colors.line,
            LineColorConfig::Gradient {
                start: hex("#1a334d"),
                end: hex("#b3cce6"),
            }
        );
    }

    #[test]
    fn parses_depth_gradient_line_color() {
        let toml = test_toml(Dimensions::TwoD, "F", 1, "90.0", "1.0", "0.0", "").replace(
            "mode = \"solid\"\ncolor = \"#00e680\"",
            "mode = \"depth_gradient\"\nstart = \"#1a334d\"\nend = \"#b3cce6\"",
        );

        let cfg = parse_config(&toml).unwrap();

        assert_eq!(
            cfg.colors.line,
            LineColorConfig::DepthGradient {
                start: hex("#1a334d"),
                end: hex("#b3cce6"),
            }
        );
    }

    #[test]
    fn parses_hue_cycle_line_color() {
        let toml = test_toml(Dimensions::TwoD, "F", 1, "90.0", "1.0", "0.0", "").replace(
            "mode = \"solid\"\ncolor = \"#00e680\"",
            "mode = \"hue_cycle\"\ninitial = \"#e60000\"",
        );

        let cfg = parse_config(&toml).unwrap();

        assert_eq!(
            cfg.colors.line,
            LineColorConfig::HueCycle {
                initial: hex("#e60000"),
            }
        );
    }

    #[test]
    fn rejects_invalid_hex_for_initial() {
        let toml = test_toml(Dimensions::TwoD, "F", 1, "90.0", "1.0", "0.0", "").replace(
            "mode = \"solid\"\ncolor = \"#00e680\"",
            "mode = \"hue_cycle\"\ninitial = \"#zzzzzz\"",
        );
        let err = parse_config(&toml).unwrap_err();
        assert!(
            matches!(
                err,
                ConfigError::InvalidHexColor { ref field, .. } if field == "colors.line.initial"
            ),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_missing_depth_gradient_keys() {
        let cases = [
            (
                "mode = \"depth_gradient\"\nstart = \"#1a334d\"",
                "colors.line.end",
            ),
            (
                "mode = \"depth_gradient\"\nend = \"#b3cce6\"",
                "colors.line.start",
            ),
        ];

        for (replacement, missing_field) in cases {
            let toml = test_toml(Dimensions::TwoD, "F", 1, "90.0", "1.0", "0.0", "")
                .replace("mode = \"solid\"\ncolor = \"#00e680\"", replacement);

            let err = parse_config(&toml).unwrap_err();

            assert_toml_deserialize_error_mentions_path(err, missing_field);
        }
    }

    #[test]
    fn parses_dotted_v2_config() {
        let toml = r##"
metadata.name = "Dotted"
l-system.dimensions = "3D"
l-system.axiom = 'F\F'
l-system.iterations = 2
l-system.rules.F = 'F\F'
turtle.angle = 45.0
turtle.step = 1.0
turtle.initial_heading = 0.0
colors.background = "#000000"
colors.line.mode = "solid"
colors.line.color = "#00e680"
"##;

        let cfg = parse_config(toml).unwrap();

        assert_eq!(cfg.name, "Dotted");
        assert_eq!(cfg.generation.dimensions, Dimensions::ThreeD);
        assert_eq!(cfg.generation.axiom, "F\\F");
        assert_eq!(cfg.generation.rules[&'F'], "F\\F");
    }

    #[test]
    fn parses_implicit_parent_tables() {
        let toml = r##"
colors.background = "#000000"

[metadata]
name = "Implicit Parents"

[l-system]
dimensions = "2D"
axiom = "F"
iterations = 1

[l-system.rules]
F = "FF"

[turtle]
angle = 60.0
step = 1.0
initial_heading = 0.0

[colors.line]
mode = "solid"
color = "#00e680"
"##;

        let cfg = parse_config(toml).unwrap();

        assert_eq!(cfg.name, "Implicit Parents");
        assert_eq!(cfg.generation.dimensions, Dimensions::TwoD);
    }

    #[test]
    fn rejects_flat_v1_schema() {
        assert!(
            parse_config(KOCH_TOML).is_err(),
            "flat v1 TOML must not parse as v2"
        );
    }

    #[test]
    fn config_document_preserves_unchanged_toml() {
        let toml = r##"# preset comment
[metadata]
name = 'Formatted'

[l-system] # keep this suffix
dimensions = "3D"
axiom = 'F\F'
iterations = 2

[l-system.rules]
F = 'F\F'

[turtle]
angle = 45.0
step = 1.0
initial_heading = 0.0

[colors]
background = "#000000"

[colors.line]
mode = "solid"
color = "#1a334d"
"##;

        let source = ConfigSource::parse(toml).unwrap();

        assert_eq!(source.to_toml_string(), toml);
        let cfg: Config = ConfigDocument::try_from(source).unwrap().into();
        assert_eq!(cfg.generation.axiom, "F\\F");
        assert_eq!(cfg.generation.rules[&'F'], "F\\F");
    }

    #[test]
    fn parses_valid_config() {
        let cfg = parse_config(NESTED_KOCH_TOML).unwrap();
        assert_eq!(cfg.generation.axiom, "F++F++F");
        assert_eq!(cfg.generation.angle, 60.0);
        assert_eq!(cfg.generation.step, 1.0);
        assert_eq!(cfg.generation.iterations, 4);
        assert_eq!(cfg.generation.rules[&'F'], "F-F++F-F");
    }

    #[test]
    fn missing_step_defaults_to_one() {
        let toml = r##"
[metadata]
name = "test"

[l-system]
dimensions = "2D"
axiom = "F"
iterations = 1

[l-system.rules]
F = "FF"

[turtle]
angle = 90.0

[colors]
background = "#000000"

[colors.line]
mode = "solid"
color = "#00e680"
"##;
        let config = parse_config(toml).unwrap();
        assert_eq!(config.generation.step, 1.0);
        assert_eq!(config.generation.initial_heading, 0.0);
    }

    #[test]
    fn missing_initial_heading_defaults_to_zero() {
        let toml = NESTED_KOCH_TOML.replace("initial_heading = 0.0\n", "");
        let config = parse_config(&toml).unwrap();
        assert_eq!(config.generation.initial_heading, 0.0);
    }

    #[test]
    fn rejects_missing_angle() {
        let toml = NESTED_KOCH_TOML.replace("angle = 60.0\n", "");
        let err = parse_config(&toml).unwrap_err();
        assert_toml_deserialize_error_contains(err, &["turtle", "angle"]);
    }

    #[test]
    fn accepts_integer_turtle_numbers() {
        let toml = test_toml(Dimensions::TwoD, "F", 1, "90", "1", "0", "");
        let config = parse_config(&toml).unwrap();
        assert_eq!(config.generation.angle, 90.0);
        assert_eq!(config.generation.step, 1.0);
        assert_eq!(config.generation.initial_heading, 0.0);
    }

    #[test]
    fn rejects_non_numeric_turtle_values() {
        for (field, source, replacement) in [
            ("angle", "angle = 60.0", r#"angle = "90""#),
            ("step", "step = 1.0", r#"step = "1""#),
            (
                "initial_heading",
                "initial_heading = 0.0",
                r#"initial_heading = "0""#,
            ),
        ] {
            let toml = NESTED_KOCH_TOML.replace(source, replacement);
            let err = parse_config(&toml).unwrap_err();

            assert_toml_deserialize_error_contains(err, &["turtle", field]);
        }
    }

    #[test]
    fn rejects_missing_rules_table() {
        let toml = r##"
[metadata]
name = "test"

[l-system]
dimensions = "2D"
axiom = "F"
iterations = 1

[turtle]
angle = 90.0
step = 1.0
initial_heading = 0.0

[colors]
background = "#000000"

[colors.line]
mode = "solid"
color = "#00e680"
"##;
        assert!(parse_config(toml).is_err());
    }

    #[test]
    fn rejects_invalid_hex_for_background_in_test_toml() {
        let toml = test_toml(Dimensions::TwoD, "F", 1, "90.0", "1.0", "0.0", "");
        let toml = toml.replace("background = \"#000000\"", "background = \"notahex\"");
        let err = parse_config(&toml).unwrap_err();
        assert!(
            matches!(
                err,
                ConfigError::InvalidHexColor {
                    ref field,
                    ref value
                } if field == "colors.background" && value == "notahex"
            ),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_invalid_hex_for_depth_gradient_colors() {
        for (expected_field, replacement) in [
            (
                "colors.line.start",
                "mode = \"depth_gradient\"\nstart = \"bad\"\nend = \"#b3cce6\"",
            ),
            (
                "colors.line.end",
                "mode = \"depth_gradient\"\nstart = \"#1a334d\"\nend = \"bad\"",
            ),
        ] {
            let toml = test_toml(Dimensions::TwoD, "F", 1, "90.0", "1.0", "0.0", "")
                .replace("mode = \"solid\"\ncolor = \"#00e680\"", replacement);
            let err = parse_config(&toml).unwrap_err();
            assert!(
                matches!(
                    err,
                    ConfigError::InvalidHexColor {
                        field: ref error_field,
                        ref value
                    } if error_field == expected_field && value == "bad"
                ),
                "unexpected error: {err}"
            );
        }
    }

    #[test]
    fn rejects_invalid_hex_for_line_color() {
        let toml = test_toml(Dimensions::TwoD, "F", 1, "90.0", "1.0", "0.0", "");
        let toml = toml.replace("color = \"#00e680\"", "color = \"#zzzzzz\"");
        let err = parse_config(&toml).unwrap_err();
        assert!(
            matches!(
                err,
                ConfigError::InvalidHexColor {
                    ref field,
                    ..
                } if field == "colors.line.color"
            ),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_old_array_for_line_color() {
        let toml = test_toml(Dimensions::TwoD, "F", 1, "90.0", "1.0", "0.0", "");
        let toml = toml.replace("color = \"#00e680\"", "color = [0.0, 0.9, 0.5]");
        let err = parse_config(&toml).unwrap_err();
        assert!(
            matches!(err, ConfigError::TomlDeserialize(_)),
            "expected TomlDeserialize error for array line color, got: {err}"
        );
    }

    #[test]
    fn rejects_unknown_line_color_mode() {
        let toml = test_toml(Dimensions::TwoD, "F", 1, "90.0", "1.0", "0.0", "");
        let toml = toml.replace("mode = \"solid\"", "mode = \"rainbow\"");
        let err = parse_config(&toml).unwrap_err();

        assert_toml_deserialize_error_contains(err, &["colors.line", "rainbow"]);
    }

    #[test]
    fn rejects_extra_keys_for_line_color_modes() {
        let cases = [
            (
                "solid",
                "mode = \"solid\"\ncolor = \"#00e680\"\nstart = \"#000000\"",
                "colors.line.start",
            ),
            (
                "gradient",
                "mode = \"gradient\"\nstart = \"#000000\"\nend = \"#ffffff\"\ncolor = \"#00e680\"",
                "colors.line.color",
            ),
            (
                "depth_gradient",
                "mode = \"depth_gradient\"\nstart = \"#000000\"\nend = \"#ffffff\"\ncolor = \"#00e680\"",
                "colors.line.color",
            ),
            (
                "hue_cycle",
                "mode = \"hue_cycle\"\ninitial = \"#00e680\"\nend = \"#ffffff\"",
                "colors.line.end",
            ),
        ];

        for (_mode, replacement, expected_field) in cases {
            let toml = test_toml(Dimensions::TwoD, "F", 1, "90.0", "1.0", "0.0", "");
            let toml = toml.replace("mode = \"solid\"\ncolor = \"#00e680\"", replacement);
            let err = parse_config(&toml).unwrap_err();

            assert_toml_deserialize_error_mentions_path(err, expected_field);
        }
    }

    #[test]
    fn rejects_unknown_schema_fields() {
        let toml = format!("{}\n[experimental]\nfoo = true\n", NESTED_KOCH_TOML);
        let err = parse_config(&toml).unwrap_err();

        assert_toml_deserialize_error_contains(err, &["experimental"]);

        let toml = NESTED_KOCH_TOML.replace(
            "initial_heading = 0.0",
            "initial_heading = 0.0\nfriction = 0.5",
        );
        let err = parse_config(&toml).unwrap_err();

        assert_toml_deserialize_error_contains(err, &["turtle", "friction"]);
    }

    #[test]
    fn bundled_presets_are_parseable() {
        let presets_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../presets");
        let mut preset_paths: Vec<_> = std::fs::read_dir(&presets_dir)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", presets_dir.display()))
            .map(|entry| entry.expect("failed to read preset dir entry").path())
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("toml"))
            .collect();
        preset_paths.sort();

        assert!(!preset_paths.is_empty(), "no preset TOML files found");
        for path in preset_paths {
            let toml = std::fs::read_to_string(&path)
                .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
            parse_config(&toml)
                .unwrap_or_else(|err| panic!("failed to parse {}: {err}", path.display()));
        }
    }

    #[test]
    fn strips_whitespace_from_axiom() {
        let toml = test_toml(
            Dimensions::TwoD,
            "F + + F",
            1,
            "90.0",
            "1.0",
            "0.0",
            r#"F = "F - F""#,
        );
        let cfg = parse_config(&toml).unwrap();
        assert_eq!(cfg.generation.axiom, "F++F");
        assert_eq!(cfg.generation.rules[&'F'], "F-F");
    }

    #[test]
    fn rejects_digit_in_axiom() {
        let toml = test_toml(Dimensions::TwoD, "F+1", 1, "90.0", "1.0", "0.0", "");
        let err = parse_config(&toml).unwrap_err();
        assert!(
            matches!(err, ConfigError::InvalidSymbol { ch: '1', .. }),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_multi_char_rule_key() {
        let toml = test_toml(
            Dimensions::TwoD,
            "F",
            1,
            "90.0",
            "1.0",
            "0.0",
            r#"FF = "FFF""#,
        );
        let err = parse_config(&toml).unwrap_err();
        assert!(
            matches!(err, ConfigError::InvalidRuleKey { .. }),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn parses_dimensions_strings_and_lowercase_aliases() {
        for (raw_dimensions, expected) in [
            (r#""2D""#, Dimensions::TwoD),
            (r#""3D""#, Dimensions::ThreeD),
            (r#""2d""#, Dimensions::TwoD),
            (r#""3d""#, Dimensions::ThreeD),
        ] {
            let toml = test_toml_with_dimensions(raw_dimensions, "F", 1, "90.0", "1.0", "0.0", "");
            let config = parse_config(&toml).unwrap();
            assert_eq!(config.generation.dimensions, expected);
        }
    }

    #[test]
    fn rejects_invalid_dimensions() {
        for bad_dim in ["1", "2", "3", "4", "300", r#""4D""#] {
            let toml = test_toml_with_dimensions(bad_dim, "F", 1, "90.0", "1.0", "0.0", "");
            let err = parse_config(&toml).unwrap_err();
            assert_toml_deserialize_error_contains(err, &["dimensions"]);
        }
    }

    #[test]
    fn rejects_non_finite_initial_heading() {
        let toml = test_toml(Dimensions::TwoD, "F", 1, "90.0", "1.0", "nan", "");
        let err = parse_config(&toml).unwrap_err();

        assert!(
            matches!(err, ConfigError::InvalidInitialHeading(_)),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn accepts_dimensions_3_with_3d_symbols() {
        let toml = test_toml(Dimensions::ThreeD, "F&F^F/F", 0, "90.0", "1.0", "0.0", "");
        parse_config(&toml).expect("3D config with 3D symbols should be valid");
    }

    #[test]
    fn rejects_3d_symbols_in_2d_config() {
        let toml = test_toml(Dimensions::TwoD, "F&F", 1, "90.0", "1.0", "0.0", "");
        let err = parse_config(&toml).unwrap_err();
        assert!(
            matches!(err, ConfigError::InvalidSymbol { ch: '&', .. }),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_unmatched_close_bracket_in_axiom() {
        let toml = test_toml(Dimensions::TwoD, "F]F", 1, "90.0", "1.0", "0.0", "");
        let err = parse_config(&toml).unwrap_err();
        assert!(
            matches!(err, ConfigError::UnmatchedClose { position: 1, .. }),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_unclosed_open_bracket_in_axiom() {
        let toml = test_toml(Dimensions::TwoD, "F[F", 1, "90.0", "1.0", "0.0", "");
        let err = parse_config(&toml).unwrap_err();
        assert!(
            matches!(err, ConfigError::UnmatchedOpen { position: 1, .. }),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn reports_first_unclosed_bracket_not_last() {
        // "F[F[F": two unclosed brackets at positions 1 and 3; error must point to 1.
        let toml = test_toml(Dimensions::TwoD, "F[F[F", 1, "90.0", "1.0", "0.0", "");
        let err = parse_config(&toml).unwrap_err();
        assert!(
            matches!(err, ConfigError::UnmatchedOpen { position: 1, .. }),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_unbalanced_brackets_in_rule() {
        let toml = test_toml(
            Dimensions::TwoD,
            "F",
            1,
            "90.0",
            "1.0",
            "0.0",
            r#"F = "F[+F""#,
        );
        let err = parse_config(&toml).unwrap_err();
        assert!(
            matches!(err, ConfigError::UnmatchedOpen { .. }),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_non_positive_step() {
        for bad_step in ["0.0", "-1.0"] {
            let toml = test_toml(Dimensions::TwoD, "F", 1, "90.0", bad_step, "0.0", "");
            let err = parse_config(&toml).unwrap_err();
            assert!(
                matches!(err, ConfigError::InvalidStep(_)),
                "step={bad_step}: unexpected error: {err}"
            );
        }
    }

    #[test]
    fn rejects_non_finite_step() {
        let toml = test_toml(Dimensions::TwoD, "F", 1, "90.0", "inf", "0.0", "");
        let err = parse_config(&toml).unwrap_err();
        assert!(
            matches!(err, ConfigError::InvalidStep(_)),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_non_finite_angle() {
        let toml = test_toml(Dimensions::TwoD, "F", 1, "nan", "1.0", "0.0", "");
        let err = parse_config(&toml).unwrap_err();
        assert!(
            matches!(err, ConfigError::InvalidAngle(_)),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn has_stack_directives_false_for_bracketless() {
        let toml = test_toml(
            Dimensions::TwoD,
            "F-F++F-F",
            1,
            "60.0",
            "1.0",
            "0.0",
            "F = \"F-F++F-F\"",
        );
        let cfg = parse_config(&toml).unwrap();
        assert!(!cfg.generation.has_stack_directives());
    }

    #[test]
    fn has_stack_directives_true_when_axiom_has_bracket() {
        let toml = test_toml(Dimensions::TwoD, "F[+F]F", 1, "25.0", "1.0", "0.0", "");
        let cfg = parse_config(&toml).unwrap();
        assert!(cfg.generation.has_stack_directives());
    }

    #[test]
    fn has_stack_directives_true_when_rule_has_bracket() {
        let toml = test_toml(
            Dimensions::TwoD,
            "F",
            1,
            "25.0",
            "1.0",
            "0.0",
            "F = \"F[+F]F[-F]F\"",
        );
        let cfg = parse_config(&toml).unwrap();
        assert!(cfg.generation.has_stack_directives());
    }

    #[test]
    fn effective_colors_normalizes_depth_gradient_to_gradient_for_bracketless() {
        let toml = test_toml(Dimensions::TwoD, "F-F++F-F", 1, "60.0", "1.0", "0.0", "");
        let mut cfg = parse_config(&toml).unwrap();
        let start = hex("#1a334d");
        let end = hex("#b3cce6");
        cfg.colors.line = LineColorConfig::DepthGradient { start, end };
        let effective = cfg.effective_colors();
        assert_eq!(
            effective.line,
            LineColorConfig::Gradient { start, end },
            "bracketless DepthGradient must normalize to Gradient"
        );
        assert_eq!(
            effective.background, cfg.colors.background,
            "background must be preserved during normalization"
        );
        assert_eq!(
            cfg.colors.line,
            LineColorConfig::DepthGradient { start, end },
            "effective_colors must not mutate the stored config"
        );
    }

    #[test]
    fn effective_colors_preserves_depth_gradient_for_bracket_fractal() {
        let toml = test_toml(
            Dimensions::TwoD,
            "F",
            1,
            "25.0",
            "1.0",
            "0.0",
            "F = \"F[+F]F[-F]F\"",
        );
        let mut cfg = parse_config(&toml).unwrap();
        let start = hex("#1a334d");
        let end = hex("#b3cce6");
        cfg.colors.line = LineColorConfig::DepthGradient { start, end };
        let effective = cfg.effective_colors();
        assert_eq!(
            effective.line,
            LineColorConfig::DepthGradient { start, end },
            "bracket fractal DepthGradient must be preserved"
        );
    }

    #[test]
    fn effective_colors_passes_through_solid_for_bracketless() {
        let toml = test_toml(Dimensions::TwoD, "F-F++F-F", 1, "60.0", "1.0", "0.0", "");
        let cfg = parse_config(&toml).unwrap();
        let effective = cfg.effective_colors();
        assert_eq!(effective.line, cfg.colors.line);
        assert_eq!(effective.background, cfg.colors.background);
    }

    #[test]
    fn effective_colors_passes_through_gradient_for_bracketless() {
        let toml = test_toml(Dimensions::TwoD, "F-F++F-F", 1, "60.0", "1.0", "0.0", "");
        let mut cfg = parse_config(&toml).unwrap();
        cfg.colors.line = LineColorConfig::Gradient {
            start: hex("#ff0000"),
            end: hex("#0000ff"),
        };
        let effective = cfg.effective_colors();
        assert_eq!(effective.line, cfg.colors.line);
        assert_eq!(effective.background, cfg.colors.background);
    }

    #[test]
    fn effective_colors_passes_through_hue_cycle_for_bracketless() {
        let toml = test_toml(Dimensions::TwoD, "F-F++F-F", 1, "60.0", "1.0", "0.0", "");
        let mut cfg = parse_config(&toml).unwrap();
        cfg.colors.line = LineColorConfig::HueCycle {
            initial: hex("#ff0000"),
        };
        let effective = cfg.effective_colors();
        assert_eq!(effective.line, cfg.colors.line);
        assert_eq!(effective.background, cfg.colors.background);
    }

    #[test]
    fn set_background_writes_hex_string() {
        let toml = test_toml(Dimensions::TwoD, "F", 1, "90.0", "1.0", "0.0", "");
        let mut source = ConfigSource::parse(&toml).unwrap();
        source.set_background(Some(HexColor::new(0xff, 0x80, 0x00)));
        let result = source.to_toml_string();
        assert!(
            result.contains(r##"background = "#ff8000""##),
            "expected hex string, got: {result}"
        );
        // Verify it round-trips: re-parsing produces the same HexColor
        let doc = ConfigDocument::try_from(ConfigSource::parse(&result).unwrap()).unwrap();
        assert_eq!(
            doc.config().colors.background,
            Some(HexColor::new(0xff, 0x80, 0x00))
        );
    }

    #[test]
    fn set_line_color_writes_hex_strings() {
        let toml = test_toml(Dimensions::TwoD, "F", 1, "90.0", "1.0", "0.0", "");
        let mut source = ConfigSource::parse(&toml).unwrap();
        source.set_line_color(&LineColorConfig::Gradient {
            start: HexColor::new(0x00, 0x11, 0x22),
            end: HexColor::new(0xaa, 0xbb, 0xcc),
        });
        let result = source.to_toml_string();
        assert!(
            result.contains(r##"start = "#001122""##),
            "expected start hex, got: {result}"
        );
        assert!(
            result.contains(r##"end = "#aabbcc""##),
            "expected end hex, got: {result}"
        );
        // Round-trip
        let doc = ConfigDocument::try_from(ConfigSource::parse(&result).unwrap()).unwrap();
        assert_eq!(
            doc.config().colors.line,
            LineColorConfig::Gradient {
                start: HexColor::new(0x00, 0x11, 0x22),
                end: HexColor::new(0xaa, 0xbb, 0xcc),
            }
        );
    }
}

#[cfg(test)]
mod hex_color {
    use super::*;

    // --- Parsing ---

    #[test]
    fn try_from_accepts_lowercase_hex() {
        let hex = HexColor::try_from("#00e680").unwrap();
        assert_eq!(hex.r(), 0x00);
        assert_eq!(hex.g(), 0xe6);
        assert_eq!(hex.b(), 0x80);
    }

    #[test]
    fn try_from_accepts_uppercase_hex() {
        let hex = HexColor::try_from("#00E680").unwrap();
        assert_eq!(hex.r(), 0x00);
        assert_eq!(hex.g(), 0xe6);
        assert_eq!(hex.b(), 0x80);
    }

    #[test]
    fn try_from_string_accepts_lowercase_hex() {
        let hex = HexColor::try_from(String::from("#00e680")).unwrap();
        assert_eq!(hex, HexColor::new(0x00, 0xe6, 0x80));
    }

    // --- to_css_hex ---

    #[test]
    fn to_css_hex_emits_canonical_lowercase() {
        assert_eq!(HexColor::new(0x00, 0xe6, 0x80).to_css_hex(), "#00e680");
        assert_eq!(HexColor::new(0xAB, 0xCD, 0xEF).to_css_hex(), "#abcdef");
        assert_eq!(HexColor::BLACK.to_css_hex(), "#000000");
    }

    // --- Rejection cases ---

    #[test]
    fn try_from_rejects_missing_leading_hash() {
        assert_eq!(HexColor::try_from("00e680"), Err(HexColorError));
    }

    #[test]
    fn try_from_rejects_too_short() {
        // 5 hex chars after #
        assert_eq!(HexColor::try_from("#0e680"), Err(HexColorError));
    }

    #[test]
    fn try_from_rejects_too_long() {
        // 7 hex chars after #
        assert_eq!(HexColor::try_from("#00e6800"), Err(HexColorError));
    }

    #[test]
    fn try_from_rejects_non_hex_character() {
        assert_eq!(HexColor::try_from("#00g680"), Err(HexColorError));
        assert_eq!(HexColor::try_from("#zzzzzz"), Err(HexColorError));
    }

    #[test]
    fn try_from_rejects_shorthand() {
        // #rgb — 3 chars
        assert_eq!(HexColor::try_from("#0e6"), Err(HexColorError));
    }

    #[test]
    fn try_from_rejects_alpha_form() {
        // #rrggbbaa — 8 chars
        assert_eq!(HexColor::try_from("#00e680ff"), Err(HexColorError));
    }

    #[test]
    fn try_from_rejects_full_width_lookalikes() {
        // Full-width '#' (U+FF03) followed by 6 ASCII hex chars
        assert_eq!(HexColor::try_from("\u{FF03}00e680"), Err(HexColorError));
        // Full-width digits
        assert_eq!(
            HexColor::try_from("#\u{FF10}\u{FF10}e680"),
            Err(HexColorError)
        );
    }

    #[test]
    fn try_from_rejects_non_ascii_same_byte_length() {
        // Two 3-byte UTF-8 chars → 6 bytes total, would panic without is_ascii() guard
        assert_eq!(HexColor::try_from("#\u{0800}\u{0800}"), Err(HexColorError));
    }

    #[test]
    fn try_from_rejects_empty_string() {
        assert_eq!(HexColor::try_from(""), Err(HexColorError));
    }

    // --- to_f32_array ---

    #[test]
    fn to_f32_array_maps_bytes_correctly() {
        let hex = HexColor::new(0x00, 0xff, 0x80);
        let [r, g, b] = hex.to_f32_array();
        assert_eq!(r, 0.0);
        assert_eq!(g, 1.0);
        assert!((b - 128.0 / 255.0).abs() < f32::EPSILON);
    }

    #[test]
    fn to_f32_array_black() {
        assert_eq!(
            HexColor::new(0x00, 0x00, 0x00).to_f32_array(),
            [0.0, 0.0, 0.0]
        );
    }

    #[test]
    fn to_f32_array_white() {
        assert_eq!(
            HexColor::new(0xff, 0xff, 0xff).to_f32_array(),
            [1.0, 1.0, 1.0]
        );
    }

    // --- Constants ---

    #[test]
    fn constants_have_correct_css_hex() {
        assert_eq!(HexColor::BLACK.to_css_hex(), "#000000");
        assert_eq!(HexColor::DEFAULT_SOLID_LINE.to_css_hex(), "#00e680");
        assert_eq!(HexColor::DEFAULT_GRADIENT_START.to_css_hex(), "#0d590d");
        assert_eq!(HexColor::DEFAULT_GRADIENT_END.to_css_hex(), "#99e61a");
        assert_eq!(HexColor::DEFAULT_HUE_CYCLE_INITIAL.to_css_hex(), "#e60000");
    }

    // --- to_hsv ---

    const EPS: f32 = 1e-5;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < EPS
    }

    #[test]
    fn to_hsv_converts_correctly() {
        // 0x40=64, 0x80=128 → r=64/255≈0.251, g=b=128/255≈0.502 → hue=180°, sat≈0.5, val≈0.502
        let (hue, saturation, value) = HexColor::new(0x40, 0x80, 0x80).to_hsv();
        assert!(close(hue, 180.0));
        assert!(close(saturation, 0.5));
        assert!(close(value, 128.0 / 255.0));
    }

    #[test]
    fn to_hsv_grayscale_has_zero_saturation_and_hue() {
        let (hue, saturation, value) = HexColor::new(0x66, 0x66, 0x66).to_hsv();
        assert!(close(hue, 0.0));
        assert!(close(saturation, 0.0));
        assert!(close(value, 0x66 as f32 / 255.0));
    }

    #[test]
    fn to_hsv_pure_blue_maps_to_blue_hue() {
        let (hue, saturation, value) = HexColor::new(0x00, 0x00, 0xff).to_hsv();
        assert!(close(hue, 240.0));
        assert!(close(saturation, 1.0));
        assert!(close(value, 1.0));
    }

    #[test]
    fn to_hsv_black_has_zero_saturation_and_value() {
        let (hue, saturation, value) = HexColor::BLACK.to_hsv();
        assert!(close(hue, 0.0));
        assert!(close(saturation, 0.0));
        assert!(close(value, 0.0));
    }

    #[test]
    fn to_hsv_red_dominant_negative_hue_wraps() {
        // Red-dominant with b > g: the raw (g-b)/delta is negative; rem_euclid keeps it positive.
        let hex = HexColor::new(0xff, 0x00, 0x80);
        let (hue, saturation, value) = hex.to_hsv();
        assert!(hue > 0.0 && hue < 360.0, "hue {hue} must be in [0, 360)");
        assert!(close(saturation, 1.0));
        assert!(close(value, 1.0));
        // Verify the exact value matches the implementation for this input.
        let [r, g, b] = hex.to_f32_array();
        let expected = 60.0 * ((g - b) / (r - 0.0_f32)).rem_euclid(6.0);
        assert!(close(hue, expected));
    }
}
