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

    #[error(
        "color component `{component}` in `{field}` must be finite and in 0.0..=1.0, got {value}"
    )]
    InvalidColorComponent {
        field: String,
        component: usize,
        value: f32,
    },
}

/// Spatial dimensions of an L-system: 2D or 3D.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dimensions {
    TwoD,
    ThreeD,
}

/// Validated RGB color with components in `0.0..=1.0`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgb {
    components: [f32; 3],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("RGB components must be finite and in 0.0..=1.0")]
pub struct RgbError;

impl Rgb {
    pub const BLACK: Self = Self::from_array_unchecked([0.0, 0.0, 0.0]);
    pub const DEFAULT_SOLID_LINE: Self = Self::from_array_unchecked([0.0, 0.9, 0.5]);
    pub const DEFAULT_GRADIENT_START: Self = Self::from_array_unchecked([0.05, 0.35, 0.05]);
    pub const DEFAULT_GRADIENT_END: Self = Self::from_array_unchecked([0.6, 0.9, 0.1]);
    pub const DEFAULT_HUE_CYCLE_INITIAL: Self = Self::from_array_unchecked([0.9, 0.0, 0.0]);

    const fn from_array_unchecked(components: [f32; 3]) -> Self {
        Self { components }
    }

    pub const fn red(self) -> f32 {
        self.components[0]
    }

    pub const fn green(self) -> f32 {
        self.components[1]
    }

    pub const fn blue(self) -> f32 {
        self.components[2]
    }

    pub const fn to_array(self) -> [f32; 3] {
        self.components
    }
}

impl TryFrom<[f32; 3]> for Rgb {
    type Error = RgbError;

    fn try_from(components: [f32; 3]) -> Result<Self, Self::Error> {
        invalid_rgb_component(components)
            .is_none()
            .then_some(Self { components })
            .ok_or(RgbError)
    }
}

/// Color mode for the fractal lines.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LineColorConfig {
    Solid { color: Rgb },
    Gradient { start: Rgb, end: Rgb },
    HueCycle { initial: Rgb },
    DepthGradient { start: Rgb, end: Rgb },
}

impl LineColorConfig {
    pub const DEFAULT_SOLID: Self = Self::Solid {
        color: Rgb::DEFAULT_SOLID_LINE,
    };
    pub const DEFAULT_GRADIENT: Self = Self::Gradient {
        start: Rgb::DEFAULT_GRADIENT_START,
        end: Rgb::DEFAULT_GRADIENT_END,
    };
    pub const DEFAULT_HUE_CYCLE: Self = Self::HueCycle {
        initial: Rgb::DEFAULT_HUE_CYCLE_INITIAL,
    };
    pub const DEFAULT_DEPTH_GRADIENT: Self = Self::DepthGradient {
        start: Rgb::DEFAULT_GRADIENT_START,
        end: Rgb::DEFAULT_GRADIENT_END,
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
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ColorConfig {
    pub background: Option<Rgb>,
    pub line: LineColorConfig,
}

impl ColorConfig {
    pub const DEFAULT_BACKGROUND: Rgb = Rgb::BLACK;

    pub fn effective_background(&self) -> Rgb {
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
        self.colors.clone()
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
    #[serde(default, deserialize_with = "deserialize_optional_rgb")]
    background: Option<[f64; 3]>,
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
    #[serde(deserialize_with = "deserialize_rgb")]
    color: [f64; 3],
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGradientColors {
    #[serde(deserialize_with = "deserialize_rgb")]
    start: [f64; 3],
    #[serde(deserialize_with = "deserialize_rgb")]
    end: [f64; 3],
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawHueCycleColor {
    #[serde(deserialize_with = "deserialize_rgb")]
    initial: [f64; 3],
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
                    .map(|color| validate_color_components(color, "colors.background"))
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
                color: validate_color_components(raw.color, "colors.line.color")?,
            },
            RawLineColor::Gradient(raw) => Self::Gradient {
                start: validate_color_components(raw.start, "colors.line.start")?,
                end: validate_color_components(raw.end, "colors.line.end")?,
            },
            RawLineColor::HueCycle(raw) => Self::HueCycle {
                initial: validate_color_components(raw.initial, "colors.line.initial")?,
            },
            RawLineColor::DepthGradient(raw) => Self::DepthGradient {
                start: validate_color_components(raw.start, "colors.line.start")?,
                end: validate_color_components(raw.end, "colors.line.end")?,
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

fn deserialize_rgb<'de, D>(deserializer: D) -> Result<[f64; 3], D::Error>
where
    D: Deserializer<'de>,
{
    let values = <[TomlNumber; 3]>::deserialize(deserializer)?;
    Ok(values.map(|value| value.0))
}

fn deserialize_optional_rgb<'de, D>(deserializer: D) -> Result<Option<[f64; 3]>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Option::<[TomlNumber; 3]>::deserialize(deserializer)?
        .map(|values| values.map(|value| value.0)))
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

fn validate_color_components(color: [f64; 3], field: &str) -> Result<Rgb, ConfigError> {
    let color = color.map(|component| component as f32);
    if let Some((component, value)) = invalid_rgb_component(color) {
        return Err(ConfigError::InvalidColorComponent {
            field: field.to_string(),
            component,
            value,
        });
    }
    Ok(Rgb::from_array_unchecked(color))
}

fn invalid_rgb_component(color: [f32; 3]) -> Option<(usize, f32)> {
    color
        .into_iter()
        .enumerate()
        .find(|(_, value)| !value.is_finite() || !(0.0..=1.0).contains(value))
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

    pub fn set_background(&mut self, background: Option<Rgb>) {
        match background {
            Some(background) => {
                set_color_value_preserving_decor(
                    &mut self.document["colors"]["background"],
                    background,
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
                set_color_value_preserving_decor(&mut line["color"], *color);
            }
            LineColorConfig::Gradient { start, end } => {
                remove_inactive_line_color_keys(line, &["start", "end"]);
                set_color_value_preserving_decor(&mut line["start"], *start);
                set_color_value_preserving_decor(&mut line["end"], *end);
            }
            LineColorConfig::HueCycle { initial } => {
                remove_inactive_line_color_keys(line, &["initial"]);
                set_color_value_preserving_decor(&mut line["initial"], *initial);
            }
            LineColorConfig::DepthGradient { start, end } => {
                remove_inactive_line_color_keys(line, &["start", "end"]);
                set_color_value_preserving_decor(&mut line["start"], *start);
                set_color_value_preserving_decor(&mut line["end"], *end);
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

fn set_color_value_preserving_decor(item: &mut Item, color: Rgb) {
    let can_update_components = item.as_array().is_some_and(|array| {
        array.len() == 3 && array.iter().all(|value| value_as_f32(value).is_some())
    });

    if can_update_components {
        let array = item
            .as_array_mut()
            .expect("array shape was checked before mutation");
        for (idx, component) in color.to_array().into_iter().enumerate() {
            array.replace(idx, color_component_value(component));
        }
    } else {
        set_value_preserving_decor(item, color_value(color));
    }
}

fn color_value(color: Rgb) -> Value {
    color
        .to_array()
        .into_iter()
        .map(color_component_value)
        .collect()
}

fn color_component_value(component: f32) -> Value {
    let raw = component.to_string();
    raw.parse()
        .expect("validated RGB component display output must parse as a TOML number")
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

fn value_as_f32(value: &Value) -> Option<f32> {
    value
        .as_float()
        .or_else(|| value.as_integer().map(|value| value as f64))
        .map(|value| value as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn parse_config(toml_str: &str) -> Result<Config, ConfigError> {
        Ok(ConfigDocument::try_from(ConfigSource::parse(toml_str)?)?.into())
    }

    fn rgb(components: [f32; 3]) -> Rgb {
        Rgb::try_from(components).unwrap()
    }

    #[test]
    fn rgb_constructs_valid_color() {
        let rgb = rgb([0.1, 0.2, 0.3]);

        assert_eq!(rgb.red(), 0.1);
        assert_eq!(rgb.green(), 0.2);
        assert_eq!(rgb.blue(), 0.3);
        assert_eq!(rgb.to_array(), [0.1, 0.2, 0.3]);
    }

    #[test]
    fn rgb_rejects_non_finite_and_out_of_range_components() {
        for color in [
            [f32::NAN, 0.0, 0.0],
            [0.0, f32::INFINITY, 0.0],
            [0.0, 0.0, f32::NEG_INFINITY],
            [-0.1, 0.0, 0.0],
            [0.0, 1.1, 0.0],
        ] {
            assert_eq!(Rgb::try_from(color), Err(RgbError), "{color:?} should fail");
        }
    }

    #[test]
    fn rgb_accepts_boundary_values() {
        assert_eq!(Rgb::try_from([0.0, 0.0, 0.0]), Ok(Rgb::BLACK));
        assert_eq!(Rgb::try_from([1.0, 1.0, 1.0]), Ok(rgb([1.0, 1.0, 1.0])));
    }

    #[test]
    fn rgb_try_from_array_uses_validated_constructor() {
        assert_eq!(Rgb::try_from([0.1, 0.2, 0.3]), Ok(rgb([0.1, 0.2, 0.3])));
        assert_eq!(Rgb::try_from([0.1, 1.2, 0.3]), Err(RgbError));
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

    const NESTED_KOCH_TOML: &str = r#"
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
background = [0.0, 0.0, 0.0]

[colors.line]
mode = "hue_cycle"
initial = [0.25, 0.5, 0.5]
"#;

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
            r#"[metadata]
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
background = [0.0, 0.0, 0.0]

[colors.line]
mode = "solid"
color = [0.0, 0.9, 0.5]
"#
        )
    }

    #[test]
    fn config_document_preserves_unmodified_toml_byte_for_byte() {
        let original = r#"# leading comment
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
background = [ 0.0, 0.1, 0.2 ]

[colors.line]
mode = "gradient"
start = [
    0.1,
    0.2,
    0.3,
]
end = [ 0.7, 0.8, 0.9 ]
"#;

        let doc = ConfigSource::parse(original).unwrap();

        assert_eq!(doc.to_string(), original);
        assert!(ConfigDocument::try_from(doc).is_ok());
    }

    #[test]
    fn set_name_preserves_existing_value_comment() {
        let original = r#"[metadata]
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
background = [0.0, 0.0, 0.0]

[colors.line]
mode = "solid"
color = [0.0, 0.9, 0.5]
"#;
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
        assert_eq!(cfg.colors.background, Some(rgb([0.0, 0.0, 0.0])));
        match cfg.colors.line {
            LineColorConfig::HueCycle { initial } => assert_eq!(initial, rgb([0.25, 0.5, 0.5])),
            other => panic!("expected hue cycle line color, got {other:?}"),
        }
    }

    #[test]
    fn parses_missing_background_as_none() {
        let toml = NESTED_KOCH_TOML.replace("background = [0.0, 0.0, 0.0]\n\n", "");
        let cfg = parse_config(&toml).unwrap();

        assert_eq!(cfg.colors.background, None);
    }

    #[test]
    fn parses_present_background_as_some_rgb() {
        let toml = NESTED_KOCH_TOML.replace(
            "background = [0.0, 0.0, 0.0]",
            "background = [0.2, 0.3, 0.4]",
        );
        let cfg = parse_config(&toml).unwrap();

        assert_eq!(
            cfg.colors.background.map(Rgb::to_array),
            Some([0.2, 0.3, 0.4])
        );
    }

    #[test]
    fn accepts_integer_color_components() {
        let toml = NESTED_KOCH_TOML
            .replace("background = [0.0, 0.0, 0.0]", "background = [0, 0, 1]")
            .replace("initial = [0.25, 0.5, 0.5]", "initial = [1, 0, 0]");
        let cfg = parse_config(&toml).unwrap();

        assert_eq!(
            cfg.colors.background.map(Rgb::to_array),
            Some([0.0, 0.0, 1.0])
        );
        assert_eq!(
            cfg.colors.line,
            LineColorConfig::HueCycle {
                initial: rgb([1.0, 0.0, 0.0]),
            }
        );
    }

    #[test]
    fn rejects_wrong_length_color_arrays() {
        for (fragments, source, replacement) in [
            (
                &["colors.background", "invalid length"][..],
                "background = [0.0, 0.0, 0.0]",
                "background = [0.0, 0.0]",
            ),
            (
                &["colors.line", "invalid length"][..],
                "initial = [0.25, 0.5, 0.5]",
                "initial = [0.25, 0.5, 0.5, 0.75]",
            ),
        ] {
            let toml = NESTED_KOCH_TOML.replace(source, replacement);
            let err = parse_config(&toml).unwrap_err();

            assert_toml_deserialize_error_contains(err, fragments);
        }
    }

    #[test]
    fn line_color_config_exposes_mode_defaults() {
        assert_eq!(
            LineColorConfig::DEFAULT_SOLID,
            LineColorConfig::Solid {
                color: rgb([0.0, 0.9, 0.5]),
            }
        );
        assert_eq!(LineColorConfig::default(), LineColorConfig::DEFAULT_SOLID);
        assert_eq!(
            LineColorConfig::DEFAULT_GRADIENT,
            LineColorConfig::Gradient {
                start: rgb([0.05, 0.35, 0.05]),
                end: rgb([0.6, 0.9, 0.1]),
            }
        );
        assert_eq!(
            LineColorConfig::DEFAULT_HUE_CYCLE,
            LineColorConfig::HueCycle {
                initial: rgb([0.9, 0.0, 0.0]),
            }
        );
        assert_eq!(
            LineColorConfig::DEFAULT_DEPTH_GRADIENT,
            LineColorConfig::DepthGradient {
                start: rgb([0.05, 0.35, 0.05]),
                end: rgb([0.6, 0.9, 0.1]),
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
                color: rgb([0.0, 0.9, 0.5]),
            }
        );
    }

    #[test]
    fn parses_gradient_line_color() {
        let toml = test_toml(Dimensions::TwoD, "F", 1, "90.0", "1.0", "0.0", "").replace(
            r#"mode = "solid"
color = [0.0, 0.9, 0.5]"#,
            r#"mode = "gradient"
start = [0.1, 0.2, 0.3]
end = [0.7, 0.8, 0.9]"#,
        );

        let cfg = parse_config(&toml).unwrap();

        assert_eq!(
            cfg.colors.line,
            LineColorConfig::Gradient {
                start: rgb([0.1, 0.2, 0.3]),
                end: rgb([0.7, 0.8, 0.9]),
            }
        );
    }

    #[test]
    fn parses_depth_gradient_line_color() {
        let toml = test_toml(Dimensions::TwoD, "F", 1, "90.0", "1.0", "0.0", "").replace(
            r#"mode = "solid"
color = [0.0, 0.9, 0.5]"#,
            r#"mode = "depth_gradient"
start = [0.1, 0.2, 0.3]
end = [0.7, 0.8, 0.9]"#,
        );

        let cfg = parse_config(&toml).unwrap();

        assert_eq!(
            cfg.colors.line,
            LineColorConfig::DepthGradient {
                start: rgb([0.1, 0.2, 0.3]),
                end: rgb([0.7, 0.8, 0.9]),
            }
        );
    }

    #[test]
    fn rejects_missing_depth_gradient_keys() {
        let cases = [
            (
                r#"mode = "depth_gradient"
start = [0.1, 0.2, 0.3]"#,
                "colors.line.end",
            ),
            (
                r#"mode = "depth_gradient"
end = [0.7, 0.8, 0.9]"#,
                "colors.line.start",
            ),
        ];

        for (replacement, missing_field) in cases {
            let toml = test_toml(Dimensions::TwoD, "F", 1, "90.0", "1.0", "0.0", "").replace(
                r#"mode = "solid"
color = [0.0, 0.9, 0.5]"#,
                replacement,
            );

            let err = parse_config(&toml).unwrap_err();

            assert_toml_deserialize_error_mentions_path(err, missing_field);
        }
    }

    #[test]
    fn parses_dotted_v2_config() {
        let toml = r#"
metadata.name = "Dotted"
l-system.dimensions = "3D"
l-system.axiom = 'F\F'
l-system.iterations = 2
l-system.rules.F = 'F\F'
turtle.angle = 45.0
turtle.step = 1.0
turtle.initial_heading = 0.0
colors.background = [0.0, 0.0, 0.0]
colors.line.mode = "solid"
colors.line.color = [0.0, 0.9, 0.5]
"#;

        let cfg = parse_config(toml).unwrap();

        assert_eq!(cfg.name, "Dotted");
        assert_eq!(cfg.generation.dimensions, Dimensions::ThreeD);
        assert_eq!(cfg.generation.axiom, "F\\F");
        assert_eq!(cfg.generation.rules[&'F'], "F\\F");
    }

    #[test]
    fn parses_implicit_parent_tables() {
        let toml = r#"
colors.background = [0.0, 0.0, 0.0]

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
color = [0.0, 0.9, 0.5]
"#;

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
        let toml = r#"# preset comment
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
background = [0.0, 0.0, 0.0]

[colors.line]
mode = "solid"
color = [0.1, 0.2, 0.3]
"#;

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
        let toml = r#"
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
background = [0.0, 0.0, 0.0]

[colors.line]
mode = "solid"
color = [0.0, 0.9, 0.5]
"#;
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
        let toml = r#"
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
background = [0.0, 0.0, 0.0]

[colors.line]
mode = "solid"
color = [0.0, 0.9, 0.5]
"#;
        assert!(parse_config(toml).is_err());
    }

    #[test]
    fn rejects_out_of_range_color_component() {
        let toml = test_toml(Dimensions::TwoD, "F", 1, "90.0", "1.0", "0.0", "");
        let toml = toml.replace(
            "background = [0.0, 0.0, 0.0]",
            "background = [0.0, 1.2, 0.0]",
        );
        let err = parse_config(&toml).unwrap_err();
        assert!(
            matches!(
                err,
                ConfigError::InvalidColorComponent {
                    ref field,
                    component: 1,
                    value
                } if field == "colors.background" && value == 1.2
            ),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_out_of_range_depth_gradient_color_component() {
        for (field, replacement) in [
            (
                "colors.line.start",
                r#"mode = "depth_gradient"
start = [0.0, 1.2, 0.0]
end = [1.0, 1.0, 1.0]"#,
            ),
            (
                "colors.line.end",
                r#"mode = "depth_gradient"
start = [0.0, 0.0, 0.0]
end = [1.0, 1.2, 1.0]"#,
            ),
        ] {
            let toml = test_toml(Dimensions::TwoD, "F", 1, "90.0", "1.0", "0.0", "").replace(
                r#"mode = "solid"
color = [0.0, 0.9, 0.5]"#,
                replacement,
            );
            let err = parse_config(&toml).unwrap_err();
            assert!(
                matches!(
                    err,
                    ConfigError::InvalidColorComponent {
                        field: ref error_field,
                        component: 1,
                        value
                    } if error_field == field && value == 1.2
                ),
                "unexpected error: {err}"
            );
        }
    }

    #[test]
    fn rejects_non_finite_line_color_component() {
        let toml = test_toml(Dimensions::TwoD, "F", 1, "90.0", "1.0", "0.0", "");
        let toml = toml.replace("color = [0.0, 0.9, 0.5]", "color = [0.0, nan, 0.5]");
        let err = parse_config(&toml).unwrap_err();
        assert!(
            matches!(
                err,
                ConfigError::InvalidColorComponent {
                    ref field,
                    component: 1,
                    ..
                } if field == "colors.line.color"
            ),
            "unexpected error: {err}"
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
                r#"mode = "solid"
color = [0.0, 0.9, 0.5]
start = [0.0, 0.0, 0.0]"#,
                "colors.line.start",
            ),
            (
                "gradient",
                r#"mode = "gradient"
start = [0.0, 0.0, 0.0]
end = [1.0, 1.0, 1.0]
color = [0.0, 0.9, 0.5]"#,
                "colors.line.color",
            ),
            (
                "depth_gradient",
                r#"mode = "depth_gradient"
start = [0.0, 0.0, 0.0]
end = [1.0, 1.0, 1.0]
color = [0.0, 0.9, 0.5]"#,
                "colors.line.color",
            ),
            (
                "hue_cycle",
                r#"mode = "hue_cycle"
initial = [0.0, 0.9, 0.5]
end = [1.0, 1.0, 1.0]"#,
                "colors.line.end",
            ),
        ];

        for (_mode, replacement, expected_field) in cases {
            let toml = test_toml(Dimensions::TwoD, "F", 1, "90.0", "1.0", "0.0", "");
            let toml = toml.replace("mode = \"solid\"\ncolor = [0.0, 0.9, 0.5]", replacement);
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
        let start = rgb([0.1, 0.2, 0.3]);
        let end = rgb([0.7, 0.8, 0.9]);
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
        let start = rgb([0.1, 0.2, 0.3]);
        let end = rgb([0.7, 0.8, 0.9]);
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
            start: rgb([1.0, 0.0, 0.0]),
            end: rgb([0.0, 0.0, 1.0]),
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
            initial: rgb([1.0, 0.0, 0.0]),
        };
        let effective = cfg.effective_colors();
        assert_eq!(effective.line, cfg.colors.line);
        assert_eq!(effective.background, cfg.colors.background);
    }
}
