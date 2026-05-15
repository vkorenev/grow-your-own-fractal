use thiserror::Error;

use crate::{Config, ConfigDocument, ConfigError};

#[derive(Debug, Error)]
pub enum ConfigWorkspaceError {
    #[error("at least one config entry is required")]
    Empty,

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
    default_text: Option<String>,
    default_document: Option<ConfigDocument>,
    draft_text: String,
    draft_document: Option<ConfigDocument>,
    last_applied_text: String,
    last_applied_document: ConfigDocument,
    last_applied_config: Config,
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
        let entries: Result<Vec<_>, ConfigWorkspaceError> = entries
            .into_iter()
            .map(|entry| {
                let ConfigWorkspaceEntry {
                    name,
                    text,
                    default_text,
                } = entry;
                let document = ConfigDocument::parse(&text)?;
                let config = document.to_config()?;
                let default_document = default_text
                    .as_deref()
                    .map(ConfigDocument::parse)
                    .transpose()?;
                Ok(ConfigEntry {
                    name,
                    default_text,
                    default_document,
                    draft_text: text.clone(),
                    draft_document: Some(document.clone()),
                    last_applied_text: text,
                    last_applied_document: document,
                    last_applied_config: config,
                })
            })
            .collect();
        let entries = entries?;
        if entries.is_empty() {
            return Err(ConfigWorkspaceError::Empty);
        }
        Ok(Self {
            entries,
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
        self.entries[self.selected].default_text.is_some()
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
        entry.draft_document = ConfigDocument::parse(&text).ok();
        entry.draft_text = text;
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

    pub fn reset_selected(&mut self) -> Result<Option<&Config>, ConfigError> {
        let entry = &mut self.entries[self.selected];
        let Some(default_text) = entry.default_text.clone() else {
            return Ok(None);
        };
        let document = entry.default_document.clone().unwrap_or_else(|| {
            ConfigDocument::parse(&default_text).expect("stored default parses")
        });
        let config = document.to_config()?;
        entry.draft_text = default_text.clone();
        entry.draft_document = Some(document.clone());
        entry.last_applied_text = default_text;
        entry.last_applied_document = document;
        entry.last_applied_config = config;
        Ok(Some(&entry.last_applied_config))
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
        assert!(workspace.apply_selected().is_err());

        assert_eq!(workspace.selected_applied_config(), &previous_config);
        assert_eq!(
            workspace.selected_applied_document().to_config().unwrap(),
            previous_config
        );
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
        workspace.set_selected_draft_text(first.replace("angle = 60", "angle = 45"));
        workspace.apply_selected().unwrap();
        assert_eq!(workspace.selected_applied_config().generation.angle, 45.0);

        let reset_config = workspace.reset_selected().unwrap().unwrap();

        assert_eq!(reset_config.generation.angle, 60.0);
        assert_eq!(workspace.selected_draft_text(), first);
        assert_eq!(workspace.selected_applied_text(), first);
        assert_eq!(workspace.selected_applied_document().to_string(), first);
        assert!(!workspace.selected_is_dirty());
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

        assert!(workspace.reset_selected().unwrap().is_none());
        assert_eq!(workspace.selected_applied_config().generation.angle, 45.0);
    }
}
