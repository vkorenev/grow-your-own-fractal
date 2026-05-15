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
#[derive(Debug, Clone, PartialEq)]
pub enum LineColorConfig {
    Solid { color: [f32; 3] },
    Gradient { start: [f32; 3], end: [f32; 3] },
    HueCycle { initial: [f32; 3] },
}

impl Default for LineColorConfig {
    fn default() -> Self {
        Self::Solid {
            color: [0.0, 0.9, 0.5],
        }
    }
}

/// Visual color settings for background and fractal lines.
#[derive(Debug, Clone, PartialEq)]
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

impl Config {
    pub fn parse(toml_str: &str) -> Result<Self, ConfigError> {
        ConfigDocument::parse(toml_str)?.to_config()
    }

    pub fn to_toml_string(&self) -> String {
        ConfigDocument::from_config(self).to_toml_string()
    }
}

/// Format-preserving TOML document for an L-system configuration.
#[derive(Debug, Clone)]
pub struct ConfigDocument {
    document: DocumentMut,
}

impl ConfigDocument {
    pub fn parse(toml_str: &str) -> Result<Self, ConfigError> {
        Ok(Self {
            document: toml_str.parse()?,
        })
    }

    pub fn to_config(&self) -> Result<Config, ConfigError> {
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

        let background = required_color(colors_table, "colors.background")?;
        validate_color(background, "colors.background")?;

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
            _ => {
                return Err(ConfigError::InvalidField {
                    field: "colors.line.mode".to_string(),
                    expected: "\"solid\", \"gradient\", or \"hue_cycle\"",
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

    pub fn from_config(config: &Config) -> Self {
        let mut document = DocumentMut::new();

        document["metadata"] = Item::Table(Table::new());
        document["metadata"]["name"] = value(config.name.clone());

        document["l-system"] = Item::Table(Table::new());
        document["l-system"]["dimensions"] = value(match config.generation.dimensions {
            Dimensions::TwoD => 2i64,
            Dimensions::ThreeD => 3i64,
        });
        document["l-system"]["axiom"] = literal_string_item(&config.generation.axiom);
        document["l-system"]["iterations"] = value(i64::from(config.generation.iterations));

        document["l-system"]["rules"] = Item::Table(Table::new());
        for (key, rhs) in &config.generation.rules {
            document["l-system"]["rules"][&key.to_string()] = literal_string_item(rhs);
        }

        document["turtle"] = Item::Table(Table::new());
        document["turtle"]["angle"] = value(f64::from(config.generation.angle));
        document["turtle"]["step"] = value(f64::from(config.generation.step));
        document["turtle"]["initial_heading"] = value(f64::from(config.generation.initial_heading));

        document["colors"] = Item::Table(Table::new());
        document["colors"]["background"] = color_item(config.colors.background);

        document["colors"]["line"] = Item::Table(Table::new());
        match &config.colors.line {
            LineColorConfig::Solid { color } => {
                document["colors"]["line"]["mode"] = value("solid");
                document["colors"]["line"]["color"] = color_item(*color);
            }
            LineColorConfig::Gradient { start, end } => {
                document["colors"]["line"]["mode"] = value("gradient");
                document["colors"]["line"]["start"] = color_item(*start);
                document["colors"]["line"]["end"] = color_item(*end);
            }
            LineColorConfig::HueCycle { initial } => {
                document["colors"]["line"]["mode"] = value("hue_cycle");
                document["colors"]["line"]["initial"] = color_item(*initial);
            }
        }

        Self { document }
    }
}

impl std::fmt::Display for ConfigDocument {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_toml_string())
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
    if table.is_implicit() || table.is_dotted() {
        return Err(ConfigError::InvalidField {
            field: field.to_string(),
            expected: "explicit table",
        });
    }
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

fn literal_string_item(value: &str) -> Item {
    if !value.contains('\'') && !value.contains('\n') && !value.contains('\r') {
        return format!("'{value}'")
            .parse::<Item>()
            .expect("validated L-system literal string should parse");
    }
    Value::from(value).into()
}

fn color_item([r, g, b]: [f32; 3]) -> Item {
    let item = format!("[{}, {}, {}]", toml_float(r), toml_float(g), toml_float(b));
    item.parse()
        .expect("validated inline color array should parse")
}

fn toml_float(value: f32) -> String {
    if value.fract() == 0.0 {
        format!("{value:.1}")
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

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

        let doc = ConfigDocument::parse(original).unwrap();

        assert_eq!(doc.to_string(), original);
        assert!(doc.to_config().is_ok());
    }

    #[test]
    fn config_uses_generation_config_for_lsystem_and_turtle_fields() {
        let cfg = Config::parse(NESTED_KOCH_TOML).unwrap();

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
    fn config_document_from_config_writes_canonical_nested_toml() {
        let mut cfg = Config::parse(NESTED_KOCH_TOML).unwrap();
        cfg.generation.dimensions = Dimensions::ThreeD;
        cfg.generation.axiom = "F\\F".to_string();
        cfg.generation.rules.insert('F', "F\\F".to_string());
        cfg.colors.line = LineColorConfig::Gradient {
            start: [0.1, 0.2, 0.3],
            end: [0.7, 0.8, 0.9],
        };

        let serialized = ConfigDocument::from_config(&cfg).to_string();

        assert!(serialized.contains("[metadata]\n"));
        assert!(serialized.contains("[l-system]\n"));
        assert!(serialized.contains("[l-system.rules]\n"));
        assert!(serialized.contains("[turtle]\n"));
        assert!(serialized.contains("[colors]\n"));
        assert!(serialized.contains("[colors.line]\n"));
        assert!(serialized.contains("axiom = 'F\\F'"));
        assert!(serialized.contains("F = 'F\\F'"));
        assert!(!serialized.contains("F\\\\F"));
        assert!(serialized.contains("start = [0.1, 0.2, 0.3]"));
        assert!(serialized.contains("end = [0.7, 0.8, 0.9]"));
        assert_eq!(
            ConfigDocument::parse(&serialized)
                .unwrap()
                .to_config()
                .unwrap(),
            cfg
        );
    }

    #[test]
    fn parses_nested_v2_config() {
        let cfg = Config::parse(NESTED_KOCH_TOML).unwrap();
        assert_eq!(cfg.name, "Koch Snowflake");
        assert_eq!(cfg.generation.dimensions, Dimensions::TwoD);
        assert_eq!(cfg.generation.axiom, "F++F++F");
        assert_eq!(cfg.generation.angle, 60.0);
        assert_eq!(cfg.generation.step, 1.0);
        assert_eq!(cfg.generation.initial_heading, 0.0);
        assert_eq!(cfg.generation.iterations, 4);
        assert_eq!(cfg.generation.rules[&'F'], "F-F++F-F");
        assert_eq!(cfg.colors.background, [0.0, 0.0, 0.0]);
        match cfg.colors.line {
            LineColorConfig::HueCycle { initial } => assert_eq!(initial, [0.25, 0.5, 0.5]),
            other => panic!("expected hue cycle line color, got {other:?}"),
        }
    }

    #[test]
    fn rejects_flat_v1_schema() {
        assert!(
            Config::parse(KOCH_TOML).is_err(),
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

        let document = ConfigDocument::parse(toml).unwrap();

        assert_eq!(document.to_toml_string(), toml);
        let cfg = document.to_config().unwrap();
        assert_eq!(cfg.generation.axiom, "F\\F");
        assert_eq!(cfg.generation.rules[&'F'], "F\\F");
    }

    #[test]
    fn serializes_canonical_nested_toml_and_round_trips() {
        let cfg = Config::parse(NESTED_KOCH_TOML).unwrap();
        let serialized = cfg.to_toml_string();

        assert!(serialized.contains("[metadata]\n"));
        assert!(serialized.contains("[l-system]\n"));
        assert!(serialized.contains("[l-system.rules]\n"));
        assert!(serialized.contains("[turtle]\n"));
        assert!(serialized.contains("[colors]\n"));
        assert!(serialized.contains("[colors.line]\n"));
        assert!(!serialized.contains("background_color"));
        assert!(!serialized.contains("[line_color]"));
        assert!(!serialized.contains("[rules]"));

        let round_tripped = Config::parse(&serialized).unwrap();
        assert_eq!(round_tripped.name, cfg.name);
        assert_eq!(
            round_tripped.generation.dimensions,
            cfg.generation.dimensions
        );
        assert_eq!(round_tripped.generation.axiom, cfg.generation.axiom);
        assert_eq!(
            round_tripped.generation.iterations,
            cfg.generation.iterations
        );
        assert_eq!(round_tripped.generation.angle, cfg.generation.angle);
        assert_eq!(round_tripped.generation.step, cfg.generation.step);
        assert_eq!(
            round_tripped.generation.initial_heading,
            cfg.generation.initial_heading
        );
        assert_eq!(round_tripped.generation.rules, cfg.generation.rules);
        assert_eq!(round_tripped.to_toml_string(), serialized);
    }

    #[test]
    fn serializes_axiom_and_rules_as_literal_strings() {
        let cfg = Config::parse(
            r#"[metadata]
name = "Backslash"

[l-system]
dimensions = 3
axiom = 'F\F'
iterations = 1

[l-system.rules]
F = 'F\F'

[turtle]
angle = 90.0
step = 1.0
initial_heading = 0.0

[colors]
background = [0.0, 0.0, 0.0]

[colors.line]
mode = "solid"
color = [0.0, 0.9, 0.5]
"#,
        )
        .unwrap();

        let serialized = cfg.to_toml_string();

        assert!(serialized.contains("axiom = 'F\\F'"));
        assert!(serialized.contains("F = 'F\\F'"));
        assert!(!serialized.contains("F\\\\F"));
        let round_tripped = Config::parse(&serialized).unwrap();
        assert_eq!(round_tripped.generation.axiom, "F\\F");
        assert_eq!(round_tripped.generation.rules[&'F'], "F\\F");
    }

    #[test]
    fn serializes_solid_line_color_and_round_trips() {
        let mut cfg = Config::parse(NESTED_KOCH_TOML).unwrap();
        cfg.colors.line = LineColorConfig::Solid {
            color: [0.1, 0.2, 0.3],
        };

        let serialized = cfg.to_toml_string();

        assert!(serialized.contains("mode = \"solid\""));
        assert!(serialized.contains("color = [0.1, 0.2, 0.3]"));
        let round_tripped = Config::parse(&serialized).unwrap();
        match round_tripped.colors.line {
            LineColorConfig::Solid { color } => assert_eq!(color, [0.1, 0.2, 0.3]),
            other => panic!("expected solid line color, got {other:?}"),
        }
        assert_eq!(round_tripped.to_toml_string(), serialized);
    }

    #[test]
    fn serializes_gradient_line_color_and_round_trips() {
        let mut cfg = Config::parse(NESTED_KOCH_TOML).unwrap();
        cfg.colors.line = LineColorConfig::Gradient {
            start: [0.1, 0.2, 0.3],
            end: [0.7, 0.8, 0.9],
        };

        let serialized = cfg.to_toml_string();

        assert!(serialized.contains("mode = \"gradient\""));
        assert!(serialized.contains("start = [0.1, 0.2, 0.3]"));
        assert!(serialized.contains("end = [0.7, 0.8, 0.9]"));
        let round_tripped = Config::parse(&serialized).unwrap();
        match round_tripped.colors.line {
            LineColorConfig::Gradient { start, end } => {
                assert_eq!(start, [0.1, 0.2, 0.3]);
                assert_eq!(end, [0.7, 0.8, 0.9]);
            }
            other => panic!("expected gradient line color, got {other:?}"),
        }
        assert_eq!(round_tripped.to_toml_string(), serialized);
    }

    #[test]
    fn serializes_empty_rules_table_for_ruleless_configs() {
        let cfg = Config::parse(&test_toml(2, "F", 0, "90.0", "1.0", "0.0", "")).unwrap();
        let serialized = cfg.to_toml_string();

        assert!(serialized.contains("[l-system.rules]\n"));
        let round_tripped = Config::parse(&serialized).unwrap();
        assert!(round_tripped.generation.rules.is_empty());
        assert_eq!(round_tripped.to_toml_string(), serialized);
    }

    #[test]
    fn serializing_mutated_invalid_config_does_not_revalidate_and_panic() {
        let mut cfg = Config::parse(NESTED_KOCH_TOML).unwrap();
        cfg.generation.axiom = "F1".to_string();

        let result = std::panic::catch_unwind(|| cfg.to_toml_string());

        assert!(result.is_ok());
    }

    #[test]
    fn parses_valid_config() {
        let cfg = Config::parse(NESTED_KOCH_TOML).unwrap();
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
        assert!(Config::parse(&toml).is_err());
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
        assert!(Config::parse(toml).is_err());
    }

    #[test]
    fn rejects_out_of_range_color_component() {
        let toml = test_toml(2, "F", 1, "90.0", "1.0", "0.0", "");
        let toml = toml.replace(
            "background = [0.0, 0.0, 0.0]",
            "background = [0.0, 1.2, 0.0]",
        );
        let err = Config::parse(&toml).unwrap_err();
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
    fn rejects_non_finite_line_color_component() {
        let toml = test_toml(2, "F", 1, "90.0", "1.0", "0.0", "");
        let toml = toml.replace("color = [0.0, 0.9, 0.5]", "color = [0.0, nan, 0.5]");
        let err = Config::parse(&toml).unwrap_err();
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
        let err = Config::parse(&toml).unwrap_err();

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
            let err = Config::parse(&toml).unwrap_err();

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
        let err = Config::parse(&toml).unwrap_err();

        assert!(
            matches!(err, ConfigError::UnknownField(ref field) if field == "experimental"),
            "unexpected error: {err}"
        );

        let toml = NESTED_KOCH_TOML.replace(
            "initial_heading = 0.0",
            "initial_heading = 0.0\nfriction = 0.5",
        );
        let err = Config::parse(&toml).unwrap_err();

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
            Config::parse(&toml)
                .unwrap_or_else(|err| panic!("failed to parse {}: {err}", path.display()));
        }
    }

    #[test]
    fn strips_whitespace_from_axiom() {
        let toml = test_toml(2, "F + + F", 1, "90.0", "1.0", "0.0", r#"F = "F - F""#);
        let cfg = Config::parse(&toml).unwrap();
        assert_eq!(cfg.generation.axiom, "F++F");
        assert_eq!(cfg.generation.rules[&'F'], "F-F");
    }

    #[test]
    fn rejects_digit_in_axiom() {
        let toml = test_toml(2, "F+1", 1, "90.0", "1.0", "0.0", "");
        let err = Config::parse(&toml).unwrap_err();
        assert!(
            matches!(err, ConfigError::InvalidSymbol { ch: '1', .. }),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_multi_char_rule_key() {
        let toml = test_toml(2, "F", 1, "90.0", "1.0", "0.0", r#"FF = "FFF""#);
        let err = Config::parse(&toml).unwrap_err();
        assert!(
            matches!(err, ConfigError::InvalidRuleKey { .. }),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_invalid_dimensions() {
        for bad_dim in [1, 4, 300] {
            let toml = test_toml(bad_dim, "F", 1, "90.0", "1.0", "0.0", "");
            let err = Config::parse(&toml).unwrap_err();
            assert!(
                matches!(err, ConfigError::InvalidDimensions(d) if d == bad_dim),
                "dim={bad_dim}: unexpected error: {err}"
            );
        }
    }

    #[test]
    fn rejects_non_finite_initial_heading() {
        let toml = test_toml(2, "F", 1, "90.0", "1.0", "nan", "");
        let err = Config::parse(&toml).unwrap_err();

        assert!(
            matches!(err, ConfigError::InvalidInitialHeading(_)),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn accepts_dimensions_3_with_3d_symbols() {
        let toml = test_toml(3, "F&F^F/F", 0, "90.0", "1.0", "0.0", "");
        Config::parse(&toml).expect("3D config with 3D symbols should be valid");
    }

    #[test]
    fn rejects_3d_symbols_in_2d_config() {
        let toml = test_toml(2, "F&F", 1, "90.0", "1.0", "0.0", "");
        let err = Config::parse(&toml).unwrap_err();
        assert!(
            matches!(err, ConfigError::InvalidSymbol { ch: '&', .. }),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_unmatched_close_bracket_in_axiom() {
        let toml = test_toml(2, "F]F", 1, "90.0", "1.0", "0.0", "");
        let err = Config::parse(&toml).unwrap_err();
        assert!(
            matches!(err, ConfigError::UnmatchedClose { position: 1, .. }),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_unclosed_open_bracket_in_axiom() {
        let toml = test_toml(2, "F[F", 1, "90.0", "1.0", "0.0", "");
        let err = Config::parse(&toml).unwrap_err();
        assert!(
            matches!(err, ConfigError::UnmatchedOpen { position: 1, .. }),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn reports_first_unclosed_bracket_not_last() {
        // "F[F[F": two unclosed brackets at positions 1 and 3; error must point to 1.
        let toml = test_toml(2, "F[F[F", 1, "90.0", "1.0", "0.0", "");
        let err = Config::parse(&toml).unwrap_err();
        assert!(
            matches!(err, ConfigError::UnmatchedOpen { position: 1, .. }),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_unbalanced_brackets_in_rule() {
        let toml = test_toml(2, "F", 1, "90.0", "1.0", "0.0", r#"F = "F[+F""#);
        let err = Config::parse(&toml).unwrap_err();
        assert!(
            matches!(err, ConfigError::UnmatchedOpen { .. }),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_non_positive_step() {
        for bad_step in ["0.0", "-1.0"] {
            let toml = test_toml(2, "F", 1, "90.0", bad_step, "0.0", "");
            let err = Config::parse(&toml).unwrap_err();
            assert!(
                matches!(err, ConfigError::InvalidStep(_)),
                "step={bad_step}: unexpected error: {err}"
            );
        }
    }

    #[test]
    fn rejects_non_finite_step() {
        let toml = test_toml(2, "F", 1, "90.0", "inf", "0.0", "");
        let err = Config::parse(&toml).unwrap_err();
        assert!(
            matches!(err, ConfigError::InvalidStep(_)),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_non_finite_angle() {
        let toml = test_toml(2, "F", 1, "nan", "1.0", "0.0", "");
        let err = Config::parse(&toml).unwrap_err();
        assert!(
            matches!(err, ConfigError::InvalidAngle(_)),
            "unexpected error: {err}"
        );
    }
}
