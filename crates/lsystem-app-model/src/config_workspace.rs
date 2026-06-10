use std::borrow::Cow;
use std::collections::BTreeSet;

use thiserror::Error;

use lsystem_core::{Dimensions, Rgb};

use crate::config_defaults::ParseConfigError;
use crate::editor_config::{ConfigDocument, ConfigSource, EditorConfig, EditorLineColorConfig};

#[derive(Debug, Error)]
pub enum ConfigWorkspaceError {
    #[error("at least one config entry is required")]
    Empty,

    #[error("invalid config entry index `{0}`")]
    InvalidIndex(usize),

    #[error("duplicate config entry name `{0}`")]
    DuplicateName(String),

    #[error(transparent)]
    ParseConfig(#[from] ParseConfigError),
}

#[derive(Debug, Clone)]
pub struct ConfigWorkspace {
    entries: Vec<ConfigEntry>,
    // INVARIANT: `selected < entries.len()`. Upheld by `from_presets` (rejects empty),
    // `select` (bounds-checks), and `copy` (assigns `len() - 1` after a push). Any future
    // method that removes or reorders entries must re-anchor `selected` to preserve it,
    // because `selected()`/`selected_mut()` index into `entries` directly.
    selected: usize,
}

#[derive(Debug, Clone)]
pub struct ConfigEntry {
    default: Option<ConfigDocument>,
    draft: Option<String>,
    last_applied: ConfigDocument,
}

/// Typed view over a `ConfigEntry` whose variant reflects whether the entry has a draft.
///
/// Operations that are only valid on a clean entry (such as `set_iterations` /
/// `set_angle`, which mutate the applied document) live on [`CleanMut`] and are therefore
/// unreachable on a dirty entry at compile time.
pub enum EntryViewMut<'a> {
    Clean(CleanMut<'a>),
    Dirty(DirtyMut<'a>),
}

pub struct CleanMut<'a>(&'a mut ConfigEntry);
pub struct DirtyMut<'a>(&'a mut ConfigEntry);

impl ConfigWorkspace {
    /// Build a workspace from a collection of `(label, text)` preset pairs.
    ///
    /// `label` is a display name used only in log warnings (typically the file path).
    /// Presets that fail to parse or fail config validation are skipped with a `log::warn!`
    /// naming the offending label — they do not cause an error. Returns
    /// [`ConfigWorkspaceError::Empty`] only if every preset is invalid (or the iterator is
    /// empty). Duplicate names among the valid presets still return
    /// [`ConfigWorkspaceError::DuplicateName`].
    ///
    /// The returned workspace has its selection pointed at the first entry.
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
            selected: 0,
        })
    }

    pub fn entries(&self) -> &[ConfigEntry] {
        &self.entries
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(ConfigEntry::name)
    }

    pub fn index_by_name(&self, name: &str) -> Option<usize> {
        self.entries.iter().position(|entry| entry.name() == name)
    }

    pub fn selected_index(&self) -> usize {
        self.selected
    }

    pub fn selected(&self) -> &ConfigEntry {
        &self.entries[self.selected]
    }

    pub fn selected_mut(&mut self) -> &mut ConfigEntry {
        &mut self.entries[self.selected]
    }

    pub fn select(&mut self, index: usize) -> Result<(), ConfigWorkspaceError> {
        if index >= self.entries.len() {
            return Err(ConfigWorkspaceError::InvalidIndex(index));
        }
        self.selected = index;
        Ok(())
    }

    pub fn can_reset(&self) -> bool {
        let entry = self.selected();
        let Some(name) = entry.default_name() else {
            return false;
        };
        entry.has_default_changes() && !self.name_exists_except(name, self.selected)
    }

    /// Creates a renamed copy of the selected entry, auto-selects the new entry, and
    /// returns a borrow of it. After the borrow drops, the same entry remains accessible
    /// via [`ConfigWorkspace::selected`].
    pub fn copy(&mut self) -> Result<&ConfigEntry, ConfigWorkspaceError> {
        let new_entry = {
            let entry = self.selected();
            let name = self.unique_name(&format!("{} copy", entry.name()));
            entry.copy_as(&name)?
        };
        self.entries.push(new_entry);
        self.selected = self.entries.len() - 1;
        Ok(&self.entries[self.selected])
    }

    pub fn rename(&mut self, index: usize, new_name: &str) -> Result<(), ConfigWorkspaceError> {
        if index >= self.entries.len() {
            return Err(ConfigWorkspaceError::InvalidIndex(index));
        }
        if self.name_exists_except(new_name, index) {
            return Err(ConfigWorkspaceError::DuplicateName(new_name.to_string()));
        }
        self.entries[index].rename_in_place(new_name)?;
        Ok(())
    }

    /// Validates the selected entry's draft text and commits it as the new applied
    /// document. Returns a borrow of the selected entry on success; the returned entry
    /// is always clean (`is_dirty() == false`). If the entry has no pending draft, this
    /// is a no-op and returns the unchanged entry. Returns
    /// [`ConfigWorkspaceError::ParseConfig`] on parse or validation failure and
    /// [`ConfigWorkspaceError::DuplicateName`] if the draft renames the entry to a name
    /// already used elsewhere in the workspace. Failure leaves workspace state untouched.
    pub fn apply(&mut self) -> Result<&ConfigEntry, ConfigWorkspaceError> {
        let idx = self.selected;
        let Some(applied) = self.entries[idx].pending_apply()? else {
            return Ok(&self.entries[idx]);
        };
        if self.name_exists_except(applied.name(), idx) {
            return Err(ConfigWorkspaceError::DuplicateName(
                applied.name().to_string(),
            ));
        }
        self.entries[idx].commit_apply(applied);
        Ok(&self.entries[idx])
    }

    /// Restores the selected entry to its bundled default document. On `Ok(Some(_))`
    /// the returned entry is clean and its applied text equals the bundled default.
    /// Returns `Ok(None)` only when the entry has no bundled default (e.g. custom copies).
    /// Callers should gate user-visible resets on [`ConfigWorkspace::can_reset`] to avoid
    /// no-op resets when the applied document already matches the default.
    pub fn reset(&mut self) -> Result<Option<&ConfigEntry>, ConfigWorkspaceError> {
        let Some(default) = self.selected().reset_candidate() else {
            return Ok(None);
        };
        let name = default.name().to_string();
        if self.name_exists_except(&name, self.selected) {
            return Err(ConfigWorkspaceError::DuplicateName(name));
        }
        let idx = self.selected;
        self.entries[idx].commit_reset(default);
        Ok(Some(&self.entries[idx]))
    }

    /// Parses and validates `text` as a config document, creates a new custom entry from it
    /// (no bundled default), auto-selects the new entry, and returns its index.
    ///
    /// If the parsed name collides with an existing entry name, ` 2`, ` 3`, … is appended
    /// until the name is unique. Returns [`ConfigWorkspaceError::ParseConfig`] if `text` fails
    /// to parse or validate. On error the workspace state is unchanged: no entry is added and
    /// the selection is not moved.
    pub fn import_toml(&mut self, text: &str) -> Result<usize, ConfigWorkspaceError> {
        let source = ConfigSource::parse(text)?;
        let doc = ConfigDocument::try_from(source)?;
        let base_name = doc.name().to_string();
        let unique = self.unique_name(&base_name);
        let doc = if unique != base_name {
            let mut source = doc.source().clone();
            source.set_name(&unique);
            ConfigDocument::try_from(source)?
        } else {
            doc
        };
        self.entries.push(ConfigEntry::custom(doc));
        self.selected = self.entries.len() - 1;
        Ok(self.selected)
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

    fn custom(last_applied: ConfigDocument) -> Self {
        Self {
            default: None,
            draft: None,
            last_applied,
        }
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

    pub fn editor_config(&self) -> &EditorConfig {
        self.last_applied.editor_config()
    }

    pub fn is_dirty(&self) -> bool {
        self.draft.is_some()
    }

    pub fn set_draft_text(&mut self, text: String) {
        let applied_text = self.applied_text();
        self.draft = (text != applied_text).then_some(text);
    }

    pub fn view_mut(&mut self) -> EntryViewMut<'_> {
        if self.draft.is_some() {
            EntryViewMut::Dirty(DirtyMut(self))
        } else {
            EntryViewMut::Clean(CleanMut(self))
        }
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

    fn reset_candidate(&self) -> Option<ConfigDocument> {
        self.default.clone()
    }

    fn commit_reset(&mut self, default: ConfigDocument) {
        self.last_applied = default;
        self.draft = None;
    }

    /// Mutates the applied TOML source via `update`. Only reachable through [`CleanMut`],
    /// so the entry is guaranteed not to have a pending draft when this runs.
    fn update_last_applied_source(
        &mut self,
        update: impl FnOnce(&mut ConfigSource),
    ) -> Result<(), ParseConfigError> {
        debug_assert!(self.draft.is_none(), "clean view should imply no draft");
        let mut source = self.last_applied.source().clone();
        update(&mut source);
        self.last_applied = ConfigDocument::try_from(source)?;
        Ok(())
    }

    fn rename_in_place(&mut self, new_name: &str) -> Result<(), ParseConfigError> {
        let mut source = self.last_applied.source().clone();
        source.set_name(new_name);
        self.last_applied = ConfigDocument::try_from(source)?;
        // Update the draft name too so applying the draft later does not silently
        // revert the rename. If the draft is unparseable TOML, leave it verbatim —
        // the apply path will surface the parse error to the user.
        if let Some(draft_text) = &self.draft
            && let Ok(mut draft_source) = ConfigSource::parse(draft_text)
        {
            draft_source.set_name(new_name);
            self.draft = Some(draft_source.to_toml_string());
        }
        Ok(())
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

impl CleanMut<'_> {
    pub fn set_iterations(&mut self, iterations: u32) -> Result<(), ParseConfigError> {
        self.0
            .update_last_applied_source(|source| source.set_iterations(iterations))
    }

    pub fn set_angle(&mut self, angle: f32) -> Result<(), ParseConfigError> {
        self.0
            .update_last_applied_source(|source| source.set_angle(angle))
    }

    pub fn set_initial_heading(&mut self, initial_heading: f32) -> Result<(), ParseConfigError> {
        self.0
            .update_last_applied_source(|source| source.set_initial_heading(initial_heading))
    }

    pub fn set_dimensions(&mut self, dimensions: Dimensions) -> Result<(), ParseConfigError> {
        self.0
            .update_last_applied_source(|source| source.set_dimensions(dimensions))
    }

    pub fn set_grammar(
        &mut self,
        axiom: &str,
        rules: &[(char, String)],
    ) -> Result<(), ParseConfigError> {
        self.0
            .update_last_applied_source(|source| source.set_grammar(axiom, rules))
    }

    pub fn set_background(&mut self, background: Option<Rgb>) -> Result<(), ParseConfigError> {
        self.0
            .update_last_applied_source(|source| source.set_background(background))
    }

    pub fn set_line_color(
        &mut self,
        line_color: Option<EditorLineColorConfig>,
    ) -> Result<(), ParseConfigError> {
        self.0
            .update_last_applied_source(|source| source.set_line_color(line_color.as_ref()))
    }
}

impl DirtyMut<'_> {
    /// Drops the pending draft, transitioning the entry back to the clean state.
    /// Consumes the view so the type system reflects that the entry is no longer dirty.
    pub fn revert(self) {
        self.0.draft = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use lsystem_core::{ConfigError, LineColorConfig};

    use crate::config_defaults::ConfigDefaults;

    fn config_text(name: &str, axiom: &str, angle: f32) -> String {
        format!(
            r##"[metadata]
name = "{name}"

[l-system]
dimensions = "2D"
axiom = "{axiom}"
iterations = 1
angle = {angle}
step = 1.0
initial_heading = 0.0

[l-system.rules]
F = "FF"

[colors]
background = "#000000"

[colors.line]
solid = "#00e680"
"##
        )
    }

    fn config_text_renamed(text: &str, name: &str) -> String {
        let mut source = ConfigSource::parse(text).unwrap();
        source.set_name(name);
        source.to_toml_string()
    }

    fn dotted_config_text() -> String {
        r##"metadata.name = "Dotted"
l-system.dimensions = "2D"
l-system.axiom = "F"
l-system.iterations = 1
l-system.angle = 60.0
l-system.step = 1.0
l-system.initial_heading = 0.0
l-system.rules.F = "FF"
colors.background = "#000000"
colors.line.solid = "#00e680"
"##
        .to_string()
    }

    fn decorated_config_text() -> String {
        r##"[metadata]
name = "Decorated"

[l-system]
dimensions = "2D"
axiom = "F"
iterations = 1 # keep iterations comment
angle = 60.0 # keep angle comment
step = 1.0
initial_heading = 0.0

[l-system.rules]
F = "FF"

[colors]
background = "#000000"

[colors.line]
solid = "#00e680"
"##
        .to_string()
    }

    fn clean_mut<'a>(workspace: &'a mut ConfigWorkspace) -> CleanMut<'a> {
        match workspace.selected_mut().view_mut() {
            EntryViewMut::Clean(clean) => clean,
            EntryViewMut::Dirty(_) => panic!("expected clean entry"),
        }
    }

    fn revert_selected(workspace: &mut ConfigWorkspace) {
        match workspace.selected_mut().view_mut() {
            EntryViewMut::Dirty(dirty) => dirty.revert(),
            EntryViewMut::Clean(_) => panic!("expected dirty entry"),
        }
    }

    fn runtime_config(entry: &ConfigEntry) -> lsystem_core::Config {
        entry
            .editor_config()
            .resolve(ConfigDefaults::embedded(), u32::MAX)
    }

    #[test]
    fn switching_entries_preserves_each_draft() {
        let first = config_text("First", "F", 60.0);
        let second = config_text("Second", "F+F", 90.0);
        let mut workspace =
            ConfigWorkspace::from_presets(vec![("First", first), ("Second", second)]).unwrap();

        workspace.select(0).unwrap();
        workspace
            .selected_mut()
            .set_draft_text("edited first".to_string());
        workspace.select(1).unwrap();
        workspace
            .selected_mut()
            .set_draft_text("edited second".to_string());

        workspace.select(0).unwrap();
        assert_eq!(workspace.selected().draft_text(), "edited first");
        assert!(ConfigSource::parse(workspace.selected().draft_text().as_ref()).is_err());
        assert!(workspace.selected().is_dirty());
        workspace.select(1).unwrap();
        assert_eq!(workspace.selected().draft_text(), "edited second");
    }

    #[test]
    fn failed_apply_preserves_last_runtime_config() {
        let first = config_text("First", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("First", first)]).unwrap();
        let previous_config = runtime_config(workspace.selected()).clone();

        workspace
            .selected_mut()
            .set_draft_text("not valid toml".to_string());
        let error = workspace.apply().unwrap_err();
        assert!(matches!(
            error,
            ConfigWorkspaceError::ParseConfig(ParseConfigError::TomlParse(_))
        ));

        assert_eq!(runtime_config(workspace.selected()), previous_config);
        assert!(workspace.selected().is_dirty());
    }

    #[test]
    fn apply_rejects_parseable_toml_with_invalid_config() {
        let first = config_text("First", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("First", first.clone())]).unwrap();

        workspace
            .selected_mut()
            .set_draft_text(first.replace("axiom = \"F\"", "axiom = \"[\""));
        let error = workspace.apply().unwrap_err();

        assert!(matches!(
            error,
            ConfigWorkspaceError::ParseConfig(ParseConfigError::Validation(
                ConfigError::UnmatchedOpen { .. }
            ))
        ));
        assert_eq!(workspace.selected().editor_config().generation.angle, 60.0);
        assert!(workspace.selected().is_dirty());
    }

    #[test]
    fn apply_unique_renamed_draft_updates_entry_name() {
        let first = config_text("First", "F", 60.0);
        let second = config_text("Second", "F+F", 90.0);
        let mut workspace =
            ConfigWorkspace::from_presets(vec![("First", first.clone()), ("Second", second)])
                .unwrap();

        workspace
            .selected_mut()
            .set_draft_text(config_text_renamed(&first, "Renamed"));
        let entry = workspace.apply().unwrap();

        assert_eq!(entry.name(), "Renamed");
        let entry_ptr = entry as *const _;
        assert!(std::ptr::eq(entry_ptr, workspace.selected()));
        assert_eq!(workspace.names().collect::<Vec<_>>(), ["Renamed", "Second"]);
        assert!(!workspace.selected().is_dirty());
    }

    #[test]
    fn apply_on_clean_entry_returns_selected_entry_unchanged() {
        let first = config_text("First", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("First", first.clone())]).unwrap();

        let entry = workspace.apply().unwrap();

        assert_eq!(entry.name(), "First");
        assert_eq!(entry.applied_text(), first);
        assert!(!entry.is_dirty());
        let entry_ptr = entry as *const _;
        assert!(std::ptr::eq(entry_ptr, workspace.selected()));
    }

    #[test]
    fn apply_rejects_duplicate_renamed_draft() {
        let first = config_text("First", "F", 60.0);
        let second = config_text("Second", "F+F", 90.0);
        let mut workspace =
            ConfigWorkspace::from_presets(vec![("First", first.clone()), ("Second", second)])
                .unwrap();

        workspace
            .selected_mut()
            .set_draft_text(config_text_renamed(&first, "Second"));
        let error = workspace.apply().unwrap_err();

        assert!(matches!(
            error,
            ConfigWorkspaceError::DuplicateName(ref name) if name == "Second"
        ));
        assert_eq!(workspace.selected().name(), "First");
        assert_eq!(workspace.selected().applied_text(), first);
        assert!(workspace.selected().is_dirty());
    }

    #[test]
    fn revert_restores_last_applied() {
        let first = config_text("First", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("First", first.clone())]).unwrap();

        workspace
            .selected_mut()
            .set_draft_text(first.replace("angle = 60", "angle = 45"));
        workspace.apply().unwrap();
        let applied = workspace.selected().draft_text().into_owned();
        workspace
            .selected_mut()
            .set_draft_text("temporary invalid text".to_string());

        revert_selected(&mut workspace);
        let reverted = workspace.selected();

        assert!(!reverted.is_dirty());
        assert_eq!(reverted.draft_text(), applied.as_str());
        let draft_document = ConfigSource::parse(reverted.draft_text().as_ref()).unwrap();
        assert_eq!(draft_document.to_string(), applied);
    }

    #[test]
    fn reset_preset_restores_default_and_applies_it() {
        let first = config_text("First", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("First", first.clone())]).unwrap();
        assert!(!workspace.can_reset());

        workspace
            .selected_mut()
            .set_draft_text(first.replace("angle = 60", "angle = 45"));
        workspace.apply().unwrap();
        assert_eq!(workspace.selected().editor_config().generation.angle, 45.0);
        assert!(workspace.can_reset());

        let reset_entry = workspace
            .reset()
            .unwrap()
            .expect("expected reset to apply default");
        assert!(!reset_entry.is_dirty());
        assert_eq!(reset_entry.editor_config().generation.angle, 60.0);
        assert_eq!(reset_entry.draft_text(), first);
        assert_eq!(reset_entry.applied_text(), first);
        let reset_entry_ptr = reset_entry as *const _;
        assert!(std::ptr::eq(reset_entry_ptr, workspace.selected()));
        assert!(!workspace.can_reset());
    }

    #[test]
    fn custom_entry_has_no_default_to_reset() {
        let first = config_text("Custom", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("Custom", first.clone())]).unwrap();
        workspace.copy().unwrap();

        assert!(!workspace.can_reset());
        let draft = workspace
            .selected()
            .draft_text()
            .replace("angle = 60", "angle = 45");
        workspace.selected_mut().set_draft_text(draft);
        workspace.apply().unwrap();

        assert!(workspace.reset().unwrap().is_none());
        assert_eq!(workspace.selected().editor_config().generation.angle, 45.0);
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

        workspace.select(0).unwrap();
        workspace
            .selected_mut()
            .set_draft_text(config_text_renamed(&first, "Third"));
        workspace.apply().unwrap();
        workspace.select(1).unwrap();
        workspace
            .selected_mut()
            .set_draft_text(config_text_renamed(&second, "First"));
        workspace.apply().unwrap();

        workspace.select(0).unwrap();
        let error = workspace.reset().unwrap_err();

        assert!(matches!(
            error,
            ConfigWorkspaceError::DuplicateName(ref name) if name == "First"
        ));
        assert!(!workspace.can_reset());
        assert_eq!(workspace.selected().name(), "Third");
    }

    #[test]
    fn copy_entry_preserves_dirty_valid_draft() {
        let first = config_text("Plant", "F", 60.0);
        let second = config_text("Plant copy", "F+F", 90.0);
        let draft = first.replace("angle = 60", "angle = 45");
        let mut workspace =
            ConfigWorkspace::from_presets(vec![("Plant", first.clone()), ("Plant copy", second)])
                .unwrap();
        workspace.select(0).unwrap();
        workspace.selected_mut().set_draft_text(draft.clone());

        let entry = workspace.copy().unwrap();
        let expected_text = config_text_renamed(&draft, "Plant copy 2");
        let expected_applied = config_text_renamed(&first, "Plant copy 2");

        assert_eq!(entry.name(), "Plant copy 2");
        assert_eq!(entry.draft_text(), expected_text);
        assert!(entry.is_dirty());
        assert_eq!(workspace.selected().name(), "Plant copy 2");
        assert_eq!(workspace.selected().draft_text(), expected_text);
        assert_eq!(workspace.selected().applied_text(), expected_applied);
        assert_eq!(workspace.selected().editor_config().name, "Plant copy 2");
        assert_eq!(workspace.selected().editor_config().generation.angle, 60.0);
        assert!(workspace.selected().is_dirty());
        assert!(!workspace.can_reset());
    }

    #[test]
    fn copy_entry_preserves_parseable_invalid_draft() {
        let first = config_text("Plant", "F", 60.0);
        let draft = first.replace("axiom = \"F\"", "axiom = \"[\"");
        let mut workspace = ConfigWorkspace::from_presets(vec![("Plant", first.clone())]).unwrap();
        workspace.selected_mut().set_draft_text(draft.clone());

        let entry = workspace.copy().unwrap();
        let expected_draft = config_text_renamed(&draft, "Plant copy");
        let expected_applied = config_text_renamed(&first, "Plant copy");

        assert_eq!(entry.name(), "Plant copy");
        assert_eq!(entry.draft_text(), expected_draft);
        assert!(entry.is_dirty());

        assert_eq!(workspace.selected_index(), 1);
        assert_eq!(workspace.selected().applied_text(), expected_applied);
        assert_eq!(workspace.selected().editor_config().name, "Plant copy");
        assert_eq!(workspace.selected().editor_config().generation.angle, 60.0);
        assert!(ConfigSource::parse(workspace.selected().draft_text().as_ref()).is_ok());
        assert!(matches!(
            workspace.apply(),
            Err(ConfigWorkspaceError::ParseConfig(
                ParseConfigError::Validation(ConfigError::UnmatchedOpen { .. })
            ))
        ));
        assert!(workspace.selected().is_dirty());
        assert!(!workspace.can_reset());
    }

    #[test]
    fn copy_entry_preserves_unparseable_draft_text() {
        let first = config_text("Plant", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("Plant", first.clone())]).unwrap();
        workspace
            .selected_mut()
            .set_draft_text("not valid toml".to_string());

        let entry = workspace.copy().unwrap();
        let expected_applied = config_text_renamed(&first, "Plant copy");

        assert_eq!(entry.name(), "Plant copy");
        assert_eq!(entry.draft_text(), "not valid toml");
        assert!(entry.is_dirty());

        assert_eq!(workspace.selected_index(), 1);
        assert_eq!(workspace.selected().applied_text(), expected_applied);
        assert_eq!(workspace.selected().editor_config().name, "Plant copy");
        assert_eq!(workspace.selected().editor_config().generation.angle, 60.0);
        assert!(ConfigSource::parse(workspace.selected().draft_text().as_ref()).is_err());
        assert!(workspace.selected().is_dirty());
        assert!(!workspace.can_reset());
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
        assert_eq!(workspace.entries()[0].name(), "First");
        assert_eq!(workspace.entries()[1].name(), "Second");
    }

    #[test]
    fn fresh_workspace_is_not_dirty_and_selects_first_entry() {
        let first = config_text("First", "F", 60.0);
        let workspace = ConfigWorkspace::from_presets(vec![("First", first)]).unwrap();

        assert_eq!(workspace.selected_index(), 0);
        assert!(!workspace.selected().is_dirty());
    }

    #[test]
    fn draft_text_matching_runtime_config_is_clean() {
        let first = config_text("First", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("First", first.clone())]).unwrap();

        workspace
            .selected_mut()
            .set_draft_text("temporary edit".to_string());
        assert!(workspace.selected().is_dirty());

        workspace.selected_mut().set_draft_text(first);

        assert!(!workspace.selected().is_dirty());
    }

    #[test]
    fn clean_entry_set_iterations_updates_toml_and_editor_config() {
        let first = config_text("First", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("First", first)]).unwrap();

        clean_mut(&mut workspace).set_iterations(5).unwrap();

        let entry = workspace.selected();
        assert!(entry.draft_text().contains("iterations = 5"));
        assert_eq!(entry.editor_config().generation.iterations, 5);
        assert!(!entry.is_dirty());
    }

    #[test]
    fn clean_entry_set_angle_updates_toml_and_editor_config() {
        let first = config_text("First", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("First", first)]).unwrap();

        clean_mut(&mut workspace).set_angle(45.5).unwrap();

        let entry = workspace.selected();
        assert!(entry.draft_text().contains("angle = 45.5"));
        assert_eq!(entry.editor_config().generation.angle, 45.5);
        assert!(!entry.is_dirty());
    }

    #[test]
    fn clean_entry_set_initial_heading_updates_toml_and_editor_config() {
        let first = config_text("First", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("First", first)]).unwrap();

        clean_mut(&mut workspace).set_initial_heading(45.0).unwrap();

        let entry = workspace.selected();
        assert!(entry.draft_text().contains("initial_heading = 45"));
        assert_eq!(entry.editor_config().generation.initial_heading, Some(45.0));
        assert!(!entry.is_dirty());
    }

    #[test]
    fn clean_entry_set_dimensions_updates_toml_and_editor_config() {
        let first = config_text("First", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("First", first)]).unwrap();
        assert_eq!(
            workspace.selected().editor_config().generation.dimensions,
            Dimensions::TwoD
        );
        clean_mut(&mut workspace)
            .set_dimensions(Dimensions::ThreeD)
            .unwrap();
        assert_eq!(
            workspace.selected().editor_config().generation.dimensions,
            Dimensions::ThreeD
        );
        assert!(
            workspace
                .selected()
                .draft_text()
                .contains("dimensions = \"3D\"")
        );
    }

    #[test]
    fn clean_entry_set_grammar_updates_axiom_and_rules() {
        let first = config_text("First", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("First", first)]).unwrap();
        let rules = vec![('F', "FF".to_string()), ('X', "F+X".to_string())];
        clean_mut(&mut workspace).set_grammar("XF", &rules).unwrap();
        let generation = &workspace.selected().editor_config().generation;
        assert_eq!(generation.axiom, "XF");
        assert_eq!(generation.rules[&'F'], "FF");
        assert_eq!(generation.rules[&'X'], "F+X");
    }

    #[test]
    fn clean_entry_set_grammar_rejects_invalid_axiom() {
        let first = config_text("First", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("First", first)]).unwrap();
        // '@' is not a valid symbol
        let result = clean_mut(&mut workspace).set_grammar("F@", &[]);
        assert!(result.is_err());
        // Entry left unchanged
        assert_eq!(workspace.selected().editor_config().generation.axiom, "F");
    }

    #[test]
    fn clean_entry_set_background_some_updates_toml_and_runtime_config() {
        let first = config_text("First", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("First", first)]).unwrap();

        clean_mut(&mut workspace)
            .set_background(Some(Rgb::new(0x1a, 0x33, 0x4d)))
            .unwrap();

        let entry = workspace.selected();
        assert!(entry.draft_text().contains("background = \"#1a334d\""));
        assert_eq!(
            entry.editor_config().colors.background,
            Some(Rgb::new(0x1a, 0x33, 0x4d))
        );
        assert_eq!(
            runtime_config(entry).colors.background,
            Rgb::new(0x1a, 0x33, 0x4d)
        );
        assert!(!entry.is_dirty());
    }

    #[test]
    fn clean_entry_set_background_none_removes_toml_and_keeps_config_valid() {
        let first = config_text("First", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("First", first)]).unwrap();

        clean_mut(&mut workspace).set_background(None).unwrap();

        let entry = workspace.selected();
        assert!(!entry.draft_text().contains("background ="));
        assert_eq!(entry.editor_config().colors.background, None);
        assert_eq!(
            runtime_config(entry).colors.background,
            ConfigDefaults::embedded().colors.background
        );
        assert!(!entry.is_dirty());
    }

    #[test]
    fn clean_entry_set_line_color_updates_solid_gradient_and_hue_cycle_configs() {
        let first = config_text("First", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("First", first)]).unwrap();

        clean_mut(&mut workspace)
            .set_line_color(Some(EditorLineColorConfig::Solid(Rgb::new(
                0x33, 0x4d, 0x66,
            ))))
            .unwrap();
        assert_eq!(
            runtime_config(workspace.selected()).colors.line,
            LineColorConfig::Solid(Rgb::new(0x33, 0x4d, 0x66))
        );

        clean_mut(&mut workspace)
            .set_line_color(Some(EditorLineColorConfig::Gradient {
                start: Some(Rgb::new(0x1a, 0x33, 0x4d)),
                end: Some(Rgb::new(0xb3, 0xcc, 0xe6)),
                topological_depth: Some(false),
            }))
            .unwrap();
        assert_eq!(
            runtime_config(workspace.selected()).colors.line,
            LineColorConfig::Gradient {
                start: Rgb::new(0x1a, 0x33, 0x4d),
                end: Rgb::new(0xb3, 0xcc, 0xe6),
                topological_depth: false,
            }
        );

        clean_mut(&mut workspace)
            .set_line_color(Some(EditorLineColorConfig::HueCycle {
                initial: Some(Rgb::new(0x40, 0x80, 0xbf)),
            }))
            .unwrap();
        assert_eq!(
            runtime_config(workspace.selected()).colors.line,
            LineColorConfig::HueCycle {
                initial: Rgb::new(0x40, 0x80, 0xbf),
            }
        );

        // topological_depth: true is preserved faithfully even for bracketless grammars —
        // normalization happens at the geometry-allocation boundary, not in resolved Config.
        clean_mut(&mut workspace)
            .set_line_color(Some(EditorLineColorConfig::Gradient {
                start: Some(Rgb::new(0x33, 0x4d, 0x66)),
                end: Some(Rgb::new(0x80, 0x99, 0xb3)),
                topological_depth: Some(true),
            }))
            .unwrap();
        assert_eq!(
            runtime_config(workspace.selected()).colors.line,
            LineColorConfig::Gradient {
                start: Rgb::new(0x33, 0x4d, 0x66),
                end: Rgb::new(0x80, 0x99, 0xb3),
                topological_depth: true,
            }
        );
    }

    #[test]
    fn clean_entry_set_line_color_transitions_remove_stale_keys() {
        let first = config_text("First", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("First", first)]).unwrap();

        clean_mut(&mut workspace)
            .set_line_color(Some(EditorLineColorConfig::Gradient {
                start: Some(Rgb::new(0x1a, 0x33, 0x4d)),
                end: Some(Rgb::new(0xb3, 0xcc, 0xe6)),
                topological_depth: Some(false),
            }))
            .unwrap();
        let text = workspace.selected().draft_text().into_owned();
        assert!(text.contains("[colors.line.gradient]"));
        assert!(text.contains("start = \"#1a334d\""));
        assert!(text.contains("end = \"#b3cce6\""));
        assert!(!text.contains("solid ="));
        assert!(!text.contains("initial ="));

        clean_mut(&mut workspace)
            .set_line_color(Some(EditorLineColorConfig::HueCycle {
                initial: Some(Rgb::new(0x66, 0x80, 0x99)),
            }))
            .unwrap();
        let text = workspace.selected().draft_text().into_owned();
        assert!(text.contains("[colors.line.hue_cycle]"));
        assert!(text.contains("initial = \"#668099\""));
        assert!(!text.contains("solid ="));
        assert!(!text.contains("start ="));
        assert!(!text.contains("end ="));

        clean_mut(&mut workspace)
            .set_line_color(Some(EditorLineColorConfig::Solid(Rgb::new(
                0x33, 0x4d, 0x66,
            ))))
            .unwrap();
        let text = workspace.selected().draft_text().into_owned();
        assert!(text.contains("solid = \"#334d66\""));
        assert!(!text.contains("initial ="));
        assert!(!text.contains("start ="));
        assert!(!text.contains("end ="));

        clean_mut(&mut workspace)
            .set_line_color(Some(EditorLineColorConfig::Gradient {
                start: Some(Rgb::new(0x1a, 0x33, 0x4d)),
                end: Some(Rgb::new(0x66, 0x80, 0x99)),
                topological_depth: Some(true),
            }))
            .unwrap();
        let text = workspace.selected().draft_text().into_owned();
        assert!(text.contains("[colors.line.gradient]"));
        assert!(text.contains("start = \"#1a334d\""));
        assert!(text.contains("end = \"#668099\""));
        assert!(text.contains("topological_depth = true"));
        assert!(!text.contains("solid ="));
        assert!(!text.contains("initial ="));
    }

    #[test]
    fn clean_entry_color_mutators_preserve_existing_value_comments() {
        let text = r##"[metadata]
name = "Decorated Color"

[l-system]
dimensions = "2D"
axiom = "F"
iterations = 1
angle = 60.0
step = 1.0
initial_heading = 0.0

[l-system.rules]
F = "FF"

[colors]
background = "#000000" # keep background comment

[colors.line]
solid = "#00e680" # keep line color comment
"##;
        let mut workspace =
            ConfigWorkspace::from_presets(vec![("Decorated", text.to_string())]).unwrap();

        clean_mut(&mut workspace)
            .set_background(Some(Rgb::new(0x1a, 0x33, 0x4d)))
            .unwrap();
        clean_mut(&mut workspace)
            .set_line_color(Some(EditorLineColorConfig::Solid(Rgb::new(
                0x66, 0x80, 0x99,
            ))))
            .unwrap();

        let text = workspace.selected().draft_text().into_owned();
        assert!(text.contains("background = \"#1a334d\" # keep background comment"));
        assert!(text.contains("solid = \"#668099\" # keep line color comment"));
    }

    #[test]
    fn clean_entry_color_mutators_preserve_existing_value_spacing() {
        let text = r##"[metadata]
name = "Decorated Arrays"

[l-system]
dimensions = "2D"
axiom = "F"
iterations = 1
angle = 60.0
step = 1.0
initial_heading = 0.0

[l-system.rules]
F = "FF"

[colors]
background = "#000000"

[colors.line.gradient]
start = "#00e680"
end = "#ffffff"
"##;
        let mut workspace =
            ConfigWorkspace::from_presets(vec![("Decorated", text.to_string())]).unwrap();

        clean_mut(&mut workspace)
            .set_background(Some(Rgb::new(0x1a, 0x33, 0x4d)))
            .unwrap();
        clean_mut(&mut workspace)
            .set_line_color(Some(EditorLineColorConfig::Gradient {
                start: Some(Rgb::new(0x66, 0x80, 0x99)),
                end: Some(Rgb::new(0xb3, 0xcc, 0xe6)),
                topological_depth: Some(false),
            }))
            .unwrap();

        let text = workspace.selected().draft_text().into_owned();
        assert!(text.contains("background = \"#1a334d\""));
        assert!(text.contains("start = \"#668099\""));
        assert!(text.contains("end = \"#b3cce6\""));
    }

    #[test]
    fn color_mutation_on_preset_entry_makes_it_resettable() {
        let first = config_text("First", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("First", first.clone())]).unwrap();
        assert!(!workspace.can_reset());

        clean_mut(&mut workspace)
            .set_background(Some(Rgb::new(0x1a, 0x33, 0x4d)))
            .unwrap();

        assert!(workspace.can_reset());

        let mut workspace = ConfigWorkspace::from_presets(vec![("First", first)]).unwrap();
        assert!(!workspace.can_reset());

        clean_mut(&mut workspace)
            .set_line_color(Some(EditorLineColorConfig::Solid(Rgb::new(
                0x1a, 0x33, 0x4d,
            ))))
            .unwrap();

        assert!(workspace.can_reset());
    }

    #[test]
    fn clean_entry_control_mutators_preserve_dotted_toml() {
        let mut workspace =
            ConfigWorkspace::from_presets(vec![("Dotted", dotted_config_text())]).unwrap();

        {
            let mut clean = clean_mut(&mut workspace);
            clean.set_iterations(5).unwrap();
            clean.set_angle(45.5).unwrap();
        }

        let entry = workspace.selected();
        let text = entry.draft_text();
        assert!(text.contains("l-system.iterations = 5"));
        assert!(text.contains("l-system.angle = 45.5"));
        assert!(!text.contains("[l-system]"));
        assert!(!text.contains("[turtle]"));
        assert_eq!(entry.editor_config().generation.iterations, 5);
        assert_eq!(entry.editor_config().generation.angle, 45.5);
        assert!(!entry.is_dirty());
    }

    #[test]
    fn clean_entry_color_mutators_preserve_dotted_background_and_write_nested_line_color() {
        let mut workspace =
            ConfigWorkspace::from_presets(vec![("Dotted", dotted_config_text())]).unwrap();

        {
            let mut clean = clean_mut(&mut workspace);
            clean
                .set_background(Some(Rgb::new(0x1a, 0x33, 0x4d)))
                .unwrap();
            clean
                .set_line_color(Some(EditorLineColorConfig::Gradient {
                    start: Some(Rgb::new(0x66, 0x80, 0x99)),
                    end: Some(Rgb::new(0xb3, 0xcc, 0xe6)),
                    topological_depth: Some(true),
                }))
                .unwrap();
        }

        let entry = workspace.selected();
        let text = entry.draft_text();
        assert!(text.contains("colors.background = \"#1a334d\""));
        assert!(text.contains("[colors.line.gradient]"));
        assert!(text.contains("start = \"#668099\""));
        assert!(text.contains("end = \"#b3cce6\""));
        assert!(text.contains("topological_depth = true"));
        assert!(!text.contains("[colors]"));
        assert!(!text.contains("[colors.line]"));
        assert_eq!(
            entry.editor_config().colors.background,
            Some(Rgb::new(0x1a, 0x33, 0x4d))
        );
        assert_eq!(
            runtime_config(entry).colors.background,
            Rgb::new(0x1a, 0x33, 0x4d)
        );
        // topological_depth: true is preserved faithfully — normalization happens at the
        // geometry-allocation boundary, not in resolved Config.
        assert_eq!(
            runtime_config(entry).colors.line,
            LineColorConfig::Gradient {
                start: Rgb::new(0x66, 0x80, 0x99),
                end: Rgb::new(0xb3, 0xcc, 0xe6),
                topological_depth: true,
            }
        );
        assert!(!entry.is_dirty());

        clean_mut(&mut workspace).set_background(None).unwrap();
        let entry = workspace.selected();
        assert!(!entry.draft_text().contains("colors.background"));
        assert_eq!(entry.editor_config().colors.background, None);
        assert_eq!(
            runtime_config(entry).colors.background,
            ConfigDefaults::embedded().colors.background
        );
    }

    #[test]
    fn clean_entry_control_mutators_preserve_scalar_comments() {
        let mut workspace =
            ConfigWorkspace::from_presets(vec![("Decorated", decorated_config_text())]).unwrap();

        {
            let mut clean = clean_mut(&mut workspace);
            clean.set_iterations(5).unwrap();
            clean.set_angle(45.5).unwrap();
        }

        let text = workspace.selected().draft_text().into_owned();
        assert!(text.contains("iterations = 5 # keep iterations comment"));
        assert!(text.contains("angle = 45.5 # keep angle comment"));
    }

    #[test]
    fn control_mutation_on_preset_entry_makes_it_resettable() {
        let first = config_text("First", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("First", first)]).unwrap();
        assert!(!workspace.can_reset());

        clean_mut(&mut workspace).set_iterations(5).unwrap();

        assert!(workspace.can_reset());
    }

    #[test]
    fn view_mut_tracks_clean_dirty_transitions() {
        let first = config_text("First", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("First", first)]).unwrap();

        // Fresh entry is clean → CleanMut is the only callable shape.
        assert!(matches!(
            workspace.selected_mut().view_mut(),
            EntryViewMut::Clean(_)
        ));

        // Editing the draft text transitions the entry to dirty.
        workspace
            .selected_mut()
            .set_draft_text("temporary edit".to_string());
        assert!(matches!(
            workspace.selected_mut().view_mut(),
            EntryViewMut::Dirty(_)
        ));

        // Reverting (only callable through the Dirty view) restores the clean state, and
        // control mutators succeed again.
        revert_selected(&mut workspace);
        match workspace.selected_mut().view_mut() {
            EntryViewMut::Clean(mut clean) => clean.set_iterations(3).unwrap(),
            EntryViewMut::Dirty(_) => panic!("entry should be clean after revert"),
        }
        assert_eq!(
            workspace.selected().editor_config().generation.iterations,
            3
        );
        assert!(!workspace.selected().is_dirty());
    }

    #[test]
    fn set_angle_rejects_non_finite_value_and_leaves_entry_unchanged() {
        let first = config_text("First", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("First", first)]).unwrap();
        let previous_text = workspace.selected().draft_text().into_owned();
        let previous_config = runtime_config(workspace.selected()).clone();

        let error = clean_mut(&mut workspace).set_angle(f32::NAN).unwrap_err();

        assert!(matches!(
            error,
            ParseConfigError::Validation(ConfigError::InvalidAngle(_))
        ));
        let entry = workspace.selected();
        assert_eq!(entry.draft_text(), previous_text);
        assert_eq!(runtime_config(entry), previous_config);
        assert!(!entry.is_dirty());
    }

    #[test]
    fn copy_selects_the_new_entry() {
        let first = config_text("Plant", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("Plant", first)]).unwrap();

        let entry = workspace.copy().unwrap();
        assert_eq!(entry.name(), "Plant copy");
        let entry_ptr = entry as *const _;

        assert_eq!(workspace.selected_index(), 1);
        assert!(std::ptr::eq(entry_ptr, workspace.selected()));
        assert_eq!(workspace.entries()[0].name(), "Plant");
    }

    #[test]
    fn select_rejects_out_of_bounds_index() {
        let first = config_text("First", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("First", first)]).unwrap();

        let error = workspace.select(1).unwrap_err();
        assert!(matches!(error, ConfigWorkspaceError::InvalidIndex(1)));
        assert_eq!(workspace.selected_index(), 0);
    }

    #[test]
    fn index_by_name_returns_none_for_unknown_name() {
        let first = config_text("First", "F", 60.0);
        let workspace = ConfigWorkspace::from_presets(vec![("First", first)]).unwrap();

        assert_eq!(workspace.index_by_name("Missing"), None);
        assert_eq!(workspace.index_by_name("First"), Some(0));
    }

    #[test]
    fn rename_updates_entry_name() {
        let first = config_text("Plant", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("Plant", first)]).unwrap();
        workspace.rename(0, "My Plant").unwrap();
        assert_eq!(workspace.selected().name(), "My Plant");
        assert!(workspace.selected().draft_text().contains("\"My Plant\""));
    }

    #[test]
    fn rename_rejects_duplicate_name() {
        let first = config_text("First", "F", 60.0);
        let second = config_text("Second", "F+F", 90.0);
        let mut workspace =
            ConfigWorkspace::from_presets(vec![("First", first), ("Second", second)]).unwrap();
        let err = workspace.rename(0, "Second").unwrap_err();
        assert!(matches!(err, ConfigWorkspaceError::DuplicateName(ref n) if n == "Second"));
        assert_eq!(workspace.selected().name(), "First");
    }

    #[test]
    fn clean_entry_set_line_color_none_removes_colors_line_and_resolves_to_solid_default() {
        let first = config_text("First", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("First", first)]).unwrap();

        clean_mut(&mut workspace).set_line_color(None).unwrap();

        let entry = workspace.selected();
        assert!(
            !entry.draft_text().contains("[colors.line"),
            "colors.line must be absent"
        );
        assert!(
            !entry.draft_text().contains("solid ="),
            "solid key must be absent"
        );
        assert!(entry.editor_config().colors.line.is_none());
        assert!(matches!(
            runtime_config(entry).colors.line,
            LineColorConfig::Solid(_)
        ));
    }

    #[test]
    fn clean_entry_set_line_color_gradient_preserves_none_fields() {
        let first = config_text("First", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("First", first)]).unwrap();

        clean_mut(&mut workspace)
            .set_line_color(Some(EditorLineColorConfig::Gradient {
                start: Some(Rgb::new(0x11, 0x22, 0x33)),
                end: None,
                topological_depth: None,
            }))
            .unwrap();

        let entry = workspace.selected();
        assert!(
            entry.draft_text().contains("#112233"),
            "authored start present"
        );
        assert!(
            !entry.draft_text().contains("end ="),
            "absent end must not appear"
        );
        assert!(
            !entry.draft_text().contains("topological_depth"),
            "absent td must not appear"
        );
        assert_eq!(
            entry.editor_config().colors.line,
            Some(EditorLineColorConfig::Gradient {
                start: Some(Rgb::new(0x11, 0x22, 0x33)),
                end: None,
                topological_depth: None,
            })
        );
        let expected_end = ConfigDefaults::embedded().colors.line.gradient.end;
        assert!(matches!(
            runtime_config(entry).colors.line,
            LineColorConfig::Gradient { end, .. } if end == expected_end
        ));
    }

    #[test]
    fn rename_rejects_invalid_index() {
        let first = config_text("First", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("First", first)]).unwrap();
        assert!(matches!(
            workspace.rename(1, "Y").unwrap_err(),
            ConfigWorkspaceError::InvalidIndex(1)
        ));
    }

    #[test]
    fn import_toml_creates_new_selected_entry_with_no_default() {
        let first = config_text("First", "F", 60.0);
        let imported = config_text("Imported", "F+F", 90.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("First", first)]).unwrap();

        let idx = workspace.import_toml(&imported).unwrap();

        assert_eq!(idx, 1);
        assert_eq!(workspace.selected_index(), 1);
        assert_eq!(workspace.selected().name(), "Imported");
        assert!(!workspace.selected().is_dirty());
        assert!(!workspace.can_reset()); // custom entries have no bundled default
        assert_eq!(workspace.entries().len(), 2);
    }

    #[test]
    fn import_toml_deduplicates_name_with_suffix() {
        let first = config_text("First", "F", 60.0);
        let duplicate = config_text("First", "F+F", 90.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("First", first)]).unwrap();

        let idx = workspace.import_toml(&duplicate).unwrap();

        assert_eq!(idx, 1);
        assert_eq!(workspace.selected().name(), "First 2");
        assert!(!workspace.selected().is_dirty());
        assert_eq!(workspace.selected().editor_config().generation.angle, 90.0);
    }

    #[test]
    fn import_toml_rejects_invalid_toml() {
        let first = config_text("First", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("First", first)]).unwrap();

        let err = workspace.import_toml("not valid toml").unwrap_err();

        assert!(matches!(
            err,
            ConfigWorkspaceError::ParseConfig(ParseConfigError::TomlParse(_))
        ));
        assert_eq!(workspace.entries().len(), 1);
        assert_eq!(workspace.selected_index(), 0);
    }

    #[test]
    fn import_toml_rejects_parseable_toml_with_invalid_config() {
        let first = config_text("First", "F", 60.0);
        let invalid = config_text("Invalid", "[", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("First", first)]).unwrap();

        let err = workspace.import_toml(&invalid).unwrap_err();

        assert!(matches!(
            err,
            ConfigWorkspaceError::ParseConfig(ParseConfigError::Validation(
                ConfigError::UnmatchedOpen { .. }
            ))
        ));
        assert_eq!(workspace.entries().len(), 1);
        assert_eq!(workspace.selected_index(), 0);
    }

    #[test]
    fn rename_works_while_entry_is_dirty() {
        let first = config_text("Plant", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("Plant", first)]).unwrap();
        workspace
            .selected_mut()
            .set_draft_text("dirty draft".to_string());
        workspace.rename(0, "Renamed Plant").unwrap();
        assert_eq!(workspace.selected().name(), "Renamed Plant");
        assert!(workspace.selected().is_dirty()); // draft preserved
    }
}
