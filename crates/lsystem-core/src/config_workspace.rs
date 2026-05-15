use std::collections::BTreeSet;

use thiserror::Error;

use crate::{Config, ConfigDocument, ConfigError};

#[derive(Debug, Error)]
pub enum ConfigWorkspaceError {
    #[error("at least one config entry is required")]
    Empty,

    #[error("duplicate config entry name `{0}`")]
    DuplicateName(String),

    #[error(transparent)]
    Config(#[from] ConfigError),
}

#[derive(Debug, Clone)]
pub struct ConfigWorkspace {
    entries: Vec<ConfigEntry>,
    selected: usize,
}

#[derive(Debug, Clone)]
pub struct ConfigWorkspaceEntry {
    name: String,
    text: String,
    default_text: Option<String>,
}

impl ConfigWorkspaceEntry {
    pub fn preset(name: String, text: String) -> Self {
        Self {
            name,
            default_text: Some(text.clone()),
            text,
        }
    }

    pub fn custom(name: String, text: String) -> Self {
        Self {
            name,
            text,
            default_text: None,
        }
    }
}

#[derive(Debug, Clone)]
struct ConfigEntry {
    name: String,
    default: Option<DefaultEntry>,
    draft_text: String,
    draft_document: Option<ConfigDocument>,
    last_applied_text: String,
    last_applied_document: ConfigDocument,
    last_applied_config: Config,
}

#[derive(Debug, Clone)]
struct DefaultEntry {
    text: String,
    document: ConfigDocument,
    config: Config,
}

impl ConfigWorkspace {
    pub fn from_presets(
        presets: impl IntoIterator<Item = (String, String)>,
    ) -> Result<Self, ConfigWorkspaceError> {
        Self::from_entries(
            presets
                .into_iter()
                .map(|(name, text)| ConfigWorkspaceEntry::preset(name, text)),
        )
    }

    pub fn from_entries(
        entries: impl IntoIterator<Item = ConfigWorkspaceEntry>,
    ) -> Result<Self, ConfigWorkspaceError> {
        let mut names = BTreeSet::new();
        let mut entries_out = Vec::new();

        for entry in entries {
            let ConfigWorkspaceEntry {
                name,
                text,
                default_text,
            } = entry;
            if !names.insert(name.clone()) {
                return Err(ConfigWorkspaceError::DuplicateName(name));
            }

            let document = ConfigDocument::parse(&text)?;
            let config = document.to_config()?;
            let default = match default_text {
                Some(text) => {
                    let document = ConfigDocument::parse(&text)?;
                    let config = document.to_config()?;
                    Some(DefaultEntry {
                        text,
                        document,
                        config,
                    })
                }
                None => None,
            };
            entries_out.push(ConfigEntry {
                name,
                default,
                draft_text: text.clone(),
                draft_document: Some(document.clone()),
                last_applied_text: text,
                last_applied_document: document,
                last_applied_config: config,
            });
        }

        if entries_out.is_empty() {
            return Err(ConfigWorkspaceError::Empty);
        }
        Ok(Self {
            entries: entries_out,
            selected: 0,
        })
    }

    pub fn selected_index(&self) -> usize {
        self.selected
    }

    pub fn selected_name(&self) -> &str {
        &self.entries[self.selected].name
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|entry| entry.name.as_str())
    }

    pub fn selected_draft_text(&self) -> &str {
        &self.entries[self.selected].draft_text
    }

    pub fn selected_draft_document(&self) -> Option<&ConfigDocument> {
        self.entries[self.selected].draft_document.as_ref()
    }

    pub fn selected_applied_text(&self) -> &str {
        &self.entries[self.selected].last_applied_text
    }

    pub fn selected_applied_document(&self) -> &ConfigDocument {
        &self.entries[self.selected].last_applied_document
    }

    pub fn selected_applied_config(&self) -> &Config {
        &self.entries[self.selected].last_applied_config
    }

    pub fn selected_is_dirty(&self) -> bool {
        let entry = &self.entries[self.selected];
        entry.draft_text != entry.last_applied_text
    }

    pub fn selected_can_reset(&self) -> bool {
        let entry = &self.entries[self.selected];
        entry
            .default
            .as_ref()
            .is_some_and(|default| entry.last_applied_text != default.text)
    }

    pub fn select_index(&mut self, index: usize) -> bool {
        if index >= self.entries.len() {
            return false;
        }
        self.selected = index;
        true
    }

    pub fn select_by_name(&mut self, name: &str) -> bool {
        let Some(index) = self.entries.iter().position(|entry| entry.name == name) else {
            return false;
        };
        self.selected = index;
        true
    }

    pub fn set_selected_draft_text(&mut self, text: String) {
        let entry = &mut self.entries[self.selected];
        // Parse failure is intentional here: a draft may be mid-edit, and
        // apply_selected re-parses and returns the error when submitted.
        entry.draft_document = ConfigDocument::parse(&text).ok();
        entry.draft_text = text;
    }

    pub fn copy_selected(&mut self) -> Result<&Config, ConfigError> {
        let selected = &self.entries[self.selected];
        let text = match selected.draft_document.as_ref() {
            Some(document) if document.to_config().is_ok() => selected.draft_text.clone(),
            _ => selected.last_applied_text.clone(),
        };
        let name = self.unique_name(&format!("{} copy", selected.name));
        self.push_custom(name, text)
    }

    pub fn apply_selected(&mut self) -> Result<&Config, ConfigError> {
        let entry = &mut self.entries[self.selected];
        let document = match &entry.draft_document {
            Some(document) => document.clone(),
            None => ConfigDocument::parse(&entry.draft_text)?,
        };
        let config = document.to_config()?;
        entry.last_applied_text = entry.draft_text.clone();
        entry.last_applied_document = document.clone();
        entry.draft_document = Some(document);
        entry.last_applied_config = config;
        Ok(&entry.last_applied_config)
    }

    pub fn revert_selected(&mut self) -> &str {
        let entry = &mut self.entries[self.selected];
        entry.draft_text.clone_from(&entry.last_applied_text);
        entry.draft_document = Some(entry.last_applied_document.clone());
        &entry.draft_text
    }

    pub fn reset_selected(&mut self) -> Option<&Config> {
        let entry = &mut self.entries[self.selected];
        let default = entry.default.clone()?;
        entry.draft_text = default.text.clone();
        entry.draft_document = Some(default.document.clone());
        entry.last_applied_text = default.text;
        entry.last_applied_document = default.document;
        entry.last_applied_config = default.config;
        Some(&entry.last_applied_config)
    }

    fn push_custom(&mut self, name: String, text: String) -> Result<&Config, ConfigError> {
        let mut document = ConfigDocument::parse(&text)?;
        document.to_config()?;
        document.set_name(&name)?;
        let config = document.to_config()?;
        let text = document.to_toml_string();
        self.entries.push(ConfigEntry {
            name,
            default: None,
            draft_text: text.clone(),
            draft_document: Some(document.clone()),
            last_applied_text: text,
            last_applied_document: document,
            last_applied_config: config,
        });
        self.selected = self.entries.len() - 1;
        Ok(&self.entries[self.selected].last_applied_config)
    }

    fn unique_name(&self, base: &str) -> String {
        let existing: BTreeSet<_> = self
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect();
        if !existing.contains(base) {
            return base.to_string();
        }

        (2..)
            .map(|suffix| format!("{base} {suffix}"))
            .find(|candidate| !existing.contains(candidate.as_str()))
            .expect("unbounded suffix search should find a unique name")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_text(name: &str, axiom: &str, angle: f32) -> String {
        format!(
            r#"[metadata]
name = "{name}"

[l-system]
dimensions = 2
axiom = "{axiom}"
iterations = 1

[l-system.rules]
F = "FF"

[turtle]
angle = {angle}
step = 1.0
initial_heading = 0.0

[colors]
background = [0.0, 0.0, 0.0]

[colors.line]
mode = "solid"
color = [0.0, 0.9, 0.5]
"#
        )
    }

    fn config_text_renamed(text: &str, name: &str) -> String {
        let current_name = Config::parse(text).unwrap().name;
        text.replace(
            &format!("name = \"{current_name}\""),
            &format!("name = \"{name}\""),
        )
    }

    #[test]
    fn switching_entries_preserves_each_draft() {
        let first = config_text("First", "F", 60.0);
        let second = config_text("Second", "F+F", 90.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![
            ("First".to_string(), first),
            ("Second".to_string(), second),
        ])
        .unwrap();

        workspace.set_selected_draft_text("edited first".to_string());
        assert!(workspace.select_by_name("Second"));
        workspace.set_selected_draft_text("edited second".to_string());
        assert!(workspace.select_by_name("First"));

        assert_eq!(workspace.selected_draft_text(), "edited first");
        assert!(workspace.selected_draft_document().is_none());
        assert!(workspace.selected_is_dirty());
        assert!(workspace.select_by_name("Second"));
        assert_eq!(workspace.selected_draft_text(), "edited second");
    }

    #[test]
    fn failed_apply_preserves_last_applied_config() {
        let first = config_text("First", "F", 60.0);
        let mut workspace =
            ConfigWorkspace::from_presets(vec![("First".to_string(), first)]).unwrap();
        let previous_config = workspace.selected_applied_config().clone();

        workspace.set_selected_draft_text("not valid toml".to_string());
        let error = workspace.apply_selected().unwrap_err();
        assert!(matches!(error, ConfigError::TomlParse(_)));

        assert_eq!(workspace.selected_applied_config(), &previous_config);
        assert_eq!(
            workspace.selected_applied_document().to_config().unwrap(),
            previous_config
        );
        assert!(workspace.selected_is_dirty());
    }

    #[test]
    fn apply_rejects_parseable_toml_with_invalid_config() {
        let first = config_text("First", "F", 60.0);
        let mut workspace =
            ConfigWorkspace::from_presets(vec![("First".to_string(), first.clone())]).unwrap();

        workspace.set_selected_draft_text(first.replace("axiom = \"F\"", "axiom = \"[\""));
        let error = workspace.apply_selected().unwrap_err();

        assert!(matches!(error, ConfigError::UnmatchedOpen { .. }));
        assert_eq!(workspace.selected_applied_config().generation.angle, 60.0);
        assert!(workspace.selected_is_dirty());
    }

    #[test]
    fn revert_restores_last_applied_document() {
        let first = config_text("First", "F", 60.0);
        let mut workspace =
            ConfigWorkspace::from_presets(vec![("First".to_string(), first.clone())]).unwrap();

        workspace.set_selected_draft_text(first.replace("angle = 60", "angle = 45"));
        workspace.apply_selected().unwrap();
        let applied = workspace.selected_draft_text().to_string();
        workspace.set_selected_draft_text("temporary invalid text".to_string());

        let reverted = workspace.revert_selected().to_string();

        assert_eq!(reverted, applied);
        assert_eq!(
            workspace.selected_draft_document().unwrap().to_string(),
            applied
        );
        assert!(!workspace.selected_is_dirty());
    }

    #[test]
    fn reset_preset_restores_default_and_applies_it() {
        let first = config_text("First", "F", 60.0);
        let mut workspace =
            ConfigWorkspace::from_presets(vec![("First".to_string(), first.clone())]).unwrap();
        assert!(!workspace.selected_can_reset());

        workspace.set_selected_draft_text(first.replace("angle = 60", "angle = 45"));
        workspace.apply_selected().unwrap();
        assert_eq!(workspace.selected_applied_config().generation.angle, 45.0);
        assert!(workspace.selected_can_reset());

        let reset_config = workspace.reset_selected().unwrap();

        assert_eq!(reset_config.generation.angle, 60.0);
        assert_eq!(workspace.selected_draft_text(), first);
        assert_eq!(workspace.selected_applied_text(), first);
        assert_eq!(workspace.selected_applied_document().to_string(), first);
        assert!(!workspace.selected_is_dirty());
        assert!(!workspace.selected_can_reset());
    }

    #[test]
    fn custom_entry_has_no_default_to_reset() {
        let first = config_text("Custom", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_entries(vec![ConfigWorkspaceEntry::custom(
            "Custom".to_string(),
            first.clone(),
        )])
        .unwrap();

        assert!(!workspace.selected_can_reset());
        workspace.set_selected_draft_text(first.replace("angle = 60", "angle = 45"));
        workspace.apply_selected().unwrap();

        assert!(workspace.reset_selected().is_none());
        assert_eq!(workspace.selected_applied_config().generation.angle, 45.0);
    }

    #[test]
    fn copy_selected_entry_uses_valid_selected_draft_and_unique_copy_name() {
        let first = config_text("Plant", "F", 60.0);
        let second = config_text("Plant copy", "F+F", 90.0);
        let draft = first.replace("angle = 60", "angle = 45");
        let mut workspace = ConfigWorkspace::from_presets(vec![
            ("Plant".to_string(), first.clone()),
            ("Plant copy".to_string(), second),
        ])
        .unwrap();
        workspace.set_selected_draft_text(draft.clone());

        workspace.copy_selected().unwrap();
        let expected_text = config_text_renamed(&draft, "Plant copy 2");

        assert_eq!(workspace.selected_name(), "Plant copy 2");
        assert_eq!(workspace.selected_draft_text(), expected_text);
        assert_eq!(workspace.selected_applied_text(), expected_text);
        assert_eq!(workspace.selected_applied_config().name, "Plant copy 2");
        assert_eq!(workspace.selected_applied_config().generation.angle, 45.0);
        assert!(!workspace.selected_is_dirty());
        assert!(!workspace.selected_can_reset());
    }

    #[test]
    fn copy_selected_entry_falls_back_to_last_applied_when_draft_is_invalid() {
        let first = config_text("Plant", "F", 60.0);
        let mut workspace =
            ConfigWorkspace::from_presets(vec![("Plant".to_string(), first.clone())]).unwrap();
        workspace.set_selected_draft_text("not valid toml".to_string());

        workspace.copy_selected().unwrap();
        let expected_text = config_text_renamed(&first, "Plant copy");

        assert_eq!(workspace.selected_name(), "Plant copy");
        assert_eq!(workspace.selected_draft_text(), expected_text);
        assert_eq!(workspace.selected_applied_text(), expected_text);
        assert_eq!(workspace.selected_applied_config().name, "Plant copy");
        assert_eq!(workspace.selected_applied_config().generation.angle, 60.0);
        assert!(!workspace.selected_is_dirty());
        assert!(!workspace.selected_can_reset());
    }

    #[test]
    fn from_entries_rejects_empty_iterator() {
        let error = ConfigWorkspace::from_entries(Vec::<ConfigWorkspaceEntry>::new()).unwrap_err();

        assert!(matches!(error, ConfigWorkspaceError::Empty));
    }

    #[test]
    fn from_entries_rejects_duplicate_names() {
        let first = config_text("Duplicate", "F", 60.0);
        let second = config_text("Duplicate", "F+F", 90.0);
        let error = ConfigWorkspace::from_presets(vec![
            ("Duplicate".to_string(), first),
            ("Duplicate".to_string(), second),
        ])
        .unwrap_err();

        assert!(matches!(
            error,
            ConfigWorkspaceError::DuplicateName(ref name) if name == "Duplicate"
        ));
    }

    #[test]
    fn from_entries_propagates_invalid_initial_text() {
        let error = ConfigWorkspace::from_entries(vec![ConfigWorkspaceEntry::custom(
            "Broken".to_string(),
            "not valid toml".to_string(),
        )])
        .unwrap_err();

        assert!(matches!(
            error,
            ConfigWorkspaceError::Config(ConfigError::TomlParse(_))
        ));
    }

    #[test]
    fn from_entries_propagates_invalid_default_text() {
        let text = config_text("Default", "F", 60.0);
        let error = ConfigWorkspace::from_entries(vec![ConfigWorkspaceEntry {
            name: "Default".to_string(),
            text,
            default_text: Some("not valid toml".to_string()),
        }])
        .unwrap_err();

        assert!(matches!(
            error,
            ConfigWorkspaceError::Config(ConfigError::TomlParse(_))
        ));
    }

    #[test]
    fn fresh_workspace_is_not_dirty() {
        let first = config_text("First", "F", 60.0);
        let workspace = ConfigWorkspace::from_presets(vec![("First".to_string(), first)]).unwrap();

        assert!(!workspace.selected_is_dirty());
    }

    #[test]
    fn select_index_returns_false_for_out_of_bounds() {
        let first = config_text("First", "F", 60.0);
        let mut workspace =
            ConfigWorkspace::from_presets(vec![("First".to_string(), first)]).unwrap();

        assert!(!workspace.select_index(1));
        assert_eq!(workspace.selected_index(), 0);
    }

    #[test]
    fn select_by_name_returns_false_for_unknown_name() {
        let first = config_text("First", "F", 60.0);
        let mut workspace =
            ConfigWorkspace::from_presets(vec![("First".to_string(), first)]).unwrap();

        assert!(!workspace.select_by_name("Missing"));
        assert_eq!(workspace.selected_name(), "First");
    }
}
