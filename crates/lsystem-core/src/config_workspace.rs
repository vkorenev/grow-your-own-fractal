use std::borrow::Cow;
use std::collections::BTreeSet;

use thiserror::Error;

use crate::{Config, ConfigDocument, ConfigError, ConfigSource};

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
pub struct ConfigEntry {
    default: Option<ConfigDocument>,
    draft: Option<String>,
    last_applied: ConfigDocument,
}

impl ConfigWorkspace {
    /// Build a workspace from a collection of `(label, text)` preset pairs.
    ///
    /// `label` is a display name used only in log warnings (typically the file path).
    /// Presets that fail to parse or fail config validation are skipped with a `log::warn!`
    /// naming the offending label — they do not cause an error. Returns
    /// [`ConfigWorkspaceError::Empty`] only if every preset is invalid (or the iterator is
    /// empty). Duplicate names among the valid presets still return
    /// [`ConfigWorkspaceError::DuplicateName`].
    pub fn from_presets<L: std::fmt::Display>(
        presets: impl IntoIterator<Item = (L, String)>,
    ) -> Result<Self, ConfigWorkspaceError> {
        let mut names = BTreeSet::new();
        let mut entries_out = Vec::new();

        for (label, text) in presets {
            let entry = match ConfigEntry::preset(text) {
                Ok(entry) => entry,
                Err(err) => {
                    log::warn!("Skipping invalid preset {label}: {err}");
                    continue;
                }
            };
            if !names.insert(entry.name().to_string()) {
                return Err(ConfigWorkspaceError::DuplicateName(
                    entry.name().to_string(),
                ));
            }
            entries_out.push(entry);
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

    pub fn entry_mut(&mut self, index: usize) -> Option<&mut ConfigEntry> {
        self.entries.get_mut(index)
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(ConfigEntry::name)
    }

    pub fn index_by_name(&self, name: &str) -> Option<usize> {
        self.entries.iter().position(|entry| entry.name() == name)
    }

    pub fn can_reset(&self, index: usize) -> bool {
        let Ok(entry) = self.entry_or_error(index) else {
            return false;
        };
        let Some(name) = entry.default_name() else {
            return false;
        };
        entry.has_default_changes() && !self.name_exists_except(name, index)
    }

    pub fn copy(&mut self, index: usize) -> Result<(usize, ConfigEntry), ConfigWorkspaceError> {
        let entry = self.entry_or_error(index)?;
        let name = self.unique_name(&format!("{} copy", entry.name()));
        let new_entry = entry.copy_as(&name)?;
        self.entries.push(new_entry.clone());
        Ok((self.entries.len() - 1, new_entry))
    }

    pub fn apply(&mut self, index: usize) -> Result<Config, ConfigWorkspaceError> {
        let entry = self.entry_or_error(index)?;
        let Some(applied) = entry.pending_apply()? else {
            return Ok(entry.applied_config());
        };
        if self.name_exists_except(&applied.config().name, index) {
            return Err(ConfigWorkspaceError::DuplicateName(
                applied.config().name.clone(),
            ));
        }
        let config = applied.config().clone();
        self.entries[index].commit_apply(applied);
        Ok(config)
    }

    /// Restores the entry at `index` to its bundled default document.
    /// Returns `Ok(None)` only when the entry has no bundled default (e.g. custom copies).
    /// Callers should gate user-visible resets on [`ConfigWorkspace::can_reset`] to avoid
    /// no-op resets when the applied document already matches the default.
    pub fn reset(&mut self, index: usize) -> Result<Option<ConfigEntry>, ConfigWorkspaceError> {
        let Some(default) = self.entry_or_error(index)?.reset_candidate() else {
            return Ok(None);
        };
        let name = default.name().to_string();
        if self.name_exists_except(&name, index) {
            return Err(ConfigWorkspaceError::DuplicateName(name));
        }
        self.entries[index].commit_reset(default);
        Ok(Some(self.entries[index].clone()))
    }

    fn entry_or_error(&self, index: usize) -> Result<&ConfigEntry, ConfigWorkspaceError> {
        self.entries
            .get(index)
            .ok_or(ConfigWorkspaceError::InvalidIndex(index))
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
    fn preset(text: String) -> Result<Self, ConfigWorkspaceError> {
        let source = ConfigSource::parse(&text)?;
        let doc = ConfigDocument::try_from(source)?;
        Ok(Self {
            default: Some(doc.clone()),
            draft: None,
            last_applied: doc,
        })
    }

    pub fn name(&self) -> &str {
        self.last_applied.name()
    }

    pub fn draft_text(&self) -> Cow<'_, str> {
        match &self.draft {
            Some(draft) => Cow::Borrowed(draft),
            None => Cow::Owned(self.applied_text()),
        }
    }

    fn applied_text(&self) -> String {
        self.last_applied.to_toml_string()
    }

    pub fn applied_config(&self) -> Config {
        self.last_applied.config().clone()
    }

    pub fn is_dirty(&self) -> bool {
        self.draft.is_some()
    }

    pub fn set_draft_text(&mut self, text: String) {
        let applied_text = self.applied_text();
        self.draft = (text != applied_text).then_some(text);
    }

    fn copy_as(&self, name: &str) -> Result<Self, ConfigWorkspaceError> {
        let draft = self.draft.as_ref().map(|draft_text| {
            ConfigSource::parse(draft_text).map_or_else(
                |_| draft_text.clone(), // draft is unparseable TOML; keep verbatim so the user can fix it
                |mut source| {
                    source.set_name(name);
                    source.to_toml_string()
                },
            )
        });

        let mut source = self.last_applied.source().clone();
        source.set_name(name);
        let last_applied = ConfigDocument::try_from(source)?;
        Ok(Self {
            default: None,
            draft,
            last_applied,
        })
    }

    fn pending_apply(&self) -> Result<Option<ConfigDocument>, ConfigWorkspaceError> {
        let Some(draft) = &self.draft else {
            return Ok(None);
        };
        let source = ConfigSource::parse(draft)?;
        let doc = ConfigDocument::try_from(source)?;
        Ok(Some(doc))
    }

    fn commit_apply(&mut self, applied: ConfigDocument) {
        self.last_applied = applied;
        self.draft = None;
    }

    pub fn revert(&mut self) {
        self.draft = None;
    }

    fn reset_candidate(&self) -> Option<ConfigDocument> {
        self.default.clone()
    }

    fn commit_reset(&mut self, default: ConfigDocument) {
        self.last_applied = default;
        self.draft = None;
    }

    fn default_name(&self) -> Option<&str> {
        self.default.as_ref().map(ConfigDocument::name)
    }

    fn has_default_changes(&self) -> bool {
        self.default
            .as_ref()
            .is_some_and(|default| self.applied_text() != default.to_toml_string())
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
        let mut source = ConfigSource::parse(text).unwrap();
        source.set_name(name);
        source.to_toml_string()
    }

    #[test]
    fn switching_entries_preserves_each_draft() {
        let first = config_text("First", "F", 60.0);
        let second = config_text("Second", "F+F", 90.0);
        let mut workspace =
            ConfigWorkspace::from_presets(vec![("First", first), ("Second", second)]).unwrap();

        workspace
            .entry_mut(0)
            .unwrap()
            .set_draft_text("edited first".to_string());
        workspace
            .entry_mut(1)
            .unwrap()
            .set_draft_text("edited second".to_string());

        assert_eq!(workspace.entry(0).unwrap().draft_text(), "edited first");
        assert!(ConfigSource::parse(workspace.entry(0).unwrap().draft_text().as_ref()).is_err());
        assert!(workspace.entry(0).unwrap().is_dirty());
        assert_eq!(workspace.entry(1).unwrap().draft_text(), "edited second");
    }

    #[test]
    fn failed_apply_preserves_last_applied_config() {
        let first = config_text("First", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("First", first)]).unwrap();
        let previous_config = workspace.entry(0).unwrap().applied_config();

        workspace
            .entry_mut(0)
            .unwrap()
            .set_draft_text("not valid toml".to_string());
        let error = workspace.apply(0).unwrap_err();
        assert!(matches!(
            error,
            ConfigWorkspaceError::Config(ConfigError::TomlParse(_))
        ));

        assert_eq!(
            workspace.entry(0).unwrap().applied_config(),
            previous_config
        );
        assert!(workspace.entry(0).unwrap().is_dirty());
    }

    #[test]
    fn apply_rejects_parseable_toml_with_invalid_config() {
        let first = config_text("First", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("First", first.clone())]).unwrap();

        workspace
            .entry_mut(0)
            .unwrap()
            .set_draft_text(first.replace("axiom = \"F\"", "axiom = \"[\""));
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
        let mut workspace =
            ConfigWorkspace::from_presets(vec![("First", first.clone()), ("Second", second)])
                .unwrap();

        workspace
            .entry_mut(0)
            .unwrap()
            .set_draft_text(config_text_renamed(&first, "Renamed"));
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
        let mut workspace =
            ConfigWorkspace::from_presets(vec![("First", first.clone()), ("Second", second)])
                .unwrap();

        workspace
            .entry_mut(0)
            .unwrap()
            .set_draft_text(config_text_renamed(&first, "Second"));
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
    fn revert_restores_last_applied() {
        let first = config_text("First", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("First", first.clone())]).unwrap();

        workspace
            .entry_mut(0)
            .unwrap()
            .set_draft_text(first.replace("angle = 60", "angle = 45"));
        workspace.apply(0).unwrap();
        let applied = workspace.entry(0).unwrap().draft_text().to_string();
        workspace
            .entry_mut(0)
            .unwrap()
            .set_draft_text("temporary invalid text".to_string());

        workspace.entry_mut(0).unwrap().revert();
        let reverted = workspace.entry(0).unwrap();

        assert!(!reverted.is_dirty());
        assert_eq!(reverted.draft_text(), applied.as_str());
        assert_eq!(
            reverted.draft_text(),
            workspace.entry(0).unwrap().draft_text()
        );
        let draft_document =
            ConfigSource::parse(workspace.entry(0).unwrap().draft_text().as_ref()).unwrap();
        assert_eq!(draft_document.to_string(), applied);
        assert!(!workspace.entry(0).unwrap().is_dirty());
    }

    #[test]
    fn reset_preset_restores_default_and_applies_it() {
        let first = config_text("First", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("First", first.clone())]).unwrap();
        assert!(!workspace.can_reset(0));

        workspace
            .entry_mut(0)
            .unwrap()
            .set_draft_text(first.replace("angle = 60", "angle = 45"));
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

        let reset_entry = workspace.reset(0).unwrap().unwrap();

        assert!(!reset_entry.is_dirty());
        assert_eq!(reset_entry.applied_config().generation.angle, 60.0);
        assert_eq!(
            reset_entry.draft_text(),
            workspace.entry(0).unwrap().draft_text()
        );
        assert_eq!(workspace.entry(0).unwrap().draft_text(), first);
        assert_eq!(workspace.entry(0).unwrap().applied_text(), first);
        assert!(!workspace.entry(0).unwrap().is_dirty());
        assert!(!workspace.can_reset(0));
    }

    #[test]
    fn custom_entry_has_no_default_to_reset() {
        let first = config_text("Custom", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("Custom", first.clone())]).unwrap();
        let (index, _) = workspace.copy(0).unwrap();

        assert!(!workspace.can_reset(index));
        let draft = workspace
            .entry(index)
            .unwrap()
            .draft_text()
            .replace("angle = 60", "angle = 45");
        workspace.entry_mut(index).unwrap().set_draft_text(draft);
        workspace.apply(index).unwrap();

        assert!(workspace.reset(index).unwrap().is_none());
        assert_eq!(
            workspace
                .entry(index)
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
        let mut workspace = ConfigWorkspace::from_presets(vec![
            ("First", first.clone()),
            ("Second", second.clone()),
        ])
        .unwrap();

        workspace
            .entry_mut(0)
            .unwrap()
            .set_draft_text(config_text_renamed(&first, "Third"));
        workspace.apply(0).unwrap();
        workspace
            .entry_mut(1)
            .unwrap()
            .set_draft_text(config_text_renamed(&second, "First"));
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
        let mut workspace =
            ConfigWorkspace::from_presets(vec![("Plant", first.clone()), ("Plant copy", second)])
                .unwrap();
        workspace
            .entry_mut(0)
            .unwrap()
            .set_draft_text(draft.clone());

        let (index, entry) = workspace.copy(0).unwrap();
        let expected_text = config_text_renamed(&draft, "Plant copy 2");
        let expected_applied = config_text_renamed(&first, "Plant copy 2");

        assert_eq!(
            entry.draft_text(),
            workspace.entry(index).unwrap().draft_text()
        );
        assert_eq!(entry.is_dirty(), workspace.entry(index).unwrap().is_dirty());
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
        let mut workspace = ConfigWorkspace::from_presets(vec![("Plant", first.clone())]).unwrap();
        workspace
            .entry_mut(0)
            .unwrap()
            .set_draft_text(draft.clone());

        let (index, entry) = workspace.copy(0).unwrap();
        let expected_draft = config_text_renamed(&draft, "Plant copy");
        let expected_applied = config_text_renamed(&first, "Plant copy");

        assert_eq!(
            entry.draft_text(),
            workspace.entry(index).unwrap().draft_text()
        );
        assert_eq!(entry.is_dirty(), workspace.entry(index).unwrap().is_dirty());
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
        assert!(ConfigSource::parse(workspace.entry(index).unwrap().draft_text().as_ref()).is_ok());
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
        let mut workspace = ConfigWorkspace::from_presets(vec![("Plant", first.clone())]).unwrap();
        workspace
            .entry_mut(0)
            .unwrap()
            .set_draft_text("not valid toml".to_string());

        let (index, entry) = workspace.copy(0).unwrap();
        let expected_applied = config_text_renamed(&first, "Plant copy");

        assert_eq!(
            entry.draft_text(),
            workspace.entry(index).unwrap().draft_text()
        );
        assert_eq!(entry.is_dirty(), workspace.entry(index).unwrap().is_dirty());
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
            ConfigSource::parse(workspace.entry(index).unwrap().draft_text().as_ref()).is_err()
        );
        assert!(workspace.entry(index).unwrap().is_dirty());
        assert!(!workspace.can_reset(index));
    }

    #[test]
    fn from_presets_rejects_empty_iterator() {
        let error = ConfigWorkspace::from_presets(Vec::<(&str, String)>::new()).unwrap_err();

        assert!(matches!(error, ConfigWorkspaceError::Empty));
    }

    #[test]
    fn from_presets_rejects_duplicate_names() {
        let first = config_text("Duplicate", "F", 60.0);
        let second = config_text("Duplicate", "F+F", 90.0);
        let error =
            ConfigWorkspace::from_presets(vec![("dup1", first), ("dup2", second)]).unwrap_err();

        assert!(matches!(
            error,
            ConfigWorkspaceError::DuplicateName(ref name) if name == "Duplicate"
        ));
    }

    #[test]
    fn from_presets_skips_invalid_text() {
        let error = ConfigWorkspace::from_presets(vec![("test", "not valid toml".to_string())])
            .unwrap_err();

        assert!(matches!(error, ConfigWorkspaceError::Empty));
    }

    #[test]
    fn from_presets_skips_invalid_presets_and_keeps_valid_ones() {
        let valid_a = config_text("First", "F", 60.0);
        let valid_b = config_text("Second", "F+F", 90.0);
        let invalid_toml = "not valid toml".to_string();
        let invalid_config =
            config_text("Bad", "F", 60.0).replace("axiom = \"F\"", "axiom = \"[\"");

        let workspace = ConfigWorkspace::from_presets(vec![
            ("first", valid_a),
            ("invalid-toml", invalid_toml),
            ("invalid-config", invalid_config),
            ("second", valid_b),
        ])
        .unwrap();

        assert_eq!(workspace.entries().len(), 2);
        assert_eq!(workspace.entry(0).unwrap().name(), "First");
        assert_eq!(workspace.entry(1).unwrap().name(), "Second");
    }

    #[test]
    fn fresh_workspace_is_not_dirty() {
        let first = config_text("First", "F", 60.0);
        let workspace = ConfigWorkspace::from_presets(vec![("First", first)]).unwrap();

        assert!(!workspace.entry(0).unwrap().is_dirty());
    }

    #[test]
    fn draft_text_matching_applied_config_is_clean() {
        let first = config_text("First", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("First", first.clone())]).unwrap();

        workspace
            .entry_mut(0)
            .unwrap()
            .set_draft_text("temporary edit".to_string());
        assert!(workspace.entry(0).unwrap().is_dirty());

        workspace.entry_mut(0).unwrap().set_draft_text(first);

        assert!(!workspace.entry(0).unwrap().is_dirty());
    }

    #[test]
    fn copy_returns_new_entry_index_without_workspace_selection() {
        let first = config_text("Plant", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("Plant", first)]).unwrap();

        let (index, entry) = workspace.copy(0).unwrap();

        assert_eq!(index, 1);
        assert_eq!(entry.name(), "Plant copy");
        assert_eq!(workspace.entry(0).unwrap().name(), "Plant");
        assert_eq!(workspace.entry(index).unwrap().name(), "Plant copy");
    }

    #[test]
    fn indexed_mutation_returns_none_for_out_of_bounds() {
        let first = config_text("First", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("First", first)]).unwrap();

        assert!(workspace.entry_mut(1).is_none());
    }

    #[test]
    fn index_by_name_returns_none_for_unknown_name() {
        let first = config_text("First", "F", 60.0);
        let workspace = ConfigWorkspace::from_presets(vec![("First", first)]).unwrap();

        assert_eq!(workspace.index_by_name("Missing"), None);
        assert_eq!(workspace.index_by_name("First"), Some(0));
    }
}
