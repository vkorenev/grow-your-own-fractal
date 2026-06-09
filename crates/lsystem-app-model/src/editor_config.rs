use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer};
use toml_edit::{DocumentMut, Item, Table, Value, value};

use lsystem_core::{
    ColorConfig, Config, ConfigError, Dimensions, GenerationConfig, LineColorConfig, Rgb,
    validate_bracket_balance, validate_symbols,
};

use crate::config_defaults::{
    ColorDefaults, ConfigDefaults, LineColorDefaults, ParseConfigError, TomlNumber,
    deserialize_number, parse_rgb, validate_initial_heading, validate_step,
};

/// Validated L-system configuration exactly as authored, before defaults are applied.
#[derive(Debug, Clone, PartialEq)]
pub struct EditorConfig {
    pub name: String,
    pub generation: EditorGenerationConfig,
    pub colors: EditorColorConfig,
}

impl EditorConfig {
    /// Resolves authored config fields with defaults into the runtime config.
    ///
    /// This leaves `EditorConfig` unchanged and fills omitted defaultable fields
    /// from `defaults`. Authored values are preserved faithfully.
    pub fn resolve(&self, defaults: &ConfigDefaults, max_iterations: u32) -> Config {
        let generation = self.generation.resolve(defaults, max_iterations);
        Config {
            name: self.name.clone(),
            generation,
            colors: self.colors.resolve(&defaults.colors),
        }
    }
}

/// Validated generation fields as authored, before defaults are applied.
#[derive(Debug, Clone, PartialEq)]
pub struct EditorGenerationConfig {
    pub dimensions: Dimensions,
    pub axiom: String,
    pub iterations: u32,
    pub angle: f32,
    pub step: Option<f32>,
    pub initial_heading: Option<f32>,
    pub rules: BTreeMap<char, String>,
}

impl EditorGenerationConfig {
    /// Returns `true` if the axiom or any rule RHS contains a `[` push directive.
    ///
    /// Only `[` needs checking; bracket balance is validated, so `]` cannot appear alone.
    pub fn has_stack_directives(&self) -> bool {
        has_stack_directives(&self.axiom, &self.rules)
    }

    /// Resolves authored generation fields with defaults into a runtime `GenerationConfig`.
    ///
    /// Fills omitted `step` and `initial_heading` from `defaults`, and clamps
    /// `iterations` to `max_iterations`. Pass `u32::MAX` to skip clamping.
    pub fn resolve(&self, defaults: &ConfigDefaults, max_iterations: u32) -> GenerationConfig {
        GenerationConfig {
            dimensions: self.dimensions,
            axiom: self.axiom.clone(),
            iterations: self.iterations.min(max_iterations),
            angle: self.angle,
            step: self.step.unwrap_or_else(|| defaults.turtle.step()),
            initial_heading: self
                .initial_heading
                .unwrap_or_else(|| defaults.turtle.initial_heading()),
            rules: self.rules.clone(),
        }
    }
}

/// Validated color fields as authored, before defaults are applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EditorColorConfig {
    pub background: Option<Rgb>,
    /// Authored line-color mode, or `None` when `colors.line` is absent and
    /// resolution should use `LineColorConfig::Solid(defaults.colors.line.solid)`.
    pub line: Option<EditorLineColorConfig>,
}

impl EditorColorConfig {
    pub fn resolve(&self, defaults: &ColorDefaults) -> ColorConfig {
        let background = self.background.unwrap_or(defaults.background);
        let line = self
            .line
            .map(|line| line.resolve(&defaults.line))
            .unwrap_or_else(|| LineColorConfig::Solid(defaults.line.solid));
        ColorConfig { background, line }
    }
}

/// Validated line-color fields as authored, before mode defaults are applied.
///
/// `Solid` always carries the authored color, matching TOML which always provides
/// a hex string. `Gradient` and `HueCycle` fields are `Option` because those
/// parameters may be omitted in TOML, inheriting from defaults at resolve time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorLineColorConfig {
    Solid(Rgb),
    Gradient {
        /// `None` inherits the gradient start color from defaults.
        start: Option<Rgb>,
        /// `None` inherits the gradient end color from defaults.
        end: Option<Rgb>,
        /// `None` inherits the topological-depth setting from defaults.
        topological_depth: Option<bool>,
    },
    HueCycle {
        /// `None` inherits the initial hue-cycle color from defaults.
        initial: Option<Rgb>,
    },
}

impl EditorLineColorConfig {
    /// Returns the gradient fields `(start, end, topological_depth)` if `self` is
    /// `Gradient`, or `(None, None, None)` for any other variant.
    pub fn gradient_fields(self) -> (Option<Rgb>, Option<Rgb>, Option<bool>) {
        match self {
            Self::Gradient {
                start,
                end,
                topological_depth,
            } => (start, end, topological_depth),
            _ => (None, None, None),
        }
    }

    pub fn resolve(self, defaults: &LineColorDefaults) -> LineColorConfig {
        match self {
            Self::Solid(color) => LineColorConfig::Solid(color),
            Self::Gradient {
                start,
                end,
                topological_depth,
            } => LineColorConfig::Gradient {
                start: start.unwrap_or(defaults.gradient.start),
                end: end.unwrap_or(defaults.gradient.end),
                topological_depth: topological_depth.unwrap_or(defaults.gradient.topological_depth),
            },
            Self::HueCycle { initial } => LineColorConfig::HueCycle {
                initial: initial.unwrap_or(defaults.hue_cycle.initial),
            },
        }
    }
}

// --- Raw deserialization types ---

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
    #[serde(default, deserialize_with = "deserialize_optional_number")]
    step: Option<f64>,
    #[serde(default, deserialize_with = "deserialize_optional_number")]
    initial_heading: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawColors {
    #[serde(default)]
    background: Option<String>,
    #[serde(default)]
    line: Option<RawLineColor>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum RawLineColor {
    Solid(String),
    Gradient {
        start: Option<String>,
        end: Option<String>,
        topological_depth: Option<bool>,
    },
    HueCycle {
        initial: Option<String>,
    },
}

// --- TryFrom impls ---

impl TryFrom<RawConfig> for EditorConfig {
    type Error = ConfigError;

    fn try_from(raw: RawConfig) -> Result<Self, Self::Error> {
        let dimensions = Dimensions::from(raw.l_system.dimensions);

        let angle = validate_angle(raw.turtle.angle as f32)?;
        let step = raw
            .turtle
            .step
            .map(|step| validate_step(step as f32))
            .transpose()?;
        let initial_heading = raw
            .turtle
            .initial_heading
            .map(|heading| validate_initial_heading(heading as f32))
            .transpose()?;

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
            generation: EditorGenerationConfig {
                dimensions,
                axiom,
                iterations: raw.l_system.iterations,
                angle,
                step,
                initial_heading,
                rules,
            },
            colors: EditorColorConfig {
                background: raw
                    .colors
                    .background
                    .map(|s| parse_rgb(s, "colors.background"))
                    .transpose()?,
                line: raw
                    .colors
                    .line
                    .map(EditorLineColorConfig::try_from)
                    .transpose()?,
            },
        })
    }
}

impl TryFrom<RawLineColor> for EditorLineColorConfig {
    type Error = ConfigError;

    fn try_from(raw: RawLineColor) -> Result<Self, Self::Error> {
        Ok(match raw {
            RawLineColor::Solid(raw) => Self::Solid(parse_rgb(raw, "colors.line.solid")?),
            RawLineColor::Gradient {
                start,
                end,
                topological_depth,
            } => Self::Gradient {
                start: start
                    .map(|value| parse_rgb(value, "colors.line.gradient.start"))
                    .transpose()?,
                end: end
                    .map(|value| parse_rgb(value, "colors.line.gradient.end"))
                    .transpose()?,
                topological_depth,
            },
            RawLineColor::HueCycle { initial } => Self::HueCycle {
                initial: initial
                    .map(|value| parse_rgb(value, "colors.line.hue_cycle.initial"))
                    .transpose()?,
            },
        })
    }
}

// --- Validation helpers ---

pub(crate) fn validate_angle(angle: f32) -> Result<f32, ConfigError> {
    if !angle.is_finite() {
        return Err(ConfigError::InvalidAngle(angle));
    }
    Ok(angle)
}

fn has_stack_directives(axiom: &str, rules: &BTreeMap<char, String>) -> bool {
    axiom.contains('[') || rules.values().any(|rhs| rhs.contains('['))
}

// --- Custom number deserializers ---

fn deserialize_optional_number<'de, D>(deserializer: D) -> Result<Option<f64>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<TomlNumber>::deserialize(deserializer).map(|n| n.map(|n| n.0))
}

// --- Format-preserving TOML source ---

/// Format-preserving TOML document for an L-system configuration.
#[derive(Debug, Clone)]
pub struct ConfigSource {
    document: DocumentMut,
}

impl ConfigSource {
    pub fn parse(toml_str: &str) -> Result<Self, ParseConfigError> {
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
                    Value::from(hex.to_string()),
                );
            }
            None => {
                if let Some(colors) = self.document["colors"].as_table_mut() {
                    colors.remove("background");
                }
            }
        }
    }

    pub fn set_line_color(&mut self, line_color: Option<&EditorLineColorConfig>) {
        match line_color {
            None => {
                if let Some(colors) = self.document["colors"].as_table_mut() {
                    colors.remove("line");
                }
            }
            Some(EditorLineColorConfig::Solid(color)) => {
                let line = line_table_mut(&mut self.document);
                remove_inactive_line_color_entries(line, "solid");
                set_value_preserving_decor(&mut line["solid"], Value::from(color.to_string()));
            }
            Some(EditorLineColorConfig::Gradient {
                start,
                end,
                topological_depth,
            }) => {
                let line = line_table_mut(&mut self.document);
                remove_inactive_line_color_entries(line, "gradient");
                let gradient = line_color_table_mut(line, "gradient");
                set_or_remove_optional_rgb(gradient, "start", *start);
                set_or_remove_optional_rgb(gradient, "end", *end);
                set_or_remove_optional_bool(gradient, "topological_depth", *topological_depth);
            }
            Some(EditorLineColorConfig::HueCycle { initial }) => {
                let line = line_table_mut(&mut self.document);
                remove_inactive_line_color_entries(line, "hue_cycle");
                let hue_cycle = line_color_table_mut(line, "hue_cycle");
                set_or_remove_optional_rgb(hue_cycle, "initial", *initial);
            }
        }
    }
}

impl std::fmt::Display for ConfigSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_toml_string())
    }
}

/// A [`ConfigSource`] paired with the validated editor config it produces.
///
/// The only constructor is `TryFrom<ConfigSource>`, which validates the document and rejects
/// invalid ones. Holding a `ConfigDocument` is a runtime invariant: the document was validated
/// at construction time, so `editor_config()` always returns the value derived then.
#[derive(Debug, Clone)]
pub struct ConfigDocument {
    source: ConfigSource,
    editor_config: EditorConfig,
}

impl TryFrom<ConfigSource> for ConfigDocument {
    type Error = ParseConfigError;

    fn try_from(source: ConfigSource) -> Result<Self, ParseConfigError> {
        let raw = toml_edit::de::from_document::<RawConfig>(source.document.clone())?;
        let editor_config = EditorConfig::try_from(raw)?;
        Ok(Self {
            source,
            editor_config,
        })
    }
}

impl ConfigDocument {
    pub fn editor_config(&self) -> &EditorConfig {
        &self.editor_config
    }

    pub fn source(&self) -> &ConfigSource {
        &self.source
    }

    pub fn name(&self) -> &str {
        &self.editor_config.name
    }

    pub fn to_toml_string(&self) -> String {
        self.source.to_toml_string()
    }
}

// --- TOML manipulation helpers ---

fn set_value_preserving_decor(item: &mut Item, mut next_value: Value) {
    // This helper only preserves decor for scalar values. Callers that write
    // line-color variant fields first ensure the active variant table exists,
    // then pass only scalar child items here.
    debug_assert!(
        item.as_value().is_some() || item.is_none(),
        "expected scalar Value or absent item; decor will be lost for table items"
    );
    if let Some(current_value) = item.as_value() {
        *next_value.decor_mut() = current_value.decor().clone();
    }
    *item = Item::Value(next_value);
}

const LINE_COLOR_KEYS: &[&str] = &[
    "solid",
    "gradient",
    "hue_cycle",
    // Legacy keys are removed when a clean config is mutated.
    "mode",
    "color",
    "start",
    "end",
    "initial",
];

fn remove_inactive_line_color_entries(line: &mut Table, active_key: &str) {
    for key in LINE_COLOR_KEYS {
        if *key != active_key {
            line.remove(key);
        }
    }
}

fn line_color_table_mut<'a>(line: &'a mut Table, key: &str) -> &'a mut Table {
    // Variant tables own their scalar fields. Replacing a non-table item here is
    // intentional: a validated config cannot have a scalar active variant.
    if line.get(key).is_none_or(|item| !item.is_table()) {
        line[key] = Item::Table(Table::new());
    }
    line[key]
        .as_table_mut()
        .expect("line color variant table was just ensured")
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

fn set_or_remove_optional_rgb(table: &mut Table, key: &str, value: Option<Rgb>) {
    match value {
        Some(rgb) => set_value_preserving_decor(&mut table[key], Value::from(rgb.to_string())),
        None => {
            table.remove(key);
        }
    }
}

fn set_or_remove_optional_bool(table: &mut Table, key: &str, value: Option<bool>) {
    match value {
        Some(b) => set_value_preserving_decor(&mut table[key], Value::from(b)),
        None => {
            table.remove(key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    use crate::config_defaults::{GradientDefaults, HueCycleDefaults, TurtleDefaults};

    fn parse_config(toml_str: &str) -> Result<Config, ParseConfigError> {
        Ok(ConfigDocument::try_from(ConfigSource::parse(toml_str)?)?
            .editor_config()
            .resolve(ConfigDefaults::embedded(), u32::MAX))
    }

    fn resolve_doc(doc: &ConfigDocument) -> Config {
        doc.editor_config()
            .resolve(ConfigDefaults::embedded(), u32::MAX)
    }

    fn hex(s: &str) -> Rgb {
        Rgb::try_from(s).unwrap()
    }

    fn custom_defaults() -> ConfigDefaults {
        ConfigDefaults {
            turtle: TurtleDefaults::try_new(2.5, 15.0).unwrap(),
            colors: crate::config_defaults::ColorDefaults {
                background: hex("#112233"),
                line: LineColorDefaults {
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

    fn assert_toml_deserialize_error_contains(err: ParseConfigError, fragments: &[&str]) -> String {
        assert!(
            matches!(err, ParseConfigError::TomlDeserialize(_)),
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

    fn assert_toml_deserialize_error_mentions_path(err: ParseConfigError, path: &str) {
        let message = err.to_string();
        assert!(
            matches!(err, ParseConfigError::TomlDeserialize(_)),
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

[colors.line.hue_cycle]
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
solid = "#00e680"
"##
        )
    }

    fn minimal_toml(name: &str) -> String {
        format!(
            r#"[metadata]
name = "{name}"

[l-system]
dimensions = "2D"
axiom = "F"
iterations = 1

[l-system.rules]
F = "F"

[turtle]
angle = 90.0

[colors]
"#
        )
    }

    #[test]
    fn set_line_color_none_removes_colors_line() {
        let toml = r##"
[metadata]
name = "t"
[l-system]
dimensions = "2D"
axiom = "F"
iterations = 1
[l-system.rules]
F = "F"
[turtle]
angle = 90.0
[colors.line.hue_cycle]
initial = "#e60000"
"##;
        let mut source = ConfigSource::parse(toml).unwrap();
        source.set_line_color(None);
        let out = source.to_toml_string();
        assert!(!out.contains("hue_cycle"), "colors.line must be removed");
    }

    #[test]
    fn set_line_color_gradient_omits_none_fields() {
        let toml = minimal_toml("t");
        let mut source = ConfigSource::parse(&toml).unwrap();
        source.set_line_color(Some(&EditorLineColorConfig::Gradient {
            start: Some(Rgb::new(0x11, 0x22, 0x33)),
            end: None,
            topological_depth: None,
        }));
        let out = source.to_toml_string();
        assert!(out.contains("#112233"), "start must be written");
        assert!(!out.contains("end"), "absent end must not be written");
        assert!(
            !out.contains("topological_depth"),
            "absent td must not be written"
        );
    }

    #[test]
    fn set_line_color_hue_cycle_omits_none_initial() {
        let toml = minimal_toml("t");
        let mut source = ConfigSource::parse(&toml).unwrap();
        source.set_line_color(Some(&EditorLineColorConfig::HueCycle { initial: None }));
        let out = source.to_toml_string();
        assert!(
            out.contains("[colors.line"),
            "hue_cycle table header must be present"
        );
        assert!(
            !out.contains("initial"),
            "absent initial must not be written"
        );
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

[colors.line.gradient]
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
solid = "#00e680"
"##;
        let mut source = ConfigSource::parse(original).unwrap();

        source.set_name("New");

        assert!(
            source
                .to_toml_string()
                .contains(r#"name = "New" # keep name comment"#)
        );
        let doc = ConfigDocument::try_from(source).unwrap();
        let config = resolve_doc(&doc);
        assert_eq!(config.name, "New");
    }

    #[test]
    fn config_document_name_comes_from_editor_config() {
        let doc = ConfigDocument::try_from(ConfigSource::parse(NESTED_KOCH_TOML).unwrap()).unwrap();

        assert_eq!(doc.name(), doc.editor_config().name);
        assert_eq!(doc.name(), "Koch Snowflake");
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
    fn editor_config_preserves_omitted_defaultable_fields() {
        let toml = NESTED_KOCH_TOML
            .replace("step = 1.0\n", "")
            .replace("initial_heading = 0.0\n", "")
            .replace("background = \"#000000\"\n\n", "")
            .replace(
                "\n[colors.line]\n\n[colors.line.hue_cycle]\ninitial = \"#408080\"\n",
                "",
            );

        let doc = ConfigDocument::try_from(ConfigSource::parse(&toml).unwrap()).unwrap();

        assert_eq!(doc.editor_config().generation.step, None);
        assert_eq!(doc.editor_config().generation.initial_heading, None);
        assert_eq!(doc.editor_config().colors.background, None);
        assert_eq!(doc.editor_config().colors.line, None);
        assert_eq!(
            resolve_doc(&doc).generation.step,
            ConfigDefaults::embedded().turtle.step()
        );
        assert_eq!(
            resolve_doc(&doc).generation.initial_heading,
            ConfigDefaults::embedded().turtle.initial_heading()
        );
        assert_eq!(
            resolve_doc(&doc).colors.background,
            ConfigDefaults::embedded().colors.background
        );
        assert_eq!(
            resolve_doc(&doc).colors.line,
            LineColorConfig::Solid(ConfigDefaults::embedded().colors.line.solid)
        );
    }

    #[test]
    fn editor_config_resolves_empty_line_mode_tables_from_defaults() {
        let toml = NESTED_KOCH_TOML.replace(
            "[colors.line]\n\n[colors.line.hue_cycle]\ninitial = \"#408080\"",
            "[colors.line.gradient]",
        );

        let doc = ConfigDocument::try_from(ConfigSource::parse(&toml).unwrap()).unwrap();

        assert_eq!(
            doc.editor_config().colors.line,
            Some(EditorLineColorConfig::Gradient {
                start: None,
                end: None,
                topological_depth: None,
            })
        );
        let defaults = ConfigDefaults::embedded().colors.line.gradient;
        assert_eq!(
            resolve_doc(&doc).colors.line,
            LineColorConfig::Gradient {
                start: defaults.start,
                end: defaults.end,
                topological_depth: defaults.topological_depth,
            }
        );
    }

    #[test]
    fn editor_gradient_resolves_from_supplied_defaults_not_rust_constants() {
        let defaults = custom_defaults();

        let resolved = EditorLineColorConfig::Gradient {
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

    #[test]
    fn editor_hue_cycle_resolves_from_supplied_defaults_not_rust_constants() {
        let defaults = custom_defaults();

        let resolved =
            EditorLineColorConfig::HueCycle { initial: None }.resolve(&defaults.colors.line);

        assert_eq!(
            resolved,
            LineColorConfig::HueCycle {
                initial: hex("#fedcba"),
            }
        );
    }

    #[test]
    fn editor_config_resolve_uses_supplied_defaults_not_rust_constants() {
        let toml = NESTED_KOCH_TOML
            .replace("step = 1.0\n", "")
            .replace("initial_heading = 0.0\n", "")
            .replace("background = \"#000000\"\n\n", "")
            .replace(
                "\n[colors.line]\n\n[colors.line.hue_cycle]\ninitial = \"#408080\"\n",
                "",
            );
        let editor = EditorConfig::try_from(
            toml_edit::de::from_document::<RawConfig>(
                ConfigSource::parse(&toml).unwrap().document.clone(),
            )
            .unwrap(),
        )
        .unwrap();

        let config = editor.resolve(&custom_defaults(), u32::MAX);

        assert_eq!(config.generation.step, 2.5);
        assert_eq!(config.generation.initial_heading, 15.0);
        assert_eq!(config.colors.background, hex("#112233"));
        assert_eq!(config.colors.line, LineColorConfig::Solid(hex("#445566")));
    }

    #[test]
    fn editor_config_preserves_present_turtle_defaults_as_some() {
        let toml = NESTED_KOCH_TOML
            .replace("step = 1.0", "step = 3.7")
            .replace("initial_heading = 0.0", "initial_heading = 45.0");

        let doc = ConfigDocument::try_from(ConfigSource::parse(&toml).unwrap()).unwrap();

        assert_eq!(doc.editor_config().generation.step, Some(3.7));
        assert_eq!(doc.editor_config().generation.initial_heading, Some(45.0));
        assert_eq!(resolve_doc(&doc).generation.step, 3.7);
        assert_eq!(resolve_doc(&doc).generation.initial_heading, 45.0);
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
        assert_eq!(cfg.colors.background, hex("#000000"));
        match cfg.colors.line {
            LineColorConfig::HueCycle { initial } => assert_eq!(initial, hex("#408080")),
            other => panic!("expected hue cycle line color, got {other:?}"),
        }
    }

    #[test]
    fn parses_missing_background_as_none() {
        let toml = NESTED_KOCH_TOML.replace("background = \"#000000\"\n\n", "");
        let doc = ConfigDocument::try_from(ConfigSource::parse(&toml).unwrap()).unwrap();

        assert_eq!(doc.editor_config().colors.background, None);
        assert_eq!(
            resolve_doc(&doc).colors.background,
            ConfigDefaults::embedded().colors.background
        );
    }

    #[test]
    fn parses_present_background_as_some_hex() {
        let toml = NESTED_KOCH_TOML.replace("background = \"#000000\"", "background = \"#334d66\"");
        let doc = ConfigDocument::try_from(ConfigSource::parse(&toml).unwrap()).unwrap();

        assert_eq!(doc.editor_config().colors.background, Some(hex("#334d66")));
        assert_eq!(resolve_doc(&doc).colors.background, hex("#334d66"));
    }

    #[test]
    fn rejects_old_array_background() {
        let toml =
            NESTED_KOCH_TOML.replace("background = \"#000000\"", "background = [0.0, 0.0, 0.0]");
        let err = parse_config(&toml).unwrap_err();

        assert!(
            matches!(err, ParseConfigError::TomlDeserialize(_)),
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
                ParseConfigError::Validation(ConfigError::InvalidRgb {
                    ref field,
                    ref value
                }) if field == "colors.background" && value == "notahex"
            ),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn accepts_uppercase_hex_background() {
        let toml = NESTED_KOCH_TOML.replace("background = \"#000000\"", "background = \"#AABBCC\"");
        let cfg = parse_config(&toml).unwrap();

        assert_eq!(cfg.colors.background, hex("#aabbcc"));
    }

    #[test]
    fn parses_omitted_line_color_as_default_solid() {
        let toml = test_toml(Dimensions::TwoD, "F", 1, "90.0", "1.0", "0.0", "")
            .replace("\n[colors.line]\nsolid = \"#00e680\"\n", "");

        let cfg = parse_config(&toml).unwrap();

        assert_eq!(
            cfg.colors.line,
            LineColorConfig::Solid(ConfigDefaults::embedded().colors.line.solid)
        );
    }

    #[test]
    fn parses_scalar_solid_line_color() {
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

        assert_eq!(cfg.colors.line, LineColorConfig::Solid(hex("#00e680")));
    }

    #[test]
    fn parses_full_gradient_line_color() {
        let toml = test_toml(Dimensions::TwoD, "F", 1, "90.0", "1.0", "0.0", "").replace(
            "[colors.line]\nsolid = \"#00e680\"",
            "[colors.line.gradient]\nstart = \"#1a334d\"\nend = \"#b3cce6\"",
        );

        let cfg = parse_config(&toml).unwrap();

        assert_eq!(
            cfg.colors.line,
            LineColorConfig::Gradient {
                start: hex("#1a334d"),
                end: hex("#b3cce6"),
                topological_depth: false,
            }
        );
    }

    #[test]
    fn parses_empty_gradient_line_color_with_defaults() {
        let toml = test_toml(Dimensions::TwoD, "F", 1, "90.0", "1.0", "0.0", "").replace(
            "[colors.line]\nsolid = \"#00e680\"",
            "[colors.line.gradient]",
        );

        let cfg = parse_config(&toml).unwrap();

        let defaults = ConfigDefaults::embedded().colors.line.gradient;
        assert_eq!(
            cfg.colors.line,
            LineColorConfig::Gradient {
                start: defaults.start,
                end: defaults.end,
                topological_depth: defaults.topological_depth,
            }
        );
    }

    #[test]
    fn rejects_empty_line_color_table() {
        let toml = test_toml(Dimensions::TwoD, "F", 1, "90.0", "1.0", "0.0", "")
            .replace("[colors.line]\nsolid = \"#00e680\"", "[colors.line]");

        let err = parse_config(&toml).unwrap_err();

        assert_toml_deserialize_error_contains(err, &["colors.line"]);
    }

    #[test]
    fn parses_gradient_line_color_with_topological_depth() {
        let toml = test_toml(Dimensions::TwoD, "F", 1, "90.0", "1.0", "0.0", "").replace(
            "[colors.line]\nsolid = \"#00e680\"",
            "[colors.line.gradient]\nstart = \"#1a334d\"\nend = \"#b3cce6\"\ntopological_depth = true",
        );

        let doc = ConfigDocument::try_from(ConfigSource::parse(&toml).unwrap()).unwrap();

        assert_eq!(
            doc.editor_config().colors.line,
            Some(EditorLineColorConfig::Gradient {
                start: Some(hex("#1a334d")),
                end: Some(hex("#b3cce6")),
                topological_depth: Some(true),
            })
        );
        // After refactor: resolve faithfully preserves authored topological_depth = true.
        // Callers that need to decide whether to allocate depth geometry must also check
        // config.generation.has_stack_directives() — bracketless grammar, depth = true
        // is valid and the geometry selection is the caller's responsibility.
        assert_eq!(
            resolve_doc(&doc).colors.line,
            LineColorConfig::Gradient {
                start: hex("#1a334d"),
                end: hex("#b3cce6"),
                topological_depth: true,
            }
        );
    }

    #[test]
    fn parses_hue_cycle_line_color() {
        let toml = test_toml(Dimensions::TwoD, "F", 1, "90.0", "1.0", "0.0", "").replace(
            "[colors.line]\nsolid = \"#00e680\"",
            "[colors.line.hue_cycle]\ninitial = \"#e60000\"",
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
    fn parses_empty_hue_cycle_line_color_with_default() {
        let toml = test_toml(Dimensions::TwoD, "F", 1, "90.0", "1.0", "0.0", "").replace(
            "[colors.line]\nsolid = \"#00e680\"",
            "[colors.line.hue_cycle]",
        );

        let cfg = parse_config(&toml).unwrap();

        assert_eq!(
            cfg.colors.line,
            LineColorConfig::HueCycle {
                initial: ConfigDefaults::embedded().colors.line.hue_cycle.initial,
            }
        );
    }

    #[test]
    fn rejects_invalid_hex_for_initial() {
        let toml = test_toml(Dimensions::TwoD, "F", 1, "90.0", "1.0", "0.0", "").replace(
            "[colors.line]\nsolid = \"#00e680\"",
            "[colors.line.hue_cycle]\ninitial = \"#zzzzzz\"",
        );
        let err = parse_config(&toml).unwrap_err();
        assert!(
            matches!(
                err,
                ParseConfigError::Validation(ConfigError::InvalidRgb { ref field, .. }) if field == "colors.line.hue_cycle.initial"
            ),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_old_line_color_mode_schema() {
        let toml = test_toml(Dimensions::TwoD, "F", 1, "90.0", "1.0", "0.0", "").replace(
            "[colors.line]\nsolid = \"#00e680\"",
            "[colors.line]\nmode = \"solid\"\ncolor = \"#00e680\"",
        );

        let err = parse_config(&toml).unwrap_err();

        assert_toml_deserialize_error_contains(err, &["colors.line"]);
    }

    #[test]
    fn rejects_multiple_active_line_color_entries() {
        let toml = test_toml(Dimensions::TwoD, "F", 1, "90.0", "1.0", "0.0", "").replace(
            "[colors.line]\nsolid = \"#00e680\"",
            "[colors.line]\nsolid = \"#00e680\"\n\n[colors.line.gradient]\nstart = \"#000000\"\nend = \"#ffffff\"",
        );

        let err = parse_config(&toml).unwrap_err();

        assert_toml_deserialize_error_contains(err, &["colors.line"]);
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
colors.line.solid = "#00e680"
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
solid = "#00e680"
"##;

        let cfg = parse_config(toml).unwrap();

        assert_eq!(cfg.name, "Implicit Parents");
        assert_eq!(cfg.generation.dimensions, Dimensions::TwoD);
    }

    #[test]
    fn rejects_flat_v1_schema() {
        let flat_toml = r#"
name = "Koch Snowflake"
dimensions = "2D"
axiom = "F++F++F"
iterations = 4
angle = 60.0
step = 1.0

[rules]
F = "F-F++F-F"
"#;
        assert!(
            parse_config(flat_toml).is_err(),
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
solid = "#1a334d"
"##;

        let source = ConfigSource::parse(toml).unwrap();

        assert_eq!(source.to_toml_string(), toml);
        let doc = ConfigDocument::try_from(source).unwrap();
        let cfg = resolve_doc(&doc);
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
    fn missing_turtle_fields_resolve_from_defaults() {
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
solid = "#00e680"
"##;
        let config = parse_config(toml).unwrap();
        let defaults = ConfigDefaults::embedded();
        assert_eq!(config.generation.step, defaults.turtle.step());
        assert_eq!(
            config.generation.initial_heading,
            defaults.turtle.initial_heading()
        );
    }

    #[test]
    fn missing_initial_heading_resolves_from_defaults() {
        let toml = NESTED_KOCH_TOML.replace("initial_heading = 0.0\n", "");
        let config = parse_config(&toml).unwrap();
        assert_eq!(
            config.generation.initial_heading,
            ConfigDefaults::embedded().turtle.initial_heading()
        );
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
solid = "#00e680"
"##;
        let err = parse_config(toml).unwrap_err();
        assert_toml_deserialize_error_mentions_path(err, "l-system.rules");
    }

    #[test]
    fn rejects_invalid_hex_for_background_in_test_toml() {
        let toml = test_toml(Dimensions::TwoD, "F", 1, "90.0", "1.0", "0.0", "");
        let toml = toml.replace("background = \"#000000\"", "background = \"notahex\"");
        let err = parse_config(&toml).unwrap_err();
        assert!(
            matches!(
                err,
                ParseConfigError::Validation(ConfigError::InvalidRgb {
                    ref field,
                    ref value
                }) if field == "colors.background" && value == "notahex"
            ),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_invalid_hex_for_gradient_colors() {
        for (expected_field, replacement) in [
            (
                "colors.line.gradient.start",
                "[colors.line.gradient]\nstart = \"bad\"\nend = \"#b3cce6\"",
            ),
            (
                "colors.line.gradient.end",
                "[colors.line.gradient]\nstart = \"#1a334d\"\nend = \"bad\"",
            ),
            (
                "colors.line.gradient.start",
                "[colors.line.gradient]\nstart = \"bad\"\nend = \"#b3cce6\"\ntopological_depth = true",
            ),
            (
                "colors.line.gradient.end",
                "[colors.line.gradient]\nstart = \"#1a334d\"\nend = \"bad\"\ntopological_depth = true",
            ),
        ] {
            let toml = test_toml(Dimensions::TwoD, "F", 1, "90.0", "1.0", "0.0", "")
                .replace("[colors.line]\nsolid = \"#00e680\"", replacement);
            let err = parse_config(&toml).unwrap_err();
            assert!(
                matches!(
                    err,
                    ParseConfigError::Validation(ConfigError::InvalidRgb {
                        field: ref error_field,
                        ref value
                    }) if error_field == expected_field && value == "bad"
                ),
                "unexpected error: {err}"
            );
        }
    }

    #[test]
    fn rejects_invalid_hex_for_line_color() {
        let toml = test_toml(Dimensions::TwoD, "F", 1, "90.0", "1.0", "0.0", "");
        let toml = toml.replace("solid = \"#00e680\"", "solid = \"#zzzzzz\"");
        let err = parse_config(&toml).unwrap_err();
        assert!(
            matches!(
                err,
                ParseConfigError::Validation(ConfigError::InvalidRgb {
                    ref field,
                    ..
                }) if field == "colors.line.solid"
            ),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_old_array_for_line_color() {
        let toml = test_toml(Dimensions::TwoD, "F", 1, "90.0", "1.0", "0.0", "");
        let toml = toml.replace("solid = \"#00e680\"", "solid = [0.0, 0.9, 0.5]");
        let err = parse_config(&toml).unwrap_err();
        assert!(
            matches!(err, ParseConfigError::TomlDeserialize(_)),
            "expected TomlDeserialize error for array line color, got: {err}"
        );
    }

    #[test]
    fn rejects_unknown_line_color_mode() {
        let toml = test_toml(Dimensions::TwoD, "F", 1, "90.0", "1.0", "0.0", "");
        let toml = toml.replace("solid = \"#00e680\"", "rainbow = \"#00e680\"");
        let err = parse_config(&toml).unwrap_err();

        assert_toml_deserialize_error_contains(err, &["colors.line", "rainbow"]);
    }

    #[test]
    fn rejects_unknown_keys_for_line_color_modes() {
        let cases = [
            (
                "solid",
                "[colors.line]\nsolid = \"#00e680\"\nstart = \"#000000\"",
                "colors.line",
            ),
            (
                "gradient",
                "[colors.line.gradient]\nstart = \"#000000\"\nend = \"#ffffff\"\ncolor = \"#00e680\"",
                "colors.line",
            ),
            (
                "hue_cycle",
                "[colors.line.hue_cycle]\ninitial = \"#00e680\"\nend = \"#ffffff\"",
                "colors.line",
            ),
        ];

        for (_, replacement, expected_field) in cases {
            let toml = test_toml(Dimensions::TwoD, "F", 1, "90.0", "1.0", "0.0", "");
            let toml = toml.replace("[colors.line]\nsolid = \"#00e680\"", replacement);
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
            matches!(
                err,
                ParseConfigError::Validation(ConfigError::InvalidSymbol { ch: '1', .. })
            ),
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
            matches!(
                err,
                ParseConfigError::Validation(ConfigError::InvalidRuleKey { .. })
            ),
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
            matches!(
                err,
                ParseConfigError::Validation(ConfigError::InvalidInitialHeading(_))
            ),
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
            matches!(
                err,
                ParseConfigError::Validation(ConfigError::InvalidSymbol { ch: '&', .. })
            ),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_unmatched_close_bracket_in_axiom() {
        let toml = test_toml(Dimensions::TwoD, "F]F", 1, "90.0", "1.0", "0.0", "");
        let err = parse_config(&toml).unwrap_err();
        assert!(
            matches!(
                err,
                ParseConfigError::Validation(ConfigError::UnmatchedClose { position: 1, .. })
            ),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_unclosed_open_bracket_in_axiom() {
        let toml = test_toml(Dimensions::TwoD, "F[F", 1, "90.0", "1.0", "0.0", "");
        let err = parse_config(&toml).unwrap_err();
        assert!(
            matches!(
                err,
                ParseConfigError::Validation(ConfigError::UnmatchedOpen { position: 1, .. })
            ),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn reports_first_unclosed_bracket_not_last() {
        let toml = test_toml(Dimensions::TwoD, "F[F[F", 1, "90.0", "1.0", "0.0", "");
        let err = parse_config(&toml).unwrap_err();
        assert!(
            matches!(
                err,
                ParseConfigError::Validation(ConfigError::UnmatchedOpen { position: 1, .. })
            ),
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
            matches!(
                err,
                ParseConfigError::Validation(ConfigError::UnmatchedOpen { .. })
            ),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_non_positive_step() {
        for bad_step in ["0.0", "-1.0"] {
            let toml = test_toml(Dimensions::TwoD, "F", 1, "90.0", bad_step, "0.0", "");
            let err = parse_config(&toml).unwrap_err();
            assert!(
                matches!(
                    err,
                    ParseConfigError::Validation(ConfigError::InvalidStep(_))
                ),
                "step={bad_step}: unexpected error: {err}"
            );
        }
    }

    #[test]
    fn rejects_non_finite_step() {
        let toml = test_toml(Dimensions::TwoD, "F", 1, "90.0", "inf", "0.0", "");
        let err = parse_config(&toml).unwrap_err();
        assert!(
            matches!(
                err,
                ParseConfigError::Validation(ConfigError::InvalidStep(_))
            ),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_non_finite_angle() {
        let toml = test_toml(Dimensions::TwoD, "F", 1, "nan", "1.0", "0.0", "");
        let err = parse_config(&toml).unwrap_err();
        assert!(
            matches!(
                err,
                ParseConfigError::Validation(ConfigError::InvalidAngle(_))
            ),
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
    fn editor_resolve_preserves_topological_gradient_for_bracketless() {
        // After the refactor: resolve() no longer normalizes; topological_depth = true
        // is preserved faithfully. Callers (svg_export, png_export) check
        // needs_topological_depth() && has_stack_directives() at their boundary.
        let toml = test_toml(Dimensions::TwoD, "F-F++F-F", 1, "60.0", "1.0", "0.0", "").replace(
            "[colors.line]\nsolid = \"#00e680\"",
            "[colors.line.gradient]\nstart = \"#1a334d\"\nend = \"#b3cce6\"\ntopological_depth = true",
        );
        let doc = ConfigDocument::try_from(ConfigSource::parse(&toml).unwrap()).unwrap();
        let start = hex("#1a334d");
        let end = hex("#b3cce6");
        assert_eq!(
            doc.editor_config().colors.line,
            Some(EditorLineColorConfig::Gradient {
                start: Some(start),
                end: Some(end),
                topological_depth: Some(true),
            }),
            "authored topological-gradient choice must be preserved in editor config"
        );
        assert_eq!(
            resolve_doc(&doc).colors.line,
            LineColorConfig::Gradient {
                start,
                end,
                topological_depth: true,
            },
            "faithful resolve must preserve authored topological_depth = true"
        );
    }

    #[test]
    fn editor_resolve_preserves_topological_gradient_for_bracket_fractal() {
        let toml = test_toml(
            Dimensions::TwoD,
            "F",
            1,
            "25.0",
            "1.0",
            "0.0",
            "F = \"F[+F]F[-F]F\"",
        )
        .replace(
            "[colors.line]\nsolid = \"#00e680\"",
            "[colors.line.gradient]\nstart = \"#1a334d\"\nend = \"#b3cce6\"\ntopological_depth = true",
        );
        let cfg = parse_config(&toml).unwrap();
        let start = hex("#1a334d");
        let end = hex("#b3cce6");
        assert_eq!(
            cfg.colors.line,
            LineColorConfig::Gradient {
                start,
                end,
                topological_depth: true,
            },
            "bracket fractal topological gradient must be preserved"
        );
    }

    #[test]
    fn editor_resolve_passes_through_solid_for_bracketless() {
        let toml = test_toml(Dimensions::TwoD, "F-F++F-F", 1, "60.0", "1.0", "0.0", "");
        let cfg = parse_config(&toml).unwrap();
        assert_eq!(cfg.colors.line, LineColorConfig::Solid(hex("#00e680")));
        assert_eq!(cfg.colors.background, hex("#000000"));
    }

    #[test]
    fn editor_resolve_passes_through_traversal_gradient_for_bracketless() {
        let toml = test_toml(Dimensions::TwoD, "F-F++F-F", 1, "60.0", "1.0", "0.0", "").replace(
            "[colors.line]\nsolid = \"#00e680\"",
            "[colors.line.gradient]\nstart = \"#ff0000\"\nend = \"#0000ff\"\ntopological_depth = false",
        );
        let cfg = parse_config(&toml).unwrap();
        assert_eq!(
            cfg.colors.line,
            LineColorConfig::Gradient {
                start: hex("#ff0000"),
                end: hex("#0000ff"),
                topological_depth: false,
            }
        );
    }

    #[test]
    fn editor_resolve_passes_through_hue_cycle_for_bracketless() {
        let toml = test_toml(Dimensions::TwoD, "F-F++F-F", 1, "60.0", "1.0", "0.0", "").replace(
            "[colors.line]\nsolid = \"#00e680\"",
            "[colors.line.hue_cycle]\ninitial = \"#ff0000\"",
        );
        let cfg = parse_config(&toml).unwrap();
        assert_eq!(
            cfg.colors.line,
            LineColorConfig::HueCycle {
                initial: hex("#ff0000"),
            }
        );
    }

    #[test]
    fn set_background_writes_hex_string() {
        let toml = test_toml(Dimensions::TwoD, "F", 1, "90.0", "1.0", "0.0", "");
        let mut source = ConfigSource::parse(&toml).unwrap();
        source.set_background(Some(Rgb::new(0xff, 0x80, 0x00)));
        let result = source.to_toml_string();
        assert!(
            result.contains(r##"background = "#ff8000""##),
            "expected hex string, got: {result}"
        );
        // Verify it round-trips: re-parsing produces the same Rgb
        let doc = ConfigDocument::try_from(ConfigSource::parse(&result).unwrap()).unwrap();
        assert_eq!(
            resolve_doc(&doc).colors.background,
            Rgb::new(0xff, 0x80, 0x00)
        );
    }
}
