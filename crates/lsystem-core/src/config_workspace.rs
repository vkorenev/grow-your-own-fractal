use std::borrow::Cow;
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
    text: String,
    default_text: Option<String>,
}

impl ConfigWorkspaceEntry {
    pub fn preset(text: String) -> Self {
        Self {
            default_text: Some(text.clone()),
            text,
        }
    }

    pub fn custom(text: String) -> Self {
        Self {
            text,
            default_text: None,
        }
    }
}

#[derive(Debug, Clone)]
struct ConfigEntry {
    default: Option<ConfigDocument>,
    draft: Option<String>,
    last_applied_document: ConfigDocument,
}

impl ConfigWorkspace {
    pub fn from_presets(
        presets: impl IntoIterator<Item = String>,
    ) -> Result<Self, ConfigWorkspaceError> {
        Self::from_entries(presets.into_iter().map(ConfigWorkspaceEntry::preset))
    }

    pub fn from_entries(
        entries: impl IntoIterator<Item = ConfigWorkspaceEntry>,
    ) -> Result<Self, ConfigWorkspaceError> {
        let mut names = BTreeSet::new();
        let mut entries_out = Vec::new();

        for entry in entries {
            let ConfigWorkspaceEntry { text, default_text } = entry;
            let document = ConfigDocument::parse(&text)?;
            let name = document.to_config()?.name;
            if !names.insert(name.clone()) {
                return Err(ConfigWorkspaceError::DuplicateName(name));
            }

            let default = match default_text {
                Some(text) => {
                    let document = ConfigDocument::parse(&text)?;
                    document.to_config()?;
                    Some(document)
                }
                None => None,
            };
            entries_out.push(ConfigEntry {
                default,
                draft: None,
                last_applied_document: document,
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
        self.entries[self.selected].name()
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(ConfigEntry::name)
    }

    pub fn selected_draft_text(&self) -> Cow<'_, str> {
        self.entries[self.selected].draft_text()
    }

    pub fn selected_draft_document(&self) -> Option<ConfigDocument> {
        self.entries[self.selected].draft_document()
    }

    pub fn selected_applied_text(&self) -> String {
        self.entries[self.selected].applied_text()
    }

    pub fn selected_applied_document(&self) -> &ConfigDocument {
        &self.entries[self.selected].last_applied_document
    }

    pub fn selected_applied_config(&self) -> Config {
        self.entries[self.selected].applied_config()
    }

    pub fn selected_is_dirty(&self) -> bool {
        self.entries[self.selected].draft.is_some()
    }

    pub fn selected_can_reset(&self) -> bool {
        let entry = &self.entries[self.selected];
        let Some(default) = &entry.default else {
            return false;
        };
        let Some(name) = default.name() else {
            return false;
        };
        entry.applied_text() != default.to_toml_string()
            && !self.name_exists_except(name, self.selected)
    }

    pub fn select_index(&mut self, index: usize) -> bool {
        if index >= self.entries.len() {
            return false;
        }
        self.selected = index;
        true
    }

    pub fn select_by_name(&mut self, name: &str) -> bool {
        let Some(index) = self.entries.iter().position(|entry| entry.name() == name) else {
            return false;
        };
        self.selected = index;
        true
    }

    pub fn set_selected_draft_text(&mut self, text: String) {
        let applied_text = self.entries[self.selected].applied_text();
        self.entries[self.selected].draft = (text != applied_text).then_some(text);
    }

    pub fn copy_selected(&mut self) -> Config {
        let selected = &self.entries[self.selected];
        let name = self.unique_name(&format!("{} copy", selected.name()));
        let draft = selected.draft.as_ref().map(|draft_text| {
            ConfigDocument::parse(draft_text).map_or_else(
                |_| draft_text.clone(),
                |mut document| {
                    document.set_name(&name);
                    document.to_toml_string()
                },
            )
        });

        let mut last_applied_document = selected.last_applied_document.clone();
        last_applied_document.set_name(&name);
        let config = last_applied_document
            .to_config()
            .expect("renamed applied document should remain a valid config");
        self.entries.push(ConfigEntry {
            default: None,
            draft,
            last_applied_document,
        });
        self.selected = self.entries.len() - 1;
        config
    }

    pub fn apply_selected(&mut self) -> Result<Config, ConfigWorkspaceError> {
        let selected = self.selected;
        let Some(draft) = &self.entries[selected].draft else {
            return Ok(self.entries[selected].applied_config());
        };
        let document = ConfigDocument::parse(draft)?;
        let config = document.to_config()?;
        if self.name_exists_except(&config.name, selected) {
            return Err(ConfigWorkspaceError::DuplicateName(config.name));
        }
        let entry = &mut self.entries[selected];
        entry.last_applied_document = document;
        entry.draft = None;
        Ok(config)
    }

    pub fn revert_selected(&mut self) -> String {
        let entry = &mut self.entries[self.selected];
        entry.draft = None;
        entry.applied_text()
    }

    pub fn reset_selected(&mut self) -> Result<Option<Config>, ConfigWorkspaceError> {
        let selected = self.selected;
        let Some(default) = self.entries[selected].default.clone() else {
            return Ok(None);
        };
        let config = default.to_config()?;
        if self.name_exists_except(&config.name, selected) {
            return Err(ConfigWorkspaceError::DuplicateName(config.name));
        }
        let entry = &mut self.entries[selected];
        entry.last_applied_document = default;
        entry.draft = None;
        Ok(Some(config))
    }

    fn unique_name(&self, base: &str) -> String {
        std::iter::once(base.to_string())
            .chain((2usize..).map(|suffix| format!("{base} {suffix}")))
            .find(|candidate| self.entries.iter().all(|entry| entry.name() != *candidate))
            .expect("suffix search should find a unique name")
    }

    fn name_exists_except(&self, name: &str, index: usize) -> bool {
        self.entries
            .iter()
            .enumerate()
            .any(|(entry_index, entry)| entry_index != index && entry.name() == name)
    }
}

impl ConfigEntry {
    fn name(&self) -> &str {
        self.last_applied_document
            .name()
            .expect("applied document should include metadata.name")
    }

    fn draft_text(&self) -> Cow<'_, str> {
        match &self.draft {
            Some(draft) => Cow::Borrowed(draft),
            None => Cow::Owned(self.applied_text()),
        }
    }

    fn draft_document(&self) -> Option<ConfigDocument> {
        match &self.draft {
            Some(draft) => ConfigDocument::parse(draft).ok(),
            None => Some(self.last_applied_document.clone()),
        }
    }

    fn applied_text(&self) -> String {
        self.last_applied_document.to_toml_string()
    }

    fn applied_config(&self) -> Config {
        self.last_applied_document
            .to_config()
            .expect("applied document should remain a valid config")
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
        let mut document = ConfigDocument::parse(text).unwrap();
        document.set_name(name);
        document.to_toml_string()
    }

    #[test]
    fn switching_entries_preserves_each_draft() {
        let first = config_text("First", "F", 60.0);
        let second = config_text("Second", "F+F", 90.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![first, second]).unwrap();

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
        let mut workspace = ConfigWorkspace::from_presets(vec![first]).unwrap();
        let previous_config = workspace.selected_applied_config();

        workspace.set_selected_draft_text("not valid toml".to_string());
        let error = workspace.apply_selected().unwrap_err();
        assert!(matches!(
            error,
            ConfigWorkspaceError::Config(ConfigError::TomlParse(_))
        ));

        assert_eq!(workspace.selected_applied_config(), previous_config);
        assert_eq!(
            workspace.selected_applied_document().to_config().unwrap(),
            previous_config
        );
        assert!(workspace.selected_is_dirty());
    }

    #[test]
    fn apply_rejects_parseable_toml_with_invalid_config() {
        let first = config_text("First", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![first.clone()]).unwrap();

        workspace.set_selected_draft_text(first.replace("axiom = \"F\"", "axiom = \"[\""));
        let error = workspace.apply_selected().unwrap_err();

        assert!(matches!(
            error,
            ConfigWorkspaceError::Config(ConfigError::UnmatchedOpen { .. })
        ));
        assert_eq!(workspace.selected_applied_config().generation.angle, 60.0);
        assert!(workspace.selected_is_dirty());
    }

    #[test]
    fn apply_unique_renamed_draft_updates_entry_name() {
        let first = config_text("First", "F", 60.0);
        let second = config_text("Second", "F+F", 90.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![first.clone(), second]).unwrap();

        workspace.set_selected_draft_text(config_text_renamed(&first, "Renamed"));
        let config = workspace.apply_selected().unwrap();

        assert_eq!(config.name, "Renamed");
        assert_eq!(workspace.selected_name(), "Renamed");
        assert_eq!(workspace.names().collect::<Vec<_>>(), ["Renamed", "Second"]);
        assert!(!workspace.selected_is_dirty());
    }

    #[test]
    fn apply_rejects_duplicate_renamed_draft() {
        let first = config_text("First", "F", 60.0);
        let second = config_text("Second", "F+F", 90.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![first.clone(), second]).unwrap();

        workspace.set_selected_draft_text(config_text_renamed(&first, "Second"));
        let error = workspace.apply_selected().unwrap_err();

        assert!(matches!(
            error,
            ConfigWorkspaceError::DuplicateName(ref name) if name == "Second"
        ));
        assert_eq!(workspace.selected_name(), "First");
        assert_eq!(workspace.selected_applied_text(), first);
        assert!(workspace.selected_is_dirty());
    }

    #[test]
    fn revert_restores_last_applied_document() {
        let first = config_text("First", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![first.clone()]).unwrap();

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
        let mut workspace = ConfigWorkspace::from_presets(vec![first.clone()]).unwrap();
        assert!(!workspace.selected_can_reset());

        workspace.set_selected_draft_text(first.replace("angle = 60", "angle = 45"));
        workspace.apply_selected().unwrap();
        assert_eq!(workspace.selected_applied_config().generation.angle, 45.0);
        assert!(workspace.selected_can_reset());

        let reset_config = workspace.reset_selected().unwrap().unwrap();

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
        let mut workspace =
            ConfigWorkspace::from_entries(vec![ConfigWorkspaceEntry::custom(first.clone())])
                .unwrap();

        assert!(!workspace.selected_can_reset());
        workspace.set_selected_draft_text(first.replace("angle = 60", "angle = 45"));
        workspace.apply_selected().unwrap();

        assert!(workspace.reset_selected().unwrap().is_none());
        assert_eq!(workspace.selected_applied_config().generation.angle, 45.0);
    }

    #[test]
    fn reset_rejects_default_name_collision() {
        let first = config_text("First", "F", 60.0);
        let second = config_text("Second", "F+F", 90.0);
        let mut workspace =
            ConfigWorkspace::from_presets(vec![first.clone(), second.clone()]).unwrap();

        workspace.set_selected_draft_text(config_text_renamed(&first, "Third"));
        workspace.apply_selected().unwrap();
        assert!(workspace.select_by_name("Second"));
        workspace.set_selected_draft_text(config_text_renamed(&second, "First"));
        workspace.apply_selected().unwrap();
        assert!(workspace.select_by_name("Third"));

        let error = workspace.reset_selected().unwrap_err();

        assert!(matches!(
            error,
            ConfigWorkspaceError::DuplicateName(ref name) if name == "First"
        ));
        assert!(!workspace.selected_can_reset());
        assert_eq!(workspace.selected_name(), "Third");
    }

    #[test]
    fn copy_selected_entry_preserves_dirty_valid_draft() {
        let first = config_text("Plant", "F", 60.0);
        let second = config_text("Plant copy", "F+F", 90.0);
        let draft = first.replace("angle = 60", "angle = 45");
        let mut workspace = ConfigWorkspace::from_presets(vec![first.clone(), second]).unwrap();
        workspace.set_selected_draft_text(draft.clone());

        workspace.copy_selected();
        let expected_text = config_text_renamed(&draft, "Plant copy 2");
        let expected_applied = config_text_renamed(&first, "Plant copy 2");

        assert_eq!(workspace.selected_name(), "Plant copy 2");
        assert_eq!(workspace.selected_draft_text(), expected_text);
        assert_eq!(workspace.selected_applied_text(), expected_applied);
        assert_eq!(workspace.selected_applied_config().name, "Plant copy 2");
        assert_eq!(workspace.selected_applied_config().generation.angle, 60.0);
        assert!(workspace.selected_is_dirty());
        assert!(!workspace.selected_can_reset());
    }

    #[test]
    fn copy_selected_entry_preserves_parseable_invalid_draft() {
        let first = config_text("Plant", "F", 60.0);
        let draft = first.replace("axiom = \"F\"", "axiom = \"[\"");
        let mut workspace = ConfigWorkspace::from_presets(vec![first.clone()]).unwrap();
        workspace.set_selected_draft_text(draft.clone());

        workspace.copy_selected();
        let expected_draft = config_text_renamed(&draft, "Plant copy");
        let expected_applied = config_text_renamed(&first, "Plant copy");

        assert_eq!(workspace.selected_name(), "Plant copy");
        assert_eq!(workspace.selected_draft_text(), expected_draft);
        assert_eq!(workspace.selected_applied_text(), expected_applied);
        assert_eq!(workspace.selected_applied_config().name, "Plant copy");
        assert_eq!(workspace.selected_applied_config().generation.angle, 60.0);
        assert!(workspace.selected_draft_document().is_some());
        assert!(matches!(
            workspace.apply_selected(),
            Err(ConfigWorkspaceError::Config(
                ConfigError::UnmatchedOpen { .. }
            ))
        ));
        assert!(workspace.selected_is_dirty());
        assert!(!workspace.selected_can_reset());
    }

    #[test]
    fn copy_selected_entry_preserves_unparseable_draft_text() {
        let first = config_text("Plant", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![first.clone()]).unwrap();
        workspace.set_selected_draft_text("not valid toml".to_string());

        workspace.copy_selected();
        let expected_applied = config_text_renamed(&first, "Plant copy");

        assert_eq!(workspace.selected_name(), "Plant copy");
        assert_eq!(workspace.selected_draft_text(), "not valid toml");
        assert_eq!(workspace.selected_applied_text(), expected_applied);
        assert_eq!(workspace.selected_applied_config().name, "Plant copy");
        assert_eq!(workspace.selected_applied_config().generation.angle, 60.0);
        assert!(workspace.selected_draft_document().is_none());
        assert!(workspace.selected_is_dirty());
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
        let error = ConfigWorkspace::from_presets(vec![first, second]).unwrap_err();

        assert!(matches!(
            error,
            ConfigWorkspaceError::DuplicateName(ref name) if name == "Duplicate"
        ));
    }

    #[test]
    fn from_entries_propagates_invalid_initial_text() {
        let error = ConfigWorkspace::from_entries(vec![ConfigWorkspaceEntry::custom(
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
    fn from_entries_propagates_parseable_invalid_default_config() {
        let text = config_text("Default", "F", 60.0);
        let error = ConfigWorkspace::from_entries(vec![ConfigWorkspaceEntry {
            text,
            default_text: Some("[metadata]\nname = \"Broken\"\n".to_string()),
        }])
        .unwrap_err();

        assert!(matches!(
            error,
            ConfigWorkspaceError::Config(ConfigError::MissingField(_))
        ));
    }

    #[test]
    fn fresh_workspace_is_not_dirty() {
        let first = config_text("First", "F", 60.0);
        let workspace = ConfigWorkspace::from_presets(vec![first]).unwrap();

        assert!(!workspace.selected_is_dirty());
    }

    #[test]
    fn draft_text_matching_applied_document_is_clean() {
        let first = config_text("First", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![first.clone()]).unwrap();

        workspace.set_selected_draft_text("temporary edit".to_string());
        assert!(workspace.selected_is_dirty());

        workspace.set_selected_draft_text(first);

        assert!(!workspace.selected_is_dirty());
    }

    #[test]
    fn select_index_returns_false_for_out_of_bounds() {
        let first = config_text("First", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![first]).unwrap();

        assert!(!workspace.select_index(1));
        assert_eq!(workspace.selected_index(), 0);
    }

    #[test]
    fn select_by_name_returns_false_for_unknown_name() {
        let first = config_text("First", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![first]).unwrap();

        assert!(!workspace.select_by_name("Missing"));
        assert_eq!(workspace.selected_name(), "First");
    }
}
