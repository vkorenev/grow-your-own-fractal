use std::borrow::Cow;
use std::collections::BTreeSet;

use thiserror::Error;

use crate::{Config, ConfigDocument, ConfigError};

#[derive(Debug, Error)]
pub enum ConfigWorkspaceError {
    #[error("at least one config entry is required")]
    Empty,

    #[error("invalid config entry index `{0}`")]
    InvalidIndex(usize),

    #[error("duplicate config entry name `{0}`")]
    DuplicateName(String),

    #[error(transparent)]
    Config(#[from] ConfigError),
}

#[derive(Debug, Clone)]
pub struct ConfigWorkspace {
    entries: Vec<ConfigEntry>,
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
pub struct ConfigEntry {
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
        })
    }

    pub fn entries(&self) -> &[ConfigEntry] {
        &self.entries
    }

    pub fn entry(&self, index: usize) -> Option<&ConfigEntry> {
        self.entries.get(index)
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(ConfigEntry::name)
    }

    pub fn index_by_name(&self, name: &str) -> Option<usize> {
        self.entries.iter().position(|entry| entry.name() == name)
    }

    pub fn can_reset(&self, index: usize) -> bool {
        let Some(entry) = self.entries.get(index) else {
            return false;
        };
        let Some(default) = &entry.default else {
            return false;
        };
        let Some(name) = default.name() else {
            return false;
        };
        entry.applied_text() != default.to_toml_string() && !self.name_exists_except(name, index)
    }

    pub fn set_draft_text(
        &mut self,
        index: usize,
        text: String,
    ) -> Result<(), ConfigWorkspaceError> {
        let entry = self
            .entries
            .get_mut(index)
            .ok_or(ConfigWorkspaceError::InvalidIndex(index))?;
        let applied_text = entry.applied_text();
        entry.draft = (text != applied_text).then_some(text);
        Ok(())
    }

    pub fn copy(&mut self, index: usize) -> Result<(usize, Config), ConfigWorkspaceError> {
        let entry = self
            .entries
            .get(index)
            .ok_or(ConfigWorkspaceError::InvalidIndex(index))?;
        let name = self.unique_name(&format!("{} copy", entry.name()));
        let draft = entry.draft.as_ref().map(|draft_text| {
            ConfigDocument::parse(draft_text).map_or_else(
                |_| draft_text.clone(),
                |mut document| {
                    document.set_name(&name);
                    document.to_toml_string()
                },
            )
        });

        let mut last_applied_document = entry.last_applied_document.clone();
        last_applied_document.set_name(&name);
        let config = last_applied_document.to_config()?;
        self.entries.push(ConfigEntry {
            default: None,
            draft,
            last_applied_document,
        });
        Ok((self.entries.len() - 1, config))
    }

    pub fn apply(&mut self, index: usize) -> Result<Config, ConfigWorkspaceError> {
        let entry = self
            .entries
            .get(index)
            .ok_or(ConfigWorkspaceError::InvalidIndex(index))?;
        let Some(draft) = &entry.draft else {
            return Ok(entry.applied_config());
        };
        let document = ConfigDocument::parse(draft)?;
        let config = document.to_config()?;
        if self.name_exists_except(&config.name, index) {
            return Err(ConfigWorkspaceError::DuplicateName(config.name));
        }
        let entry = &mut self.entries[index];
        entry.last_applied_document = document;
        entry.draft = None;
        Ok(config)
    }

    pub fn revert(&mut self, index: usize) -> Result<String, ConfigWorkspaceError> {
        let entry = self
            .entries
            .get_mut(index)
            .ok_or(ConfigWorkspaceError::InvalidIndex(index))?;
        entry.draft = None;
        Ok(entry.applied_text())
    }

    pub fn reset(&mut self, index: usize) -> Result<Option<Config>, ConfigWorkspaceError> {
        let entry = self
            .entries
            .get(index)
            .ok_or(ConfigWorkspaceError::InvalidIndex(index))?;
        let Some(default) = entry.default.clone() else {
            return Ok(None);
        };
        let config = default.to_config()?;
        if self.name_exists_except(&config.name, index) {
            return Err(ConfigWorkspaceError::DuplicateName(config.name));
        }
        let entry = &mut self.entries[index];
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
    pub fn name(&self) -> &str {
        self.last_applied_document
            .name()
            .expect("applied document should include metadata.name")
    }

    pub fn draft_text(&self) -> Cow<'_, str> {
        match &self.draft {
            Some(draft) => Cow::Borrowed(draft),
            None => Cow::Owned(self.applied_text()),
        }
    }

    pub fn applied_text(&self) -> String {
        self.last_applied_document.to_toml_string()
    }

    pub fn applied_document(&self) -> &ConfigDocument {
        &self.last_applied_document
    }

    pub fn applied_config(&self) -> Config {
        self.last_applied_document
            .to_config()
            .expect("applied document should remain a valid config")
    }

    pub fn is_dirty(&self) -> bool {
        self.draft.is_some()
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

        workspace
            .set_draft_text(0, "edited first".to_string())
            .unwrap();
        workspace
            .set_draft_text(1, "edited second".to_string())
            .unwrap();

        assert_eq!(workspace.entry(0).unwrap().draft_text(), "edited first");
        assert!(ConfigDocument::parse(workspace.entry(0).unwrap().draft_text().as_ref()).is_err());
        assert!(workspace.entry(0).unwrap().is_dirty());
        assert_eq!(workspace.entry(1).unwrap().draft_text(), "edited second");
    }

    #[test]
    fn failed_apply_preserves_last_applied_config() {
        let first = config_text("First", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![first]).unwrap();
        let previous_config = workspace.entry(0).unwrap().applied_config();

        workspace
            .set_draft_text(0, "not valid toml".to_string())
            .unwrap();
        let error = workspace.apply(0).unwrap_err();
        assert!(matches!(
            error,
            ConfigWorkspaceError::Config(ConfigError::TomlParse(_))
        ));

        assert_eq!(
            workspace.entry(0).unwrap().applied_config(),
            previous_config
        );
        assert_eq!(
            workspace
                .entry(0)
                .unwrap()
                .applied_document()
                .to_config()
                .unwrap(),
            previous_config
        );
        assert!(workspace.entry(0).unwrap().is_dirty());
    }

    #[test]
    fn apply_rejects_parseable_toml_with_invalid_config() {
        let first = config_text("First", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![first.clone()]).unwrap();

        workspace
            .set_draft_text(0, first.replace("axiom = \"F\"", "axiom = \"[\""))
            .unwrap();
        let error = workspace.apply(0).unwrap_err();

        assert!(matches!(
            error,
            ConfigWorkspaceError::Config(ConfigError::UnmatchedOpen { .. })
        ));
        assert_eq!(
            workspace
                .entry(0)
                .unwrap()
                .applied_config()
                .generation
                .angle,
            60.0
        );
        assert!(workspace.entry(0).unwrap().is_dirty());
    }

    #[test]
    fn apply_unique_renamed_draft_updates_entry_name() {
        let first = config_text("First", "F", 60.0);
        let second = config_text("Second", "F+F", 90.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![first.clone(), second]).unwrap();

        workspace
            .set_draft_text(0, config_text_renamed(&first, "Renamed"))
            .unwrap();
        let config = workspace.apply(0).unwrap();

        assert_eq!(config.name, "Renamed");
        assert_eq!(workspace.entry(0).unwrap().name(), "Renamed");
        assert_eq!(workspace.names().collect::<Vec<_>>(), ["Renamed", "Second"]);
        assert!(!workspace.entry(0).unwrap().is_dirty());
    }

    #[test]
    fn apply_rejects_duplicate_renamed_draft() {
        let first = config_text("First", "F", 60.0);
        let second = config_text("Second", "F+F", 90.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![first.clone(), second]).unwrap();

        workspace
            .set_draft_text(0, config_text_renamed(&first, "Second"))
            .unwrap();
        let error = workspace.apply(0).unwrap_err();

        assert!(matches!(
            error,
            ConfigWorkspaceError::DuplicateName(ref name) if name == "Second"
        ));
        assert_eq!(workspace.entry(0).unwrap().name(), "First");
        assert_eq!(workspace.entry(0).unwrap().applied_text(), first);
        assert!(workspace.entry(0).unwrap().is_dirty());
    }

    #[test]
    fn revert_restores_last_applied_document() {
        let first = config_text("First", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![first.clone()]).unwrap();

        workspace
            .set_draft_text(0, first.replace("angle = 60", "angle = 45"))
            .unwrap();
        workspace.apply(0).unwrap();
        let applied = workspace.entry(0).unwrap().draft_text().to_string();
        workspace
            .set_draft_text(0, "temporary invalid text".to_string())
            .unwrap();

        let reverted = workspace.revert(0).unwrap();

        assert_eq!(reverted, applied);
        let draft_document =
            ConfigDocument::parse(workspace.entry(0).unwrap().draft_text().as_ref()).unwrap();
        assert_eq!(draft_document.to_string(), applied);
        assert!(!workspace.entry(0).unwrap().is_dirty());
    }

    #[test]
    fn reset_preset_restores_default_and_applies_it() {
        let first = config_text("First", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![first.clone()]).unwrap();
        assert!(!workspace.can_reset(0));

        workspace
            .set_draft_text(0, first.replace("angle = 60", "angle = 45"))
            .unwrap();
        workspace.apply(0).unwrap();
        assert_eq!(
            workspace
                .entry(0)
                .unwrap()
                .applied_config()
                .generation
                .angle,
            45.0
        );
        assert!(workspace.can_reset(0));

        let reset_config = workspace.reset(0).unwrap().unwrap();

        assert_eq!(reset_config.generation.angle, 60.0);
        assert_eq!(workspace.entry(0).unwrap().draft_text(), first);
        assert_eq!(workspace.entry(0).unwrap().applied_text(), first);
        assert_eq!(
            workspace.entry(0).unwrap().applied_document().to_string(),
            first
        );
        assert!(!workspace.entry(0).unwrap().is_dirty());
        assert!(!workspace.can_reset(0));
    }

    #[test]
    fn custom_entry_has_no_default_to_reset() {
        let first = config_text("Custom", "F", 60.0);
        let mut workspace =
            ConfigWorkspace::from_entries(vec![ConfigWorkspaceEntry::custom(first.clone())])
                .unwrap();

        assert!(!workspace.can_reset(0));
        workspace
            .set_draft_text(0, first.replace("angle = 60", "angle = 45"))
            .unwrap();
        workspace.apply(0).unwrap();

        assert!(workspace.reset(0).unwrap().is_none());
        assert_eq!(
            workspace
                .entry(0)
                .unwrap()
                .applied_config()
                .generation
                .angle,
            45.0
        );
    }

    #[test]
    fn reset_rejects_default_name_collision() {
        let first = config_text("First", "F", 60.0);
        let second = config_text("Second", "F+F", 90.0);
        let mut workspace =
            ConfigWorkspace::from_presets(vec![first.clone(), second.clone()]).unwrap();

        workspace
            .set_draft_text(0, config_text_renamed(&first, "Third"))
            .unwrap();
        workspace.apply(0).unwrap();
        workspace
            .set_draft_text(1, config_text_renamed(&second, "First"))
            .unwrap();
        workspace.apply(1).unwrap();

        let error = workspace.reset(0).unwrap_err();

        assert!(matches!(
            error,
            ConfigWorkspaceError::DuplicateName(ref name) if name == "First"
        ));
        assert!(!workspace.can_reset(0));
        assert_eq!(workspace.entry(0).unwrap().name(), "Third");
    }

    #[test]
    fn copy_entry_preserves_dirty_valid_draft() {
        let first = config_text("Plant", "F", 60.0);
        let second = config_text("Plant copy", "F+F", 90.0);
        let draft = first.replace("angle = 60", "angle = 45");
        let mut workspace = ConfigWorkspace::from_presets(vec![first.clone(), second]).unwrap();
        workspace.set_draft_text(0, draft.clone()).unwrap();

        let (index, _) = workspace.copy(0).unwrap();
        let expected_text = config_text_renamed(&draft, "Plant copy 2");
        let expected_applied = config_text_renamed(&first, "Plant copy 2");

        assert_eq!(workspace.entry(index).unwrap().name(), "Plant copy 2");
        assert_eq!(workspace.entry(index).unwrap().draft_text(), expected_text);
        assert_eq!(
            workspace.entry(index).unwrap().applied_text(),
            expected_applied
        );
        assert_eq!(
            workspace.entry(index).unwrap().applied_config().name,
            "Plant copy 2"
        );
        assert_eq!(
            workspace
                .entry(index)
                .unwrap()
                .applied_config()
                .generation
                .angle,
            60.0
        );
        assert!(workspace.entry(index).unwrap().is_dirty());
        assert!(!workspace.can_reset(index));
    }

    #[test]
    fn copy_entry_preserves_parseable_invalid_draft() {
        let first = config_text("Plant", "F", 60.0);
        let draft = first.replace("axiom = \"F\"", "axiom = \"[\"");
        let mut workspace = ConfigWorkspace::from_presets(vec![first.clone()]).unwrap();
        workspace.set_draft_text(0, draft.clone()).unwrap();

        let (index, _) = workspace.copy(0).unwrap();
        let expected_draft = config_text_renamed(&draft, "Plant copy");
        let expected_applied = config_text_renamed(&first, "Plant copy");

        assert_eq!(workspace.entry(index).unwrap().name(), "Plant copy");
        assert_eq!(workspace.entry(index).unwrap().draft_text(), expected_draft);
        assert_eq!(
            workspace.entry(index).unwrap().applied_text(),
            expected_applied
        );
        assert_eq!(
            workspace.entry(index).unwrap().applied_config().name,
            "Plant copy"
        );
        assert_eq!(
            workspace
                .entry(index)
                .unwrap()
                .applied_config()
                .generation
                .angle,
            60.0
        );
        assert!(
            ConfigDocument::parse(workspace.entry(index).unwrap().draft_text().as_ref()).is_ok()
        );
        assert!(matches!(
            workspace.apply(index),
            Err(ConfigWorkspaceError::Config(
                ConfigError::UnmatchedOpen { .. }
            ))
        ));
        assert!(workspace.entry(index).unwrap().is_dirty());
        assert!(!workspace.can_reset(index));
    }

    #[test]
    fn copy_entry_preserves_unparseable_draft_text() {
        let first = config_text("Plant", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![first.clone()]).unwrap();
        workspace
            .set_draft_text(0, "not valid toml".to_string())
            .unwrap();

        let (index, _) = workspace.copy(0).unwrap();
        let expected_applied = config_text_renamed(&first, "Plant copy");

        assert_eq!(workspace.entry(index).unwrap().name(), "Plant copy");
        assert_eq!(
            workspace.entry(index).unwrap().draft_text(),
            "not valid toml"
        );
        assert_eq!(
            workspace.entry(index).unwrap().applied_text(),
            expected_applied
        );
        assert_eq!(
            workspace.entry(index).unwrap().applied_config().name,
            "Plant copy"
        );
        assert_eq!(
            workspace
                .entry(index)
                .unwrap()
                .applied_config()
                .generation
                .angle,
            60.0
        );
        assert!(
            ConfigDocument::parse(workspace.entry(index).unwrap().draft_text().as_ref()).is_err()
        );
        assert!(workspace.entry(index).unwrap().is_dirty());
        assert!(!workspace.can_reset(index));
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

        assert!(!workspace.entry(0).unwrap().is_dirty());
    }

    #[test]
    fn draft_text_matching_applied_document_is_clean() {
        let first = config_text("First", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![first.clone()]).unwrap();

        workspace
            .set_draft_text(0, "temporary edit".to_string())
            .unwrap();
        assert!(workspace.entry(0).unwrap().is_dirty());

        workspace.set_draft_text(0, first).unwrap();

        assert!(!workspace.entry(0).unwrap().is_dirty());
    }

    #[test]
    fn copy_returns_new_entry_index_without_workspace_selection() {
        let first = config_text("Plant", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![first]).unwrap();

        let (index, config) = workspace.copy(0).unwrap();

        assert_eq!(index, 1);
        assert_eq!(config.name, "Plant copy");
        assert_eq!(workspace.entry(0).unwrap().name(), "Plant");
        assert_eq!(workspace.entry(index).unwrap().name(), "Plant copy");
    }

    #[test]
    fn indexed_mutation_returns_error_for_out_of_bounds() {
        let first = config_text("First", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![first]).unwrap();

        let error = workspace
            .set_draft_text(1, "edited".to_string())
            .unwrap_err();

        assert!(matches!(error, ConfigWorkspaceError::InvalidIndex(1)));
    }

    #[test]
    fn index_by_name_returns_none_for_unknown_name() {
        let first = config_text("First", "F", 60.0);
        let workspace = ConfigWorkspace::from_presets(vec![first]).unwrap();

        assert_eq!(workspace.index_by_name("Missing"), None);
        assert_eq!(workspace.index_by_name("First"), Some(0));
    }
}
