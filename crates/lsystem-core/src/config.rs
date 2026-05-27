use std::collections::BTreeMap;

use thiserror::Error;
use toml_edit::{DocumentMut, Item, Table, Value, value};

use crate::alphabet::{validate_bracket_balance, validate_symbols};

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("TOML parse error: {0}")]
    TomlParse(#[from] toml_edit::TomlError),

    #[error("missing required field `{0}`")]
    MissingField(String),

    #[error("invalid value for `{field}`: expected {expected}")]
    InvalidField {
        field: String,
        expected: &'static str,
    },

    #[error("unknown field `{0}`")]
    UnknownField(String),

    #[error("rule key {key:?} must be a single ASCII letter")]
    InvalidRuleKey { key: String },

    #[error("invalid symbol {ch:?} at position {position} in `{field}`")]
    InvalidSymbol {
        ch: char,
        field: String,
        position: usize,
    },

    #[error("unsupported dimensions value {0} (must be 2 or 3)")]
    InvalidDimensions(i64),

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

/// Color mode for the fractal lines.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LineColorConfig {
    Solid { color: [f32; 3] },
    Gradient { start: [f32; 3], end: [f32; 3] },
    HueCycle { initial: [f32; 3] },
    DepthGradient { start: [f32; 3], end: [f32; 3] },
}

impl LineColorConfig {
    pub const DEFAULT_SOLID: Self = Self::Solid {
        color: [0.0, 0.9, 0.5],
    };
    pub const DEFAULT_GRADIENT: Self = Self::Gradient {
        start: [0.05, 0.35, 0.05],
        end: [0.6, 0.9, 0.1],
    };
    pub const DEFAULT_HUE_CYCLE: Self = Self::HueCycle {
        initial: [0.9, 0.0, 0.0],
    };
    pub const DEFAULT_DEPTH_GRADIENT: Self = Self::DepthGradient {
        start: [0.05, 0.35, 0.05],
        end: [0.6, 0.9, 0.1],
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
    pub background: Option<[f32; 3]>,
    pub line: LineColorConfig,
}

impl ColorConfig {
    pub const DEFAULT_BACKGROUND: [f32; 3] = [0.0, 0.0, 0.0];

    pub fn effective_background(&self) -> [f32; 3] {
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

    fn to_config(&self) -> Result<Config, ConfigError> {
        validate_schema(&self.document)?;

        let metadata = required_table(&self.document, "metadata")?;
        let l_system = required_table(&self.document, "l-system")?;
        let rules_table = required_table(l_system, "rules")?;
        let turtle = required_table(&self.document, "turtle")?;
        let colors_table = required_table(&self.document, "colors")?;
        let line_table = required_table(colors_table, "line")?;

        let dimensions = required_dimensions(l_system, "l-system.dimensions")?;
        let step = required_f32(turtle, "turtle.step")?;
        let angle = required_f32(turtle, "turtle.angle")?;
        let initial_heading = required_f32(turtle, "turtle.initial_heading")?;
        if !step.is_finite() || step <= 0.0 {
            return Err(ConfigError::InvalidStep(step));
        }
        if !angle.is_finite() {
            return Err(ConfigError::InvalidAngle(angle));
        }
        if !initial_heading.is_finite() {
            return Err(ConfigError::InvalidInitialHeading(initial_heading));
        }

        // Strip whitespace from axiom and rule RHS, then validate symbols.
        let axiom_raw = required_str(l_system, "l-system.axiom")?;
        let axiom: String = axiom_raw.chars().filter(|c| !c.is_whitespace()).collect();
        validate_symbols(&axiom, "axiom", dimensions)?;
        validate_bracket_balance(&axiom, "axiom")?;

        let mut rules = BTreeMap::new();
        for (key_str, item) in rules_table.iter() {
            let mut key_chars = key_str.chars();
            let key = key_chars
                .next()
                .filter(|c| c.is_ascii_alphabetic())
                .ok_or_else(|| ConfigError::InvalidRuleKey {
                    key: key_str.to_string(),
                })?;
            if key_chars.next().is_some() {
                return Err(ConfigError::InvalidRuleKey {
                    key: key_str.to_string(),
                });
            }

            let rhs_raw = item.as_str().ok_or_else(|| ConfigError::InvalidField {
                field: format!("l-system.rules.{key}"),
                expected: "string",
            })?;
            let rhs: String = rhs_raw.chars().filter(|c| !c.is_whitespace()).collect();
            validate_symbols(&rhs, &format!("l-system.rules.{key}"), dimensions)?;
            validate_bracket_balance(&rhs, &format!("l-system.rules.{key}"))?;
            rules.insert(key, rhs);
        }

        let background = optional_color(colors_table, "colors.background")?;
        if let Some(background) = background {
            validate_color(background, "colors.background")?;
        }

        let mode = required_str(line_table, "colors.line.mode")?;
        let line = match mode {
            "solid" => {
                validate_keys(line_table, "colors.line", &["mode", "color"])?;
                let color = required_color(line_table, "colors.line.color")?;
                validate_color(color, "colors.line.color")?;
                LineColorConfig::Solid { color }
            }
            "gradient" => {
                validate_keys(line_table, "colors.line", &["mode", "start", "end"])?;
                let start = required_color(line_table, "colors.line.start")?;
                let end = required_color(line_table, "colors.line.end")?;
                validate_color(start, "colors.line.start")?;
                validate_color(end, "colors.line.end")?;
                LineColorConfig::Gradient { start, end }
            }
            "hue_cycle" => {
                validate_keys(line_table, "colors.line", &["mode", "initial"])?;
                let initial = required_color(line_table, "colors.line.initial")?;
                validate_color(initial, "colors.line.initial")?;
                LineColorConfig::HueCycle { initial }
            }
            "depth_gradient" => {
                validate_keys(line_table, "colors.line", &["mode", "start", "end"])?;
                let start = required_color(line_table, "colors.line.start")?;
                let end = required_color(line_table, "colors.line.end")?;
                validate_color(start, "colors.line.start")?;
                validate_color(end, "colors.line.end")?;
                LineColorConfig::DepthGradient { start, end }
            }
            _ => {
                return Err(ConfigError::InvalidField {
                    field: "colors.line.mode".to_string(),
                    expected: "\"solid\", \"gradient\", \"hue_cycle\", or \"depth_gradient\"",
                });
            }
        };

        let colors = ColorConfig { background, line };

        Ok(Config {
            name: required_str(metadata, "metadata.name")?.to_string(),
            generation: GenerationConfig {
                dimensions,
                axiom,
                iterations: required_u32(l_system, "l-system.iterations")?,
                angle,
                step,
                initial_heading,
                rules,
            },
            colors,
        })
    }

    pub fn to_toml_string(&self) -> String {
        self.document.to_string()
    }

    pub(crate) fn set_name(&mut self, name: &str) {
        if self
            .document
            .get("metadata")
            .is_none_or(|item| !item.is_table())
        {
            self.document["metadata"] = Item::Table(Table::new());
        }
        self.document["metadata"]["name"] = value(name);
    }

    pub(crate) fn set_iterations(&mut self, iterations: u32) {
        set_value_preserving_decor(
            &mut self.document["l-system"]["iterations"],
            Value::from(i64::from(iterations)),
        );
    }

    pub(crate) fn set_angle(&mut self, angle: f32) {
        set_value_preserving_decor(
            &mut self.document["turtle"]["angle"],
            Value::from(f64::from(angle)),
        );
    }

    pub(crate) fn set_background(&mut self, background: Option<[f32; 3]>) {
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

    pub(crate) fn set_line_color(&mut self, line_color: &LineColorConfig) {
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
        let config = source.to_config()?;
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

fn validate_schema(document: &DocumentMut) -> Result<(), ConfigError> {
    validate_keys(document, "", &["metadata", "l-system", "turtle", "colors"])?;
    validate_keys(required_table(document, "metadata")?, "metadata", &["name"])?;
    validate_keys(
        required_table(document, "l-system")?,
        "l-system",
        &["dimensions", "axiom", "iterations", "rules"],
    )?;
    validate_keys(
        required_table(document, "turtle")?,
        "turtle",
        &["angle", "step", "initial_heading"],
    )?;
    validate_keys(
        required_table(document, "colors")?,
        "colors",
        &["background", "line"],
    )?;
    Ok(())
}

trait TableLike {
    fn get_item(&self, key: &str) -> Option<&Item>;
    fn keys<'a>(&'a self) -> Box<dyn Iterator<Item = &'a str> + 'a>;
}

impl TableLike for DocumentMut {
    fn get_item(&self, key: &str) -> Option<&Item> {
        self.get(key)
    }

    fn keys<'a>(&'a self) -> Box<dyn Iterator<Item = &'a str> + 'a> {
        Box::new(self.iter().map(|(key, _)| key))
    }
}

impl TableLike for Table {
    fn get_item(&self, key: &str) -> Option<&Item> {
        self.get(key)
    }

    fn keys<'a>(&'a self) -> Box<dyn Iterator<Item = &'a str> + 'a> {
        Box::new(self.iter().map(|(key, _)| key))
    }
}

fn validate_keys(table: &impl TableLike, path: &str, allowed: &[&str]) -> Result<(), ConfigError> {
    for key in table.keys() {
        if !allowed.contains(&key) {
            let field = if path.is_empty() {
                key.to_string()
            } else {
                format!("{path}.{key}")
            };
            return Err(ConfigError::UnknownField(field));
        }
    }
    Ok(())
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

fn set_color_value_preserving_decor(item: &mut Item, color: [f32; 3]) {
    let can_update_components = item.as_array().is_some_and(|array| {
        array.len() == 3 && array.iter().all(|value| value_as_f32(value).is_some())
    });

    if can_update_components {
        let array = item
            .as_array_mut()
            .expect("array shape was checked before mutation");
        for (idx, component) in color.into_iter().enumerate() {
            array.replace(idx, color_component_value(component));
        }
    } else {
        set_value_preserving_decor(item, color_value(color));
    }
}

fn color_value(color: [f32; 3]) -> Value {
    color.into_iter().map(color_component_value).collect()
}

fn color_component_value(component: f32) -> Value {
    let raw = if component.is_nan() {
        "nan".to_string()
    } else if component == f32::INFINITY {
        "inf".to_string()
    } else if component == f32::NEG_INFINITY {
        "-inf".to_string()
    } else {
        component.to_string()
    };
    raw.parse()
        .unwrap_or_else(|_| Value::from(f64::from(component)))
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

fn required_item<'a>(table: &'a impl TableLike, field: &str) -> Result<&'a Item, ConfigError> {
    let key = field.rsplit('.').next().unwrap_or(field);
    table
        .get_item(key)
        .ok_or_else(|| ConfigError::MissingField(field.to_string()))
}

fn required_table<'a>(table: &'a impl TableLike, field: &str) -> Result<&'a Table, ConfigError> {
    let table =
        required_item(table, field)?
            .as_table()
            .ok_or_else(|| ConfigError::InvalidField {
                field: field.to_string(),
                expected: "table",
            })?;
    Ok(table)
}

fn required_str<'a>(table: &'a impl TableLike, field: &str) -> Result<&'a str, ConfigError> {
    required_item(table, field)?
        .as_str()
        .ok_or_else(|| ConfigError::InvalidField {
            field: field.to_string(),
            expected: "string",
        })
}

fn required_dimensions(table: &impl TableLike, field: &str) -> Result<Dimensions, ConfigError> {
    let value =
        required_item(table, field)?
            .as_integer()
            .ok_or_else(|| ConfigError::InvalidField {
                field: field.to_string(),
                expected: "integer",
            })?;
    match value {
        2 => Ok(Dimensions::TwoD),
        3 => Ok(Dimensions::ThreeD),
        other => Err(ConfigError::InvalidDimensions(other)),
    }
}

fn required_u32(table: &impl TableLike, field: &str) -> Result<u32, ConfigError> {
    let value =
        required_item(table, field)?
            .as_integer()
            .ok_or_else(|| ConfigError::InvalidField {
                field: field.to_string(),
                expected: "integer",
            })?;
    u32::try_from(value).map_err(|_| ConfigError::InvalidField {
        field: field.to_string(),
        expected: "integer in 0..=4294967295",
    })
}

fn required_f32(table: &impl TableLike, field: &str) -> Result<f32, ConfigError> {
    let item = required_item(table, field)?;
    let value = if let Some(value) = item.as_float() {
        value
    } else if let Some(value) = item.as_integer() {
        value as f64
    } else {
        return Err(ConfigError::InvalidField {
            field: field.to_string(),
            expected: "number",
        });
    };
    Ok(value as f32)
}

fn required_color(table: &impl TableLike, field: &str) -> Result<[f32; 3], ConfigError> {
    let array =
        required_item(table, field)?
            .as_array()
            .ok_or_else(|| ConfigError::InvalidField {
                field: field.to_string(),
                expected: "array of three numbers",
            })?;

    if array.len() != 3 {
        return Err(ConfigError::InvalidField {
            field: field.to_string(),
            expected: "array of three numbers",
        });
    }

    let mut out = [0.0; 3];
    for (idx, value) in array.iter().enumerate() {
        out[idx] = value_as_f32(value).ok_or_else(|| ConfigError::InvalidField {
            field: format!("{field}[{idx}]"),
            expected: "number",
        })?;
    }
    Ok(out)
}

fn optional_color(table: &impl TableLike, field: &str) -> Result<Option<[f32; 3]>, ConfigError> {
    match required_color(table, field) {
        Ok(color) => Ok(Some(color)),
        Err(ConfigError::MissingField(_)) => Ok(None),
        Err(err) => Err(err),
    }
}

fn value_as_f32(value: &Value) -> Option<f32> {
    value
        .as_float()
        .or_else(|| value.as_integer().map(|value| value as f64))
        .map(|value| value as f32)
}

fn validate_color(color: [f32; 3], field: &str) -> Result<(), ConfigError> {
    for (component, value) in color.into_iter().enumerate() {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(ConfigError::InvalidColorComponent {
                field: field.to_string(),
                component,
                value,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn parse_config(toml_str: &str) -> Result<Config, ConfigError> {
        Ok(ConfigDocument::try_from(ConfigSource::parse(toml_str)?)?.into())
    }

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

    const NESTED_KOCH_TOML: &str = r#"
[metadata]
name = "Koch Snowflake"

[l-system]
dimensions = 2
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
        dimensions: i64,
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
dimensions = 3
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
        assert_eq!(cfg.colors.background, Some([0.0, 0.0, 0.0]));
        match cfg.colors.line {
            LineColorConfig::HueCycle { initial } => assert_eq!(initial, [0.25, 0.5, 0.5]),
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

        assert_eq!(cfg.colors.background, Some([0.2, 0.3, 0.4]));
    }

    #[test]
    fn line_color_config_exposes_mode_defaults() {
        assert_eq!(
            LineColorConfig::DEFAULT_SOLID,
            LineColorConfig::Solid {
                color: [0.0, 0.9, 0.5],
            }
        );
        assert_eq!(LineColorConfig::default(), LineColorConfig::DEFAULT_SOLID);
        assert_eq!(
            LineColorConfig::DEFAULT_GRADIENT,
            LineColorConfig::Gradient {
                start: [0.05, 0.35, 0.05],
                end: [0.6, 0.9, 0.1],
            }
        );
        assert_eq!(
            LineColorConfig::DEFAULT_HUE_CYCLE,
            LineColorConfig::HueCycle {
                initial: [0.9, 0.0, 0.0],
            }
        );
        assert_eq!(
            LineColorConfig::DEFAULT_DEPTH_GRADIENT,
            LineColorConfig::DepthGradient {
                start: [0.05, 0.35, 0.05],
                end: [0.6, 0.9, 0.1],
            }
        );
    }

    #[test]
    fn parses_depth_gradient_line_color() {
        let toml = test_toml(2, "F", 1, "90.0", "1.0", "0.0", "").replace(
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
                start: [0.1, 0.2, 0.3],
                end: [0.7, 0.8, 0.9],
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
            let toml = test_toml(2, "F", 1, "90.0", "1.0", "0.0", "").replace(
                r#"mode = "solid"
color = [0.0, 0.9, 0.5]"#,
                replacement,
            );

            let err = parse_config(&toml).unwrap_err();

            assert!(
                matches!(
                    err,
                    ConfigError::MissingField(ref field) if field == missing_field
                ),
                "unexpected error: {err}"
            );
        }
    }

    #[test]
    fn parses_dotted_v2_config() {
        let toml = r#"
metadata.name = "Dotted"
l-system.dimensions = 3
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
dimensions = 2
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
dimensions = 3
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
    fn rejects_missing_step() {
        let toml = r#"
[metadata]
name = "test"

[l-system]
dimensions = 2
axiom = "F"
iterations = 1

[turtle]
angle = 90.0

[colors]
background = [0.0, 0.0, 0.0]

[colors.line]
mode = "solid"
color = [0.0, 0.9, 0.5]
"#;
        assert!(parse_config(toml).is_err());
    }

    #[test]
    fn rejects_missing_rules_table() {
        let toml = r#"
[metadata]
name = "test"

[l-system]
dimensions = 2
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
        let toml = test_toml(2, "F", 1, "90.0", "1.0", "0.0", "");
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
            let toml = test_toml(2, "F", 1, "90.0", "1.0", "0.0", "").replace(
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
        let toml = test_toml(2, "F", 1, "90.0", "1.0", "0.0", "");
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
        let toml = test_toml(2, "F", 1, "90.0", "1.0", "0.0", "");
        let toml = toml.replace("mode = \"solid\"", "mode = \"rainbow\"");
        let err = parse_config(&toml).unwrap_err();

        assert!(
            matches!(
                err,
                ConfigError::InvalidField { ref field, .. } if field == "colors.line.mode"
            ),
            "unexpected error: {err}"
        );
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

        for (mode, replacement, expected_field) in cases {
            let toml = test_toml(2, "F", 1, "90.0", "1.0", "0.0", "");
            let toml = toml.replace("mode = \"solid\"\ncolor = [0.0, 0.9, 0.5]", replacement);
            let err = parse_config(&toml).unwrap_err();

            assert!(
                matches!(
                    err,
                    ConfigError::UnknownField(ref field) if field == expected_field
                ),
                "mode={mode}: unexpected error: {err}"
            );
        }
    }

    #[test]
    fn rejects_unknown_schema_fields() {
        let toml = format!("{}\n[experimental]\nfoo = true\n", NESTED_KOCH_TOML);
        let err = parse_config(&toml).unwrap_err();

        assert!(
            matches!(err, ConfigError::UnknownField(ref field) if field == "experimental"),
            "unexpected error: {err}"
        );

        let toml = NESTED_KOCH_TOML.replace(
            "initial_heading = 0.0",
            "initial_heading = 0.0\nfriction = 0.5",
        );
        let err = parse_config(&toml).unwrap_err();

        assert!(
            matches!(err, ConfigError::UnknownField(ref field) if field == "turtle.friction"),
            "unexpected error: {err}"
        );
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
        let toml = test_toml(2, "F + + F", 1, "90.0", "1.0", "0.0", r#"F = "F - F""#);
        let cfg = parse_config(&toml).unwrap();
        assert_eq!(cfg.generation.axiom, "F++F");
        assert_eq!(cfg.generation.rules[&'F'], "F-F");
    }

    #[test]
    fn rejects_digit_in_axiom() {
        let toml = test_toml(2, "F+1", 1, "90.0", "1.0", "0.0", "");
        let err = parse_config(&toml).unwrap_err();
        assert!(
            matches!(err, ConfigError::InvalidSymbol { ch: '1', .. }),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_multi_char_rule_key() {
        let toml = test_toml(2, "F", 1, "90.0", "1.0", "0.0", r#"FF = "FFF""#);
        let err = parse_config(&toml).unwrap_err();
        assert!(
            matches!(err, ConfigError::InvalidRuleKey { .. }),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_invalid_dimensions() {
        for bad_dim in [1, 4, 300] {
            let toml = test_toml(bad_dim, "F", 1, "90.0", "1.0", "0.0", "");
            let err = parse_config(&toml).unwrap_err();
            assert!(
                matches!(err, ConfigError::InvalidDimensions(d) if d == bad_dim),
                "dim={bad_dim}: unexpected error: {err}"
            );
        }
    }

    #[test]
    fn rejects_non_finite_initial_heading() {
        let toml = test_toml(2, "F", 1, "90.0", "1.0", "nan", "");
        let err = parse_config(&toml).unwrap_err();

        assert!(
            matches!(err, ConfigError::InvalidInitialHeading(_)),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn accepts_dimensions_3_with_3d_symbols() {
        let toml = test_toml(3, "F&F^F/F", 0, "90.0", "1.0", "0.0", "");
        parse_config(&toml).expect("3D config with 3D symbols should be valid");
    }

    #[test]
    fn rejects_3d_symbols_in_2d_config() {
        let toml = test_toml(2, "F&F", 1, "90.0", "1.0", "0.0", "");
        let err = parse_config(&toml).unwrap_err();
        assert!(
            matches!(err, ConfigError::InvalidSymbol { ch: '&', .. }),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_unmatched_close_bracket_in_axiom() {
        let toml = test_toml(2, "F]F", 1, "90.0", "1.0", "0.0", "");
        let err = parse_config(&toml).unwrap_err();
        assert!(
            matches!(err, ConfigError::UnmatchedClose { position: 1, .. }),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_unclosed_open_bracket_in_axiom() {
        let toml = test_toml(2, "F[F", 1, "90.0", "1.0", "0.0", "");
        let err = parse_config(&toml).unwrap_err();
        assert!(
            matches!(err, ConfigError::UnmatchedOpen { position: 1, .. }),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn reports_first_unclosed_bracket_not_last() {
        // "F[F[F": two unclosed brackets at positions 1 and 3; error must point to 1.
        let toml = test_toml(2, "F[F[F", 1, "90.0", "1.0", "0.0", "");
        let err = parse_config(&toml).unwrap_err();
        assert!(
            matches!(err, ConfigError::UnmatchedOpen { position: 1, .. }),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_unbalanced_brackets_in_rule() {
        let toml = test_toml(2, "F", 1, "90.0", "1.0", "0.0", r#"F = "F[+F""#);
        let err = parse_config(&toml).unwrap_err();
        assert!(
            matches!(err, ConfigError::UnmatchedOpen { .. }),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_non_positive_step() {
        for bad_step in ["0.0", "-1.0"] {
            let toml = test_toml(2, "F", 1, "90.0", bad_step, "0.0", "");
            let err = parse_config(&toml).unwrap_err();
            assert!(
                matches!(err, ConfigError::InvalidStep(_)),
                "step={bad_step}: unexpected error: {err}"
            );
        }
    }

    #[test]
    fn rejects_non_finite_step() {
        let toml = test_toml(2, "F", 1, "90.0", "inf", "0.0", "");
        let err = parse_config(&toml).unwrap_err();
        assert!(
            matches!(err, ConfigError::InvalidStep(_)),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_non_finite_angle() {
        let toml = test_toml(2, "F", 1, "nan", "1.0", "0.0", "");
        let err = parse_config(&toml).unwrap_err();
        assert!(
            matches!(err, ConfigError::InvalidAngle(_)),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn has_stack_directives_false_for_bracketless() {
        let toml = test_toml(2, "F-F++F-F", 1, "60.0", "1.0", "0.0", "F = \"F-F++F-F\"");
        let cfg = parse_config(&toml).unwrap();
        assert!(!cfg.generation.has_stack_directives());
    }

    #[test]
    fn has_stack_directives_true_when_axiom_has_bracket() {
        let toml = test_toml(2, "F[+F]F", 1, "25.0", "1.0", "0.0", "");
        let cfg = parse_config(&toml).unwrap();
        assert!(cfg.generation.has_stack_directives());
    }

    #[test]
    fn has_stack_directives_true_when_rule_has_bracket() {
        let toml = test_toml(2, "F", 1, "25.0", "1.0", "0.0", "F = \"F[+F]F[-F]F\"");
        let cfg = parse_config(&toml).unwrap();
        assert!(cfg.generation.has_stack_directives());
    }

    #[test]
    fn effective_colors_normalizes_depth_gradient_to_gradient_for_bracketless() {
        let toml = test_toml(2, "F-F++F-F", 1, "60.0", "1.0", "0.0", "");
        let mut cfg = parse_config(&toml).unwrap();
        let start = [0.1, 0.2, 0.3];
        let end = [0.7, 0.8, 0.9];
        cfg.colors.line = LineColorConfig::DepthGradient { start, end };
        let effective = cfg.effective_colors();
        assert_eq!(
            effective.line,
            LineColorConfig::Gradient { start, end },
            "bracketless DepthGradient must normalize to Gradient"
        );
        assert_eq!(cfg.colors.line, LineColorConfig::DepthGradient { start, end },
            "effective_colors must not mutate the stored config");
    }

    #[test]
    fn effective_colors_preserves_depth_gradient_for_bracket_fractal() {
        let toml = test_toml(2, "F", 1, "25.0", "1.0", "0.0", "F = \"F[+F]F[-F]F\"");
        let mut cfg = parse_config(&toml).unwrap();
        let start = [0.1, 0.2, 0.3];
        let end = [0.7, 0.8, 0.9];
        cfg.colors.line = LineColorConfig::DepthGradient { start, end };
        let effective = cfg.effective_colors();
        assert_eq!(
            effective.line,
            LineColorConfig::DepthGradient { start, end },
            "bracket fractal DepthGradient must be preserved"
        );
    }

    #[test]
    fn effective_colors_passes_through_other_modes_for_bracketless() {
        let toml = test_toml(2, "F-F++F-F", 1, "60.0", "1.0", "0.0", "");
        let cfg = parse_config(&toml).unwrap();
        assert_eq!(cfg.effective_colors().line, cfg.colors.line);
    }
}
