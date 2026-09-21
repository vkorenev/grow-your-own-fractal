use std::borrow::Cow;
use std::collections::HashMap;

use thiserror::Error;

use lsystem_core::{Dimensions, Rgb};

use crate::config_defaults::ParseConfigError;
use crate::editor_config::{ConfigDocument, ConfigSource, EditorConfig, EditorLineColorConfig};
use crate::persistence::{CustomId, PersistedBaseline, PersistedKey, StoredState};

#[derive(Debug, Error)]
pub enum ConfigWorkspaceError {
    #[error("at least one config entry is required")]
    Empty,

    #[error("unknown config entry id `{0}`")]
    UnknownId(ConfigEntryId),

    #[error("a bundled config cannot be removed")]
    CannotRemoveBundled,

    #[error(transparent)]
    ParseConfig(#[from] ParseConfigError),
}

/// The set of config entries a user can select, edit, copy, import and remove.
///
/// Only the selected entry is edited by the user. Non-selected entries are mutated solely
/// by [`ConfigWorkspace::assign_custom_id`], which does not change the entry's applied
/// text. Anything that persists the workspace may therefore skip re-serializing
/// non-selected entries until the selection, the entry set, or an assigned id changes.
#[derive(Debug, Clone)]
pub struct ConfigWorkspace {
    entries: Vec<ConfigEntry>,
    // INVARIANT: `selected < entries.len()`. Upheld by `from_presets` (rejects empty),
    // `select_by_id` (derives the index via `entries.iter().position`), `copy` and
    // `import_toml` (assign `len() - 1` after a push), and `remove_selected` (re-anchors
    // to the following entry, or the preceding one when the last entry is removed).
    // `entries` never becomes empty: `from_presets` requires a bundled entry and
    // `remove_selected` refuses to remove bundled entries. Any future method that removes
    // or reorders entries must re-anchor `selected` to preserve it, because `selected()`/
    // `selected_mut()` index into `entries` directly.
    selected: usize,
    next_id: u64,
}

#[derive(Debug, Clone)]
pub struct ConfigEntry {
    id: ConfigEntryId,
    default: Option<ConfigDocument>,
    draft: Option<String>,
    last_applied: ConfigDocument,
    /// The path a bundled preset was loaded from; `None` for custom entries. Preset
    /// entries are matched to persisted state by this path, never by name or position.
    origin_path: Option<String>,
    /// Storage-minted identity of a custom entry; `None` for presets and for customs that
    /// storage has not yet assigned an id.
    custom_id: Option<CustomId>,
}

/// Opaque, stable identifier for a `ConfigEntry` within a `ConfigWorkspace`.
///
/// Assigned when an entry is created and never reused, reordered, or persisted to a
/// config document. It exists so UI code and workspace methods can identify an entry
/// without depending on its (possibly duplicated) authored display name. Ids are only
/// meaningful relative to the `ConfigWorkspace` instance that allocated them — do not
/// compare ids minted by separate workspace instances.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConfigEntryId(u64);

/// Renders the id as a plain integer string, for round-tripping through string-typed
/// UI widgets (e.g. an HTML `<option value>`). Not used for persistence.
impl std::fmt::Display for ConfigEntryId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Parses an id previously rendered by [`ConfigEntryId`]'s `Display` impl, for
/// round-tripping through string-typed UI widgets. Not a general-purpose constructor:
/// parsing succeeds for any well-formed integer, regardless of whether it names a real
/// entry in any workspace — callers must still validate via [`ConfigWorkspace::select_by_id`].
impl std::str::FromStr for ConfigEntryId {
    type Err = std::num::ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(s.parse()?))
    }
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
    /// `label` is the preset's path (its file path). It appears in log warnings and is also
    /// the preset's persistence identity: it is stored as the entry's origin path and
    /// persisted state is matched to the preset by it.
    ///
    /// Presets that fail to parse or fail config validation are skipped with a `log::warn!`
    /// naming the offending label — they do not cause an error. Returns
    /// [`ConfigWorkspaceError::Empty`] only if every preset is invalid (or the iterator is
    /// empty). Duplicate names among the valid presets are allowed.
    ///
    /// The returned workspace has its selection pointed at the first entry.
    pub fn from_presets<L: std::fmt::Display>(
        presets: impl IntoIterator<Item = (L, String)>,
    ) -> Result<Self, ConfigWorkspaceError> {
        let mut entries_out = Vec::new();
        let mut next_id = 0u64;

        for (label, text) in presets {
            let entry = match ConfigEntry::preset(text, label.to_string(), ConfigEntryId(next_id)) {
                Ok(entry) => entry,
                Err(err) => {
                    log::warn!("Skipping invalid preset {label}: {err}");
                    continue;
                }
            };
            next_id += 1;
            entries_out.push(entry);
        }

        if entries_out.is_empty() {
            return Err(ConfigWorkspaceError::Empty);
        }
        Ok(Self {
            entries: entries_out,
            selected: 0,
            next_id,
        })
    }

    pub fn entries(&self) -> &[ConfigEntry] {
        &self.entries
    }

    #[cfg(test)]
    fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(ConfigEntry::name)
    }

    /// Derives a display label per entry, in current workspace order, disambiguating
    /// entries that share an authored `metadata.name`. A unique name is returned as-is;
    /// a name shared by N entries is suffixed `" (1)"` through `" (N)"` in workspace
    /// order. Display-only — never used as identity and never written to any document.
    pub fn display_options(&self) -> Vec<(ConfigEntryId, String)> {
        // (total occurrences of this name, occurrences assigned a label so far)
        let mut counts: HashMap<&str, (usize, usize)> = HashMap::new();
        for entry in &self.entries {
            counts.entry(entry.name()).or_insert((0, 0)).0 += 1;
        }

        self.entries
            .iter()
            .map(|entry| {
                let name = entry.name();
                let (total, seen) = counts.get_mut(name).expect("counted above");
                let label = if *total == 1 {
                    name.to_string()
                } else {
                    *seen += 1;
                    format!("{name} ({seen})")
                };
                (entry.id(), label)
            })
            .collect()
    }

    pub fn selected(&self) -> &ConfigEntry {
        &self.entries[self.selected]
    }

    pub fn selected_mut(&mut self) -> &mut ConfigEntry {
        &mut self.entries[self.selected]
    }

    /// Selects the entry with the given id. Returns
    /// [`ConfigWorkspaceError::UnknownId`] if no entry in this workspace has that id —
    /// in particular, an id from a different `ConfigWorkspace` instance is rejected.
    pub fn select_by_id(&mut self, id: ConfigEntryId) -> Result<(), ConfigWorkspaceError> {
        let index = self
            .entries
            .iter()
            .position(|entry| entry.id() == id)
            .ok_or(ConfigWorkspaceError::UnknownId(id))?;
        self.selected = index;
        Ok(())
    }

    /// Creates a renamed copy of the selected entry, appends it to the end of the workspace
    /// order, auto-selects it, and returns a borrow of it. After the borrow drops, the same
    /// entry remains accessible via [`ConfigWorkspace::selected`].
    pub fn copy(&mut self) -> Result<&ConfigEntry, ConfigWorkspaceError> {
        let id = self.allocate_id();
        let new_entry = {
            let entry = self.selected();
            let name = self.unique_name(&format!("{} copy", entry.name()));
            entry.copy_as(&name, id)?
        };
        self.entries.push(new_entry);
        self.selected = self.entries.len() - 1;
        Ok(&self.entries[self.selected])
    }

    /// Parses and validates `text` as a config document, creates a new custom entry from it
    /// (no bundled default), appends it to the end of the workspace order, auto-selects it,
    /// and returns a borrow of it.
    ///
    /// The imported document's `metadata.name` is preserved unchanged, even if it collides
    /// with an existing entry's name — duplicate names are allowed. Returns
    /// [`ConfigWorkspaceError::ParseConfig`] if `text` fails to parse or validate. On error
    /// the workspace state is unchanged: no entry is added and the selection is not moved.
    pub fn import_toml(&mut self, text: &str) -> Result<&ConfigEntry, ConfigWorkspaceError> {
        let source = ConfigSource::parse(text)?;
        let doc = ConfigDocument::try_from(source)?;
        self.push_custom(doc, None);
        self.selected = self.entries.len() - 1;
        Ok(&self.entries[self.selected])
    }

    /// Appends a custom entry for `doc` and returns its id, without changing the selection.
    fn push_custom(&mut self, doc: ConfigDocument, custom_id: Option<CustomId>) -> ConfigEntryId {
        let id = self.allocate_id();
        let mut entry = ConfigEntry::custom(doc, id);
        entry.custom_id = custom_id;
        self.entries.push(entry);
        id
    }

    /// Records the storage-minted `id` on the custom entry `entry`. Returns `false`, changing
    /// nothing, if no entry has that id or the entry already has a custom id.
    pub fn assign_custom_id(&mut self, entry: ConfigEntryId, id: CustomId) -> bool {
        match self
            .entries
            .iter_mut()
            .find(|candidate| candidate.id() == entry)
        {
            Some(candidate) if candidate.custom_id.is_none() => {
                candidate.custom_id = Some(id);
                true
            }
            _ => false,
        }
    }

    /// Restores `stored` onto this workspace, which must have just been built by
    /// [`from_presets`](Self::from_presets), and returns the baseline of what this window now
    /// holds. Nothing here fails as a whole: every problem is a per-entry `log::warn!` and the
    /// entry is skipped.
    ///
    /// - A stored preset is applied onto the entry with that path (through the draft, so the
    ///   bundled default is kept and Reset stays available). If it cannot be applied the entry
    ///   is left as it was.
    /// - Stored customs are recreated in ascending [`CustomId`] order, whatever order they are
    ///   supplied in, each carrying its id.
    /// - A stored preset whose path is no longer bundled becomes a custom entry after the
    ///   others. It has no id yet, so the next save mints one. The baseline still records the
    ///   old `preset:` key, which the live workspace no longer has, so that save also deletes the
    ///   stale row in the same transaction.
    /// - The stored selection is applied if it names an entry (or a converted orphan); otherwise
    ///   the selection is left as it is. The baseline records the stored selection either way,
    ///   so an unresolved one is corrected by the next save.
    ///
    /// Stored content this window cannot parse or validate is never deleted: it is noted in the
    /// baseline's ignored map, not its entries, so no later diff touches it.
    pub fn restore(&mut self, stored: &StoredState) -> PersistedBaseline {
        let mut baseline = PersistedBaseline::default();
        let mut customs: Vec<(CustomId, &str)> = Vec::new();
        let mut orphans: Vec<(&str, &str)> = Vec::new();

        for persisted in &stored.entries {
            let toml = persisted.toml.as_str();
            match &persisted.key {
                PersistedKey::Custom(id) => customs.push((*id, toml)),
                PersistedKey::Preset(path) => match self.position_of(&persisted.key) {
                    Some(index) => {
                        let entry = &mut self.entries[index];
                        entry.set_draft_text(toml.to_string());
                        match entry.apply_draft() {
                            Ok(()) => {
                                let id = entry.id();
                                self.record_in_baseline(id, &mut baseline);
                            }
                            Err(err) => {
                                if let EntryViewMut::Dirty(dirty) = entry.view_mut() {
                                    dirty.revert();
                                }
                                log::warn!("Ignoring stored preset {path}: {err}");
                                baseline.ignore(persisted.key.clone(), toml.to_string());
                            }
                        }
                    }
                    None => orphans.push((path, toml)),
                },
            }
        }

        customs.sort_by_key(|(id, _)| *id);
        for (id, toml) in customs {
            match parse_document(toml) {
                Ok(doc) => {
                    let entry = self.push_custom(doc, Some(id));
                    self.record_in_baseline(entry, &mut baseline);
                }
                Err(err) => {
                    log::warn!("Ignoring stored custom entry {}: {err}", id.get());
                    baseline.ignore(PersistedKey::Custom(id), toml.to_string());
                }
            }
        }

        // Entry index of each orphaned preset that was converted to a custom.
        let mut converted: Vec<(&str, usize)> = Vec::new();
        for (path, toml) in orphans {
            let key = PersistedKey::Preset(path.to_string());
            match parse_document(toml) {
                Ok(doc) => {
                    self.push_custom(doc, None);
                    converted.push((path, self.entries.len() - 1));
                    // Deliberately the one baseline record that does not go through
                    // `record_in_baseline` and has no matching entry: the next diff sees a
                    // recorded key the live workspace lacks and deletes the stale `preset:`
                    // row in the same transaction that mints the converted custom.
                    baseline.set(key, toml.to_string());
                }
                Err(err) => {
                    log::warn!("Ignoring stored preset {path}, which is no longer bundled: {err}");
                    baseline.ignore(key, toml.to_string());
                }
            }
        }

        if let Some(key) = &stored.selected {
            let resolved = self.position_of(key).or_else(|| match key {
                PersistedKey::Preset(path) => converted
                    .iter()
                    .find(|(orphan, _)| orphan == path)
                    .map(|&(_, index)| index),
                PersistedKey::Custom(_) => None,
            });
            match resolved {
                // An index into `entries`, so the `selected` invariant holds.
                Some(index) => self.selected = index,
                None => log::warn!("Stored selection {key:?} names no entry; keeping selection"),
            }
        }
        baseline.set_selected(stored.selected.clone());
        baseline
    }

    /// The index of the entry that `key` identifies: a preset by its path, a custom by its
    /// storage-minted id. Never by `metadata.name` or position.
    fn position_of(&self, key: &PersistedKey) -> Option<usize> {
        self.entries.iter().position(|entry| match key {
            PersistedKey::Preset(path) => entry.preset_path() == Some(path.as_str()),
            PersistedKey::Custom(id) => entry.custom_id() == Some(*id),
        })
    }

    /// Records what the entry `entry` holds into `baseline`, in the same representation as
    /// [`persisted_view`](Self::persisted_view): a preset only while it differs from its bundled
    /// default (otherwise any record of it is removed, since a stored row equal to the default is
    /// treated as absent locally), and a custom always, under its storage-minted id. A custom
    /// that has no id yet has no key and is not recorded.
    ///
    /// It records the entry's applied text, not whatever text was loaded into it. This is the
    /// only way baseline entries for a preset are to be written: they must never be `set`
    /// directly. An ignored note for the key is not touched by the removal but is cleared by a
    /// recording, as [`PersistedBaseline::set`] specifies.
    fn record_in_baseline(&self, entry: ConfigEntryId, baseline: &mut PersistedBaseline) {
        let Some(entry) = self
            .entries
            .iter()
            .find(|candidate| candidate.id() == entry)
        else {
            return;
        };
        if let Some(path) = entry.preset_path() {
            let key = PersistedKey::Preset(path.to_string());
            if entry.differs_from_default() {
                baseline.set(key, entry.applied_text());
            } else {
                baseline.remove(&key);
            }
        } else if let Some(id) = entry.custom_id() {
            baseline.set(PersistedKey::Custom(id), entry.applied_text());
        }
    }

    /// Removes the selected custom entry, together with its pending draft, and selects a
    /// neighbor: the entry that followed it in workspace order, or the preceding entry when
    /// the removed one was last. Returns [`ConfigWorkspaceError::CannotRemoveBundled`],
    /// leaving the workspace untouched, if the selected entry is bundled.
    ///
    /// Ids are never reused: `next_id` is left unchanged, so entries created later receive
    /// ids distinct from the removed one. Callers are responsible for confirming with the
    /// user first.
    pub fn remove_selected(&mut self) -> Result<(), ConfigWorkspaceError> {
        if self.selected().is_bundled() {
            return Err(ConfigWorkspaceError::CannotRemoveBundled);
        }
        self.entries.remove(self.selected);
        // The following entry (if any) slid into the removed index. Otherwise the removed
        // entry was last, so step back to its predecessor. `entries` cannot be empty here:
        // a bundled entry always remains (see the invariant on `selected`).
        if self.selected == self.entries.len() {
            self.selected -= 1;
        }
        Ok(())
    }

    fn unique_name(&self, base: &str) -> String {
        std::iter::once(base.to_string())
            .chain((2usize..).map(|suffix| format!("{base} {suffix}")))
            .find(|candidate| self.entries.iter().all(|entry| entry.name() != *candidate))
            .expect("suffix search should find a unique name")
    }

    /// Returns the stable id of the currently selected entry.
    pub fn selected_id(&self) -> ConfigEntryId {
        self.selected().id()
    }

    fn allocate_id(&mut self) -> ConfigEntryId {
        let id = ConfigEntryId(self.next_id);
        self.next_id += 1;
        id
    }
}

/// Parses and validates `text` as a config document.
fn parse_document(text: &str) -> Result<ConfigDocument, ParseConfigError> {
    ConfigDocument::try_from(ConfigSource::parse(text)?)
}

impl ConfigEntry {
    fn preset(text: String, path: String, id: ConfigEntryId) -> Result<Self, ConfigWorkspaceError> {
        let source = ConfigSource::parse(&text)?;
        let doc = ConfigDocument::try_from(source)?;
        Ok(Self {
            id,
            default: Some(doc.clone()),
            draft: None,
            last_applied: doc,
            origin_path: Some(path),
            custom_id: None,
        })
    }

    fn custom(last_applied: ConfigDocument, id: ConfigEntryId) -> Self {
        Self {
            id,
            default: None,
            draft: None,
            last_applied,
            origin_path: None,
            custom_id: None,
        }
    }

    /// This entry's stable identity. See [`ConfigEntryId`] for its guarantees.
    pub fn id(&self) -> ConfigEntryId {
        self.id
    }

    /// The path of the bundled preset this entry came from, or `None` for a custom entry.
    pub fn preset_path(&self) -> Option<&str> {
        self.origin_path.as_deref()
    }

    /// The storage-minted id of this custom entry, if it has one yet.
    pub fn custom_id(&self) -> Option<CustomId> {
        self.custom_id
    }

    pub fn name(&self) -> &str {
        self.last_applied.name()
    }

    /// Returns the name to prefill in a rename control.
    ///
    /// A parseable pending draft takes precedence over the applied document so opening
    /// the control cannot silently overwrite an unapplied `metadata.name` edit. An
    /// unparseable draft has no reliably authored name, so this falls back to the last
    /// applied name.
    pub fn name_for_rename(&self) -> Cow<'_, str> {
        self.draft
            .as_deref()
            .and_then(|draft| ConfigSource::parse(draft).ok())
            .and_then(|source| source.authored_name().map(str::to_owned))
            .map_or_else(|| Cow::Borrowed(self.name()), Cow::Owned)
    }

    pub fn draft_text(&self) -> Cow<'_, str> {
        match &self.draft {
            Some(draft) => Cow::Borrowed(draft),
            None => Cow::Owned(self.applied_text()),
        }
    }

    /// The TOML text of this entry's currently applied (committed) configuration,
    /// as opposed to [`Self::draft_text`] which may contain an unapplied draft.
    pub fn applied_text(&self) -> String {
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

    fn copy_as(&self, name: &str, id: ConfigEntryId) -> Result<Self, ConfigWorkspaceError> {
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
            id,
            default: None,
            draft,
            last_applied,
            // A copy is a new custom entry: it never inherits the source's identity.
            origin_path: None,
            custom_id: None,
        })
    }

    /// Validates this entry's draft text and commits it as the new applied document.
    /// A no-op if there is no pending draft. Returns an error on parse or validation
    /// failure; the entry is left unchanged in that case. Duplicate names with other
    /// entries in the workspace are allowed, so applying a draft renamed to match
    /// another entry succeeds.
    pub fn apply_draft(&mut self) -> Result<(), ParseConfigError> {
        let Some(draft) = &self.draft else {
            return Ok(());
        };
        let source = ConfigSource::parse(draft)?;
        let doc = ConfigDocument::try_from(source)?;
        self.last_applied = doc;
        self.draft = None;
        Ok(())
    }

    /// Restores this entry to its bundled default document, discarding any draft.
    /// Returns `false` without changing anything if the entry has no bundled default
    /// (e.g. a custom copy or import). Callers should gate user-visible resets on
    /// [`ConfigEntry::can_reset`]. The method itself always discards a pending draft, even
    /// when the applied document already matches the default.
    pub fn reset_to_default(&mut self) -> bool {
        let Some(default) = self.default.clone() else {
            return false;
        };
        self.last_applied = default;
        self.draft = None;
        true
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

    /// Renames this entry, updating both the applied document and any pending draft
    /// (so applying the draft later does not silently revert the rename).
    pub fn rename(&mut self, new_name: &str) -> Result<(), ParseConfigError> {
        let mut source = self.last_applied.source().clone();
        source.set_name(new_name);
        self.last_applied = ConfigDocument::try_from(source)?;
        // Update the draft name too so applying the draft later does not silently
        // revert the rename. If the draft is unparseable TOML, leave it verbatim —
        // the apply path will surface the parse error to the user. Re-derive dirtiness
        // the same way `set_draft_text` does: a rename can make the draft collapse back
        // to exactly the applied text (e.g. the draft already had this name), in which
        // case the entry should become clean rather than staying spuriously dirty.
        if let Some(draft_text) = &self.draft
            && let Ok(mut draft_source) = ConfigSource::parse(draft_text)
        {
            draft_source.set_name(new_name);
            let rewritten = draft_source.to_toml_string();
            self.draft = (rewritten != self.applied_text()).then_some(rewritten);
        }
        Ok(())
    }

    /// Whether this entry's applied document differs from its bundled default.
    /// Always `false` for entries without a bundled default (e.g. custom copies).
    pub fn differs_from_default(&self) -> bool {
        self.default
            .as_ref()
            .is_some_and(|default| self.applied_text() != default.to_toml_string())
    }

    /// Whether this entry ships with the application (a bundled preset), as opposed to a
    /// custom copy or import. Bundled entries can be reset but not removed; custom entries
    /// can be removed but not reset.
    pub fn is_bundled(&self) -> bool {
        self.default.is_some()
    }

    /// Whether a user-visible Reset should be enabled: the entry is bundled and resetting
    /// would either change its applied document or discard a pending draft. Always `false`
    /// for custom entries, even when dirty.
    pub fn can_reset(&self) -> bool {
        self.is_bundled() && (self.differs_from_default() || self.is_dirty())
    }
}

impl CleanMut<'_> {
    pub fn set_iterations(&mut self, iterations: u16) -> Result<(), ParseConfigError> {
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
    use crate::persistence::{
        PersistedEntry, PersistedKey, PersistedView, SelectionView, StoredState, diff,
    };

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
            .resolve(ConfigDefaults::embedded(), u16::MAX)
    }

    #[test]
    fn switching_entries_preserves_each_draft() {
        let first = config_text("First", "F", 60.0);
        let second = config_text("Second", "F+F", 90.0);
        let mut workspace =
            ConfigWorkspace::from_presets(vec![("First", first), ("Second", second)]).unwrap();
        let first_id = workspace.entries()[0].id();
        let second_id = workspace.entries()[1].id();

        workspace
            .selected_mut()
            .set_draft_text("edited first".to_string());
        workspace.select_by_id(second_id).unwrap();
        workspace
            .selected_mut()
            .set_draft_text("edited second".to_string());

        workspace.select_by_id(first_id).unwrap();
        assert_eq!(workspace.selected().draft_text(), "edited first");
        assert!(ConfigSource::parse(workspace.selected().draft_text().as_ref()).is_err());
        assert!(workspace.selected().is_dirty());
        workspace.select_by_id(second_id).unwrap();
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
        let error = workspace.selected_mut().apply_draft().unwrap_err();
        assert!(matches!(error, ParseConfigError::TomlParse(_)));

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
        let error = workspace.selected_mut().apply_draft().unwrap_err();

        assert!(matches!(
            error,
            ParseConfigError::Validation(ConfigError::UnmatchedOpen { .. })
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
        workspace.selected_mut().apply_draft().unwrap();

        assert_eq!(workspace.selected().name(), "Renamed");
        assert_eq!(workspace.names().collect::<Vec<_>>(), ["Renamed", "Second"]);
        assert!(!workspace.selected().is_dirty());
    }

    #[test]
    fn apply_on_clean_entry_returns_selected_entry_unchanged() {
        let first = config_text("First", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("First", first.clone())]).unwrap();

        workspace.selected_mut().apply_draft().unwrap();
        let entry = workspace.selected();

        assert_eq!(entry.name(), "First");
        assert_eq!(entry.applied_text(), first);
        assert!(!entry.is_dirty());
    }

    #[test]
    fn apply_accepts_duplicate_renamed_draft() {
        let first = config_text("First", "F", 60.0);
        let second = config_text("Second", "F+F", 90.0);
        let mut workspace =
            ConfigWorkspace::from_presets(vec![("First", first.clone()), ("Second", second)])
                .unwrap();

        workspace
            .selected_mut()
            .set_draft_text(config_text_renamed(&first, "Second"));
        workspace.selected_mut().apply_draft().unwrap();

        assert_eq!(workspace.selected().name(), "Second");
        assert!(!workspace.selected().is_dirty());
        assert_eq!(workspace.names().collect::<Vec<_>>(), ["Second", "Second"]);
    }

    #[test]
    fn revert_restores_last_applied() {
        let first = config_text("First", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("First", first.clone())]).unwrap();

        workspace
            .selected_mut()
            .set_draft_text(first.replace("angle = 60", "angle = 45"));
        workspace.selected_mut().apply_draft().unwrap();
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
        assert!(!workspace.selected().differs_from_default());

        workspace
            .selected_mut()
            .set_draft_text(first.replace("angle = 60", "angle = 45"));
        workspace.selected_mut().apply_draft().unwrap();
        assert_eq!(workspace.selected().editor_config().generation.angle, 45.0);
        assert!(workspace.selected().differs_from_default());

        assert!(workspace.selected_mut().reset_to_default());
        let reset_entry = workspace.selected();
        assert!(!reset_entry.is_dirty());
        assert_eq!(reset_entry.editor_config().generation.angle, 60.0);
        assert_eq!(reset_entry.draft_text(), first);
        assert_eq!(reset_entry.applied_text(), first);
        assert!(!workspace.selected().differs_from_default());
    }

    #[test]
    fn custom_entry_has_no_default_to_reset() {
        let first = config_text("Custom", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("Custom", first.clone())]).unwrap();
        workspace.copy().unwrap();

        assert!(!workspace.selected().differs_from_default());
        let draft = workspace
            .selected()
            .draft_text()
            .replace("angle = 60", "angle = 45");
        workspace.selected_mut().set_draft_text(draft);
        workspace.selected_mut().apply_draft().unwrap();

        assert!(!workspace.selected_mut().reset_to_default());
        assert_eq!(workspace.selected().editor_config().generation.angle, 45.0);
    }

    #[test]
    fn reset_to_default_discards_pending_draft() {
        // `can_reset()` is enabled for a dirty bundled entry (the specification requires
        // Reset to be available when it would discard a pending draft), so resetting a
        // dirty entry is a reachable path in both apps.
        let first = config_text("First", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("First", first.clone())]).unwrap();

        workspace
            .selected_mut()
            .set_draft_text(first.replace("angle = 60", "angle = 45"));
        workspace.selected_mut().apply_draft().unwrap();
        assert!(workspace.selected().differs_from_default());

        // Leave a pending, unapplied draft in place before resetting.
        workspace
            .selected_mut()
            .set_draft_text(first.replace("angle = 60", "angle = 30"));
        assert!(workspace.selected().is_dirty());

        assert!(workspace.selected_mut().reset_to_default());

        let reset_entry = workspace.selected();
        assert!(!reset_entry.is_dirty());
        assert_eq!(reset_entry.editor_config().generation.angle, 60.0);
        assert_eq!(reset_entry.draft_text(), first);
    }

    #[test]
    fn can_reset_requires_bundled_entry_with_change_or_draft() {
        let first = config_text("First", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("First", first.clone())]).unwrap();

        // Unchanged bundled entry: nothing to reset.
        assert!(workspace.selected().is_bundled());
        assert!(!workspace.selected().can_reset());

        // Only a pending draft: Reset would discard it.
        workspace
            .selected_mut()
            .set_draft_text(first.replace("angle = 60", "angle = 45"));
        assert!(!workspace.selected().differs_from_default());
        assert!(workspace.selected().can_reset());
        revert_selected(&mut workspace);
        assert!(!workspace.selected().can_reset());

        // Changed applied document: Reset would restore the default.
        workspace
            .selected_mut()
            .set_draft_text(first.replace("angle = 60", "angle = 45"));
        workspace.selected_mut().apply_draft().unwrap();
        assert!(workspace.selected().can_reset());

        // A custom entry can never be reset, even when it is dirty or differs.
        workspace.copy().unwrap();
        assert!(!workspace.selected().is_bundled());
        workspace
            .selected_mut()
            .set_draft_text("pending".to_string());
        assert!(workspace.selected().is_dirty());
        assert!(!workspace.selected().can_reset());
    }

    #[test]
    fn reset_of_default_bundled_entry_discards_draft_and_disables_reset() {
        let first = config_text("First", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("First", first.clone())]).unwrap();
        workspace
            .selected_mut()
            .set_draft_text(first.replace("angle = 60", "angle = 45"));
        assert!(workspace.selected().can_reset());

        assert!(workspace.selected_mut().reset_to_default());

        assert!(!workspace.selected().is_dirty());
        assert_eq!(workspace.selected().draft_text(), first);
        assert!(!workspace.selected().can_reset());
    }

    /// Bundled `A`, `B` followed by custom copies, so custom entries have neighbors on both
    /// sides. Returns the workspace and the ids of the entries in workspace order.
    fn workspace_with_copies() -> (ConfigWorkspace, Vec<ConfigEntryId>) {
        let mut workspace = ConfigWorkspace::from_presets(vec![
            ("A", config_text("A", "F", 60.0)),
            ("B", config_text("B", "F+F", 90.0)),
        ])
        .unwrap();
        workspace.copy().unwrap(); // "A copy"
        workspace.import_toml(&config_text("C", "F", 30.0)).unwrap();
        workspace.import_toml(&config_text("D", "F", 45.0)).unwrap();
        let ids = workspace.entries().iter().map(ConfigEntry::id).collect();
        (workspace, ids)
    }

    #[test]
    fn copy_and_import_append_in_workspace_order() {
        let (workspace, ids) = workspace_with_copies();

        assert_eq!(
            workspace.names().collect::<Vec<_>>(),
            ["A", "B", "A copy", "C", "D"]
        );
        assert_eq!(workspace.selected_id(), ids[4]);
    }

    #[test]
    fn remove_bundled_entry_is_rejected_without_changes() {
        let (mut workspace, ids) = workspace_with_copies();
        workspace.select_by_id(ids[1]).unwrap();
        workspace
            .selected_mut()
            .set_draft_text("pending".to_string());
        let names_before: Vec<String> = workspace.names().map(str::to_owned).collect();

        let error = workspace.remove_selected().unwrap_err();

        assert!(matches!(error, ConfigWorkspaceError::CannotRemoveBundled));
        assert_eq!(workspace.names().collect::<Vec<_>>(), names_before);
        assert_eq!(workspace.selected_id(), ids[1]);
        assert_eq!(workspace.selected().draft_text(), "pending");
    }

    #[test]
    fn remove_middle_custom_entry_selects_following_entry() {
        let (mut workspace, ids) = workspace_with_copies();
        workspace.select_by_id(ids[3]).unwrap(); // "C"

        workspace.remove_selected().unwrap();

        assert_eq!(
            workspace.names().collect::<Vec<_>>(),
            ["A", "B", "A copy", "D"]
        );
        assert_eq!(workspace.selected_id(), ids[4]);
        assert_eq!(workspace.selected().name(), "D");
        assert!(workspace.select_by_id(ids[3]).is_err());
    }

    #[test]
    fn remove_last_entry_selects_preceding_entry() {
        let (mut workspace, ids) = workspace_with_copies();
        assert_eq!(workspace.selected_id(), ids[4]); // "D" is last and selected

        workspace.remove_selected().unwrap();

        assert_eq!(
            workspace.names().collect::<Vec<_>>(),
            ["A", "B", "A copy", "C"]
        );
        assert_eq!(workspace.selected_id(), ids[3]);
    }

    #[test]
    fn remove_first_custom_entry_after_bundled_selects_following_entry() {
        let (mut workspace, ids) = workspace_with_copies();
        workspace.select_by_id(ids[2]).unwrap(); // "A copy"

        workspace.remove_selected().unwrap();

        assert_eq!(workspace.selected_id(), ids[3]);
        assert_eq!(workspace.names().collect::<Vec<_>>(), ["A", "B", "C", "D"]);
    }

    #[test]
    fn removing_all_custom_entries_leaves_last_bundled_entry_selected() {
        let (mut workspace, _) = workspace_with_copies();

        for _ in 0..3 {
            workspace.remove_selected().unwrap();
        }

        assert_eq!(workspace.names().collect::<Vec<_>>(), ["A", "B"]);
        assert_eq!(workspace.selected().name(), "B");
        assert!(matches!(
            workspace.remove_selected(),
            Err(ConfigWorkspaceError::CannotRemoveBundled)
        ));
    }

    #[test]
    fn remove_dirty_custom_entry_discards_draft_and_keeps_neighbor_intact() {
        let (mut workspace, ids) = workspace_with_copies();
        workspace.select_by_id(ids[3]).unwrap(); // "C"
        workspace
            .selected_mut()
            .set_draft_text("unapplied edit".to_string());
        assert!(workspace.selected().is_dirty());
        workspace.select_by_id(ids[4]).unwrap(); // "D"
        let neighbor_applied = workspace.selected().applied_text();
        workspace.select_by_id(ids[3]).unwrap();

        workspace.remove_selected().unwrap();

        assert_eq!(workspace.selected_id(), ids[4]);
        assert!(!workspace.selected().is_dirty());
        assert_eq!(workspace.selected().applied_text(), neighbor_applied);
        assert!(workspace.entries().iter().all(|entry| !entry.is_dirty()));
    }

    #[test]
    fn remove_recomputes_duplicate_display_suffixes() {
        let mut workspace =
            ConfigWorkspace::from_presets(vec![("A", config_text("A", "F", 60.0))]).unwrap();
        workspace.import_toml(&config_text("A", "F", 30.0)).unwrap();
        workspace.import_toml(&config_text("A", "F", 45.0)).unwrap();
        let labels = |workspace: &ConfigWorkspace| -> Vec<String> {
            workspace
                .display_options()
                .into_iter()
                .map(|(_, label)| label)
                .collect()
        };
        assert_eq!(labels(&workspace), ["A (1)", "A (2)", "A (3)"]);

        // Remove the middle duplicate ("A (2)").
        let middle = workspace.display_options()[1].0;
        workspace.select_by_id(middle).unwrap();
        workspace.remove_selected().unwrap();

        assert_eq!(labels(&workspace), ["A (1)", "A (2)"]);

        // Removing down to a single entry drops the suffix entirely.
        workspace.remove_selected().unwrap();
        assert_eq!(labels(&workspace), ["A"]);
    }

    #[test]
    fn ids_are_not_reused_after_removal() {
        let (mut workspace, ids) = workspace_with_copies();
        let removed = ids[4];

        workspace.remove_selected().unwrap();
        let new_id = workspace.copy().unwrap().id();

        assert!(!ids.contains(&new_id));
        assert_ne!(new_id, removed);
    }

    #[test]
    fn reset_restores_default_name_despite_collision() {
        let first = config_text("First", "F", 60.0);
        let second = config_text("Second", "F+F", 90.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![
            ("First", first.clone()),
            ("Second", second.clone()),
        ])
        .unwrap();
        let first_id = workspace.entries()[0].id();
        let second_id = workspace.entries()[1].id();

        workspace
            .selected_mut()
            .set_draft_text(config_text_renamed(&first, "Third"));
        workspace.selected_mut().apply_draft().unwrap();
        workspace.select_by_id(second_id).unwrap();
        workspace
            .selected_mut()
            .set_draft_text(config_text_renamed(&second, "First"));
        workspace.selected_mut().apply_draft().unwrap();

        workspace.select_by_id(first_id).unwrap();
        assert!(workspace.selected().differs_from_default());
        assert!(workspace.selected_mut().reset_to_default());

        assert_eq!(workspace.selected().name(), "First");
        assert_eq!(workspace.names().collect::<Vec<_>>(), ["First", "First"]);
    }

    #[test]
    fn copy_entry_preserves_dirty_valid_draft() {
        let first = config_text("Plant", "F", 60.0);
        let second = config_text("Plant copy", "F+F", 90.0);
        let draft = first.replace("angle = 60", "angle = 45");
        let mut workspace =
            ConfigWorkspace::from_presets(vec![("Plant", first.clone()), ("Plant copy", second)])
                .unwrap();
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
        assert!(!workspace.selected().differs_from_default());
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

        assert_eq!(entry.id(), workspace.selected_id());
        assert_eq!(workspace.selected().applied_text(), expected_applied);
        assert_eq!(workspace.selected().editor_config().name, "Plant copy");
        assert_eq!(workspace.selected().editor_config().generation.angle, 60.0);
        assert!(ConfigSource::parse(workspace.selected().draft_text().as_ref()).is_ok());
        assert!(matches!(
            workspace.selected_mut().apply_draft(),
            Err(ParseConfigError::Validation(
                ConfigError::UnmatchedOpen { .. }
            ))
        ));
        assert!(workspace.selected().is_dirty());
        assert!(!workspace.selected().differs_from_default());
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

        assert_eq!(entry.id(), workspace.selected_id());
        assert_eq!(workspace.selected().applied_text(), expected_applied);
        assert_eq!(workspace.selected().editor_config().name, "Plant copy");
        assert_eq!(workspace.selected().editor_config().generation.angle, 60.0);
        assert!(ConfigSource::parse(workspace.selected().draft_text().as_ref()).is_err());
        assert!(workspace.selected().is_dirty());
        assert!(!workspace.selected().differs_from_default());
    }

    #[test]
    fn from_presets_rejects_empty_iterator() {
        let error = ConfigWorkspace::from_presets(Vec::<(&str, String)>::new()).unwrap_err();

        assert!(matches!(error, ConfigWorkspaceError::Empty));
    }

    #[test]
    fn from_presets_accepts_duplicate_names() {
        let first = config_text("Duplicate", "F", 60.0);
        let second = config_text("Duplicate", "F+F", 90.0);
        let workspace =
            ConfigWorkspace::from_presets(vec![("dup1", first), ("dup2", second)]).unwrap();

        assert_eq!(workspace.entries().len(), 2);
        assert_eq!(
            workspace.names().collect::<Vec<_>>(),
            ["Duplicate", "Duplicate"]
        );
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

        assert_eq!(workspace.selected().name(), "First");
        assert!(!workspace.selected().is_dirty());
    }

    #[test]
    fn entries_get_distinct_stable_ids() {
        let first = config_text("First", "F", 60.0);
        let second = config_text("Second", "F+F", 90.0);
        let mut workspace =
            ConfigWorkspace::from_presets(vec![("First", first), ("Second", second)]).unwrap();

        let first_id = workspace.entries()[0].id();
        let second_id = workspace.entries()[1].id();
        assert_ne!(first_id, second_id);

        workspace.select_by_id(second_id).unwrap();
        assert_eq!(workspace.selected_id(), second_id);

        workspace.select_by_id(first_id).unwrap();
        assert_eq!(workspace.selected_id(), first_id);
    }

    #[test]
    fn display_options_returns_plain_name_when_unique() {
        let first = config_text("Plant", "F", 60.0);
        let workspace = ConfigWorkspace::from_presets(vec![("Plant", first)]).unwrap();

        let labels: Vec<String> = workspace
            .display_options()
            .into_iter()
            .map(|(_, label)| label)
            .collect();
        assert_eq!(labels, ["Plant"]);
    }

    #[test]
    fn display_options_disambiguates_duplicate_names_in_workspace_order() {
        let first = config_text("Plant", "F", 60.0);
        let second = config_text("Plant", "F+F", 90.0);
        let mut workspace =
            ConfigWorkspace::from_presets(vec![("a", first), ("b", second)]).unwrap();
        let third = config_text("Other", "F", 60.0);
        workspace.import_toml(&third).unwrap();

        let labels: Vec<String> = workspace
            .display_options()
            .into_iter()
            .map(|(_, label)| label)
            .collect();
        assert_eq!(labels, ["Plant (1)", "Plant (2)", "Other"]);
    }

    #[test]
    fn display_options_pairs_each_label_with_its_entry_id() {
        let first = config_text("Plant", "F", 60.0);
        let second = config_text("Plant", "F+F", 90.0);
        let workspace = ConfigWorkspace::from_presets(vec![("a", first), ("b", second)]).unwrap();

        let options = workspace.display_options();
        assert_eq!(options[0].0, workspace.entries()[0].id());
        assert_eq!(options[1].0, workspace.entries()[1].id());
    }

    #[test]
    fn copy_and_import_assign_fresh_ids() {
        let first = config_text("Plant", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("Plant", first)]).unwrap();
        let original_id = workspace.entries()[0].id();

        let copied_id = workspace.copy().unwrap().id();
        assert_ne!(copied_id, original_id);

        let imported = config_text("Imported", "F+F", 90.0);
        let imported_id = workspace.import_toml(&imported).unwrap().id();
        assert_ne!(imported_id, original_id);
        assert_ne!(imported_id, copied_id);
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
        assert!(!workspace.selected().differs_from_default());

        clean_mut(&mut workspace)
            .set_background(Some(Rgb::new(0x1a, 0x33, 0x4d)))
            .unwrap();

        assert!(workspace.selected().differs_from_default());

        let mut workspace = ConfigWorkspace::from_presets(vec![("First", first)]).unwrap();
        assert!(!workspace.selected().differs_from_default());

        clean_mut(&mut workspace)
            .set_line_color(Some(EditorLineColorConfig::Solid(Rgb::new(
                0x1a, 0x33, 0x4d,
            ))))
            .unwrap();

        assert!(workspace.selected().differs_from_default());
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
        assert!(!workspace.selected().differs_from_default());

        clean_mut(&mut workspace).set_iterations(5).unwrap();

        assert!(workspace.selected().differs_from_default());
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

        assert!(std::ptr::eq(entry_ptr, workspace.selected()));
        assert_eq!(workspace.entries()[0].name(), "Plant");
    }

    #[test]
    fn select_by_id_selects_matching_entry() {
        let first = config_text("First", "F", 60.0);
        let second = config_text("Second", "F+F", 90.0);
        let mut workspace =
            ConfigWorkspace::from_presets(vec![("First", first), ("Second", second)]).unwrap();
        let second_id = workspace.entries()[1].id();

        workspace.select_by_id(second_id).unwrap();

        assert_eq!(workspace.selected_id(), second_id);
    }

    #[test]
    fn select_by_id_rejects_unknown_id() {
        let first = config_text("First", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("First", first)]).unwrap();
        let original_id = workspace.selected_id();
        let unknown_id = {
            let mut other =
                ConfigWorkspace::from_presets(vec![("Other", config_text("Other", "F", 60.0))])
                    .unwrap();
            // `workspace` only ever allocates id 0; `other.copy()` allocates id 1, which is
            // guaranteed absent from `workspace`.
            other.copy().unwrap().id()
        };

        let error = workspace.select_by_id(unknown_id).unwrap_err();

        assert!(matches!(error, ConfigWorkspaceError::UnknownId(id) if id == unknown_id));
        assert_eq!(workspace.selected_id(), original_id);
    }

    #[test]
    fn select_by_id_works_independently_of_duplicate_names() {
        let first = config_text("Plant", "F", 60.0);
        let second = config_text("Plant", "F+F", 90.0);
        let mut workspace =
            ConfigWorkspace::from_presets(vec![("a", first), ("b", second)]).unwrap();
        let first_id = workspace.entries()[0].id();
        let second_id = workspace.entries()[1].id();

        workspace.select_by_id(second_id).unwrap();
        assert_eq!(workspace.selected().editor_config().generation.angle, 90.0);

        workspace.select_by_id(first_id).unwrap();
        assert_eq!(workspace.selected().editor_config().generation.angle, 60.0);
    }

    #[test]
    fn rename_updates_entry_name() {
        let first = config_text("Plant", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("Plant", first)]).unwrap();
        workspace.selected_mut().rename("My Plant").unwrap();
        assert_eq!(workspace.selected().name(), "My Plant");
        assert!(workspace.selected().draft_text().contains("\"My Plant\""));
    }

    #[test]
    fn name_for_rename_prefers_a_parseable_pending_draft() {
        let first = config_text("Plant", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("Plant", first.clone())]).unwrap();
        workspace
            .selected_mut()
            .set_draft_text(config_text_renamed(&first, "Draft Plant"));

        assert_eq!(workspace.selected().name(), "Plant");
        assert_eq!(workspace.selected().name_for_rename(), "Draft Plant");
    }

    #[test]
    fn name_for_rename_falls_back_to_applied_name_for_unparseable_draft() {
        let first = config_text("Plant", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("Plant", first)]).unwrap();
        workspace
            .selected_mut()
            .set_draft_text("not valid toml".to_string());

        assert_eq!(workspace.selected().name_for_rename(), "Plant");
    }

    #[test]
    fn rename_accepts_duplicate_name() {
        let first = config_text("First", "F", 60.0);
        let second = config_text("Second", "F+F", 90.0);
        let mut workspace =
            ConfigWorkspace::from_presets(vec![("First", first), ("Second", second)]).unwrap();

        workspace.selected_mut().rename("Second").unwrap();

        assert_eq!(workspace.selected().name(), "Second");
        assert_eq!(workspace.names().collect::<Vec<_>>(), ["Second", "Second"]);
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
    fn import_toml_creates_new_selected_entry_with_no_default() {
        let first = config_text("First", "F", 60.0);
        let imported = config_text("Imported", "F+F", 90.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("First", first)]).unwrap();

        let entry = workspace.import_toml(&imported).unwrap();

        assert_eq!(entry.id(), workspace.selected_id());
        assert_eq!(workspace.selected().name(), "Imported");
        assert!(!workspace.selected().is_dirty());
        assert!(!workspace.selected().differs_from_default()); // custom entries have no bundled default
        assert_eq!(workspace.entries().len(), 2);
    }

    #[test]
    fn import_toml_preserves_duplicate_name() {
        let first = config_text("First", "F", 60.0);
        let duplicate = config_text("First", "F+F", 90.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("First", first)]).unwrap();

        let entry = workspace.import_toml(&duplicate).unwrap();

        assert_eq!(entry.id(), workspace.selected_id());
        assert_eq!(workspace.selected().name(), "First");
        assert!(!workspace.selected().is_dirty());
        assert_eq!(workspace.selected().editor_config().generation.angle, 90.0);
        assert_eq!(workspace.names().collect::<Vec<_>>(), ["First", "First"]);
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
    }

    #[test]
    fn rename_works_while_entry_is_dirty() {
        let first = config_text("Plant", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("Plant", first)]).unwrap();
        workspace
            .selected_mut()
            .set_draft_text("dirty draft".to_string());
        workspace.selected_mut().rename("Renamed Plant").unwrap();
        assert_eq!(workspace.selected().name(), "Renamed Plant");
        assert!(workspace.selected().is_dirty()); // draft preserved
    }

    #[test]
    fn rename_rewrites_parseable_draft_so_apply_does_not_revert_the_rename() {
        let first = config_text("Plant", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("Plant", first.clone())]).unwrap();
        // A parseable draft that still differs from the applied document (different
        // angle), so it stays dirty across the rename.
        workspace
            .selected_mut()
            .set_draft_text(first.replace("angle = 60", "angle = 90"));

        workspace.selected_mut().rename("Renamed Plant").unwrap();
        assert!(workspace.selected().is_dirty());
        assert!(
            workspace
                .selected()
                .draft_text()
                .contains("name = \"Renamed Plant\"")
        );

        workspace.selected_mut().apply_draft().unwrap();
        assert_eq!(workspace.selected().name(), "Renamed Plant");
        assert_eq!(workspace.selected().editor_config().generation.angle, 90.0);
    }

    #[test]
    fn rename_clears_draft_that_becomes_identical_to_applied() {
        let first = config_text("Plant", "F", 60.0);
        let mut workspace = ConfigWorkspace::from_presets(vec![("Plant", first.clone())]).unwrap();
        // Hand-edit the draft so it differs from the applied document only by name.
        let draft = config_text_renamed(&first, "Renamed Plant");
        workspace.selected_mut().set_draft_text(draft);
        assert!(workspace.selected().is_dirty());

        // Renaming the entry to match makes the draft and the applied document
        // identical again — the entry should become clean, not stay spuriously dirty.
        workspace.selected_mut().rename("Renamed Plant").unwrap();

        assert_eq!(workspace.selected().name(), "Renamed Plant");
        assert!(!workspace.selected().is_dirty());
    }

    fn preset_key(path: &str) -> PersistedKey {
        PersistedKey::Preset(path.to_string())
    }

    /// Two presets labelled by path, so persistence identity is distinguishable from the
    /// authored names.
    fn workspace_with_paths() -> ConfigWorkspace {
        ConfigWorkspace::from_presets(vec![
            ("presets/a.toml", config_text("A", "F", 60.0)),
            ("presets/b.toml", config_text("B", "F+F", 90.0)),
        ])
        .unwrap()
    }

    fn unminted_entry_ids(view: &PersistedView) -> Vec<ConfigEntryId> {
        view.unminted.iter().map(|custom| custom.entry).collect()
    }

    #[test]
    fn preset_entries_record_their_path_and_customs_have_none() {
        let (workspace, ids) = workspace_with_copies();
        let entries = workspace.entries();

        assert_eq!(entries[0].preset_path(), Some("A"));
        assert_eq!(entries[1].preset_path(), Some("B"));
        assert!(
            entries[2..]
                .iter()
                .all(|entry| entry.preset_path().is_none())
        );
        assert!(entries.iter().all(|entry| entry.custom_id().is_none()));
        assert_eq!(entries.len(), ids.len());
    }

    #[test]
    fn unedited_workspace_persists_no_entries_and_selects_first_preset_path() {
        let workspace = workspace_with_paths();

        let view = workspace.persisted_view();

        assert!(view.entries.is_empty());
        assert!(view.unminted.is_empty());
        assert_eq!(
            view.selected,
            SelectionView::Key(preset_key("presets/a.toml"))
        );
    }

    #[test]
    fn editing_a_preset_persists_it_by_path_and_leaves_untouched_sibling_out() {
        let mut workspace = workspace_with_paths();
        let second_id = workspace.entries()[1].id();
        workspace.select_by_id(second_id).unwrap();
        clean_mut(&mut workspace).set_iterations(5).unwrap();

        let view = workspace.persisted_view();

        assert_eq!(
            view.entries,
            [PersistedEntry {
                key: preset_key("presets/b.toml"),
                toml: workspace.selected().applied_text(),
            }]
        );
        assert!(view.unminted.is_empty());
        assert_eq!(
            view.selected,
            SelectionView::Key(preset_key("presets/b.toml"))
        );
    }

    #[test]
    fn preset_is_matched_by_path_not_by_name() {
        // Renaming the preset (or giving it the sibling's name) must not change its key.
        let mut workspace = workspace_with_paths();
        workspace.selected_mut().rename("B").unwrap();

        let view = workspace.persisted_view();

        assert_eq!(view.entries.len(), 1);
        assert_eq!(view.entries[0].key, preset_key("presets/a.toml"));
    }

    #[test]
    fn resetting_an_edited_preset_drops_it_from_the_view() {
        let mut workspace = workspace_with_paths();
        clean_mut(&mut workspace).set_iterations(5).unwrap();
        assert_eq!(workspace.persisted_view().entries.len(), 1);

        assert!(workspace.selected_mut().reset_to_default());

        assert!(workspace.persisted_view().entries.is_empty());
    }

    #[test]
    fn pending_draft_is_not_reflected_in_the_view() {
        let mut workspace = workspace_with_paths();
        let text = workspace.selected().draft_text().into_owned();
        workspace
            .selected_mut()
            .set_draft_text(text.replace("angle = 60", "angle = 45"));
        assert!(workspace.selected().is_dirty());

        assert!(workspace.persisted_view().entries.is_empty());

        // Once the preset is applied-changed, a further unapplied draft still does not
        // leak: the entry carries its applied text.
        workspace.selected_mut().apply_draft().unwrap();
        let applied = workspace.selected().applied_text();
        workspace
            .selected_mut()
            .set_draft_text("unapplied edit".to_string());

        let view = workspace.persisted_view();

        assert_eq!(view.entries.len(), 1);
        assert_eq!(view.entries[0].toml, applied);
    }

    #[test]
    fn pending_draft_on_a_custom_is_not_reflected_in_the_view() {
        let (mut workspace, _) = workspace_with_copies();
        let applied = workspace.selected().applied_text();
        workspace
            .selected_mut()
            .set_draft_text("unapplied edit".to_string());

        let view = workspace.persisted_view();

        assert_eq!(view.unminted.last().unwrap().toml, applied);
    }

    #[test]
    fn copy_and_import_are_unminted_until_a_custom_id_is_assigned() {
        let (mut workspace, ids) = workspace_with_copies();

        let view = workspace.persisted_view();
        assert!(view.entries.is_empty());
        assert_eq!(unminted_entry_ids(&view), ids[2..]);
        assert_eq!(view.selected, SelectionView::Unminted(ids[4]));

        // Assign an id to the middle custom: only it moves into `entries`.
        assert!(workspace.assign_custom_id(ids[3], CustomId::new(7)));
        let view = workspace.persisted_view();
        assert_eq!(
            view.entries,
            [PersistedEntry {
                key: PersistedKey::Custom(CustomId::new(7)),
                toml: workspace.entries()[3].applied_text(),
            }]
        );
        assert_eq!(unminted_entry_ids(&view), [ids[2], ids[4]]);

        assert!(workspace.assign_custom_id(ids[2], CustomId::new(8)));
        assert!(workspace.assign_custom_id(ids[4], CustomId::new(9)));
        let view = workspace.persisted_view();
        assert!(view.unminted.is_empty());
        let keys: Vec<_> = view.entries.iter().map(|entry| entry.key.clone()).collect();
        assert_eq!(
            keys,
            [
                PersistedKey::Custom(CustomId::new(8)),
                PersistedKey::Custom(CustomId::new(7)),
                PersistedKey::Custom(CustomId::new(9)),
            ]
        );
        assert_eq!(workspace.entries()[3].custom_id(), Some(CustomId::new(7)));
    }

    #[test]
    fn unminted_customs_carry_their_applied_text() {
        let (workspace, _) = workspace_with_copies();

        let view = workspace.persisted_view();

        let texts: Vec<_> = view
            .unminted
            .iter()
            .map(|custom| custom.toml.clone())
            .collect();
        let expected: Vec<_> = workspace.entries()[2..]
            .iter()
            .map(ConfigEntry::applied_text)
            .collect();
        assert_eq!(texts, expected);
    }

    #[test]
    fn assign_custom_id_refuses_unknown_entry_or_second_id() {
        let (mut workspace, ids) = workspace_with_copies();
        let removed = ids[4];
        workspace.remove_selected().unwrap();

        assert!(!workspace.assign_custom_id(removed, CustomId::new(1)));

        assert!(workspace.assign_custom_id(ids[2], CustomId::new(2)));
        assert!(!workspace.assign_custom_id(ids[2], CustomId::new(3)));
        assert_eq!(workspace.entries()[2].custom_id(), Some(CustomId::new(2)));
    }

    #[test]
    fn selecting_an_unminted_custom_yields_unminted_selection_until_minted() {
        let (mut workspace, ids) = workspace_with_copies();
        workspace.select_by_id(ids[2]).unwrap();

        assert_eq!(
            workspace.persisted_view().selected,
            SelectionView::Unminted(ids[2])
        );

        assert!(workspace.assign_custom_id(ids[2], CustomId::new(4)));
        assert_eq!(
            workspace.persisted_view().selected,
            SelectionView::Key(PersistedKey::Custom(CustomId::new(4)))
        );
    }

    #[test]
    fn copy_of_a_custom_with_an_id_does_not_inherit_the_id() {
        let (mut workspace, ids) = workspace_with_copies();
        assert!(workspace.assign_custom_id(ids[4], CustomId::new(5)));
        assert_eq!(workspace.selected_id(), ids[4]);

        let copy_id = workspace.copy().unwrap().id();

        assert!(workspace.selected().custom_id().is_none());
        let view = workspace.persisted_view();
        assert!(unminted_entry_ids(&view).contains(&copy_id));
        assert!(
            view.entries
                .iter()
                .any(|entry| entry.key == PersistedKey::Custom(CustomId::new(5)))
        );
        assert_eq!(view.selected, SelectionView::Unminted(copy_id));
    }

    #[test]
    fn copy_of_a_preset_has_no_path_and_is_unminted() {
        let mut workspace = workspace_with_paths();

        let copy = workspace.copy().unwrap();

        assert!(copy.preset_path().is_none());
        assert!(copy.custom_id().is_none());
    }

    #[test]
    fn push_custom_appends_without_changing_the_selection() {
        let (mut workspace, ids) = workspace_with_copies();
        workspace.select_by_id(ids[1]).unwrap();
        let source = ConfigSource::parse(&config_text("Pushed", "F", 15.0)).unwrap();
        let doc = ConfigDocument::try_from(source).unwrap();

        let pushed = workspace.push_custom(doc, Some(CustomId::new(11)));

        assert!(!ids.contains(&pushed));
        assert_eq!(workspace.selected_id(), ids[1]);
        let last = workspace.entries().last().unwrap();
        assert_eq!(last.id(), pushed);
        assert_eq!(last.name(), "Pushed");
        assert_eq!(last.custom_id(), Some(CustomId::new(11)));
        assert!(last.preset_path().is_none());
    }

    #[test]
    fn import_toml_still_appends_and_selects_the_new_entry() {
        let mut workspace = workspace_with_paths();
        let first_id = workspace.selected_id();

        let imported_id = workspace
            .import_toml(&config_text("Imported", "F", 30.0))
            .unwrap()
            .id();

        assert_ne!(imported_id, first_id);
        assert_eq!(workspace.selected_id(), imported_id);
        assert_eq!(workspace.entries().last().unwrap().id(), imported_id);
        assert_eq!(workspace.entries().len(), 3);
        assert!(workspace.selected().custom_id().is_none());
        assert!(workspace.selected().preset_path().is_none());

        // A failed import leaves both the entries and the selection alone.
        assert!(workspace.import_toml("not valid toml").is_err());
        assert_eq!(workspace.entries().len(), 3);
        assert_eq!(workspace.selected_id(), imported_id);
    }

    fn custom_key(id: u64) -> PersistedKey {
        PersistedKey::Custom(CustomId::new(id))
    }

    fn stored(entries: Vec<(PersistedKey, String)>, selected: Option<PersistedKey>) -> StoredState {
        StoredState {
            entries: entries
                .into_iter()
                .map(|(key, toml)| PersistedEntry { key, toml })
                .collect(),
            selected,
        }
    }

    /// The applied text of `presets/b.toml` of [`workspace_with_paths`] after an edit, as a
    /// window would have stored it.
    fn edited_b_text() -> String {
        let mut workspace = workspace_with_paths();
        let second_id = workspace.entries()[1].id();
        workspace.select_by_id(second_id).unwrap();
        clean_mut(&mut workspace).set_iterations(5).unwrap();
        workspace.selected().applied_text()
    }

    fn entry_names(workspace: &ConfigWorkspace) -> Vec<&str> {
        workspace.names().collect()
    }

    #[test]
    fn restore_applies_an_edited_preset_onto_the_same_entry() {
        let mut workspace = workspace_with_paths();
        let edited = edited_b_text();
        let b_id = workspace.entries()[1].id();

        let baseline = workspace.restore(&stored(
            vec![(preset_key("presets/b.toml"), edited.clone())],
            None,
        ));

        assert_eq!(workspace.entries().len(), 2);
        let entry = &workspace.entries()[1];
        assert_eq!(entry.id(), b_id);
        assert_eq!(entry.preset_path(), Some("presets/b.toml"));
        assert_eq!(entry.applied_text(), edited);
        assert!(!entry.is_dirty());
        assert!(entry.differs_from_default());
        assert!(entry.can_reset());
        assert_eq!(baseline.get(&preset_key("presets/b.toml")), Some(&*edited));
        assert_eq!(baseline.get(&preset_key("presets/a.toml")), None);
    }

    #[test]
    fn restore_selects_the_preset_the_selection_names() {
        let mut workspace = workspace_with_paths();
        let b_id = workspace.entries()[1].id();

        let baseline = workspace.restore(&stored(
            vec![(preset_key("presets/b.toml"), edited_b_text())],
            Some(preset_key("presets/b.toml")),
        ));

        assert_eq!(workspace.selected_id(), b_id);
        assert_eq!(baseline.selected(), Some(&preset_key("presets/b.toml")));
    }

    #[test]
    fn restore_matches_presets_by_path_not_by_name_or_position() {
        // The stored row's text is named "A" but its key is b's path: it lands on b.
        let mut workspace = workspace_with_paths();
        let text = config_text("A", "F", 45.0);

        workspace.restore(&stored(vec![(preset_key("presets/b.toml"), text)], None));

        assert_eq!(workspace.entries()[1].name(), "A");
        assert!(workspace.entries()[1].differs_from_default());
        assert_eq!(workspace.entries()[0].name(), "A");
        assert!(!workspace.entries()[0].differs_from_default());
    }

    #[test]
    fn restore_recreates_customs_in_id_order_even_when_supplied_shuffled() {
        let mut workspace = workspace_with_paths();
        let first_id = workspace.selected_id();

        let baseline = workspace.restore(&stored(
            vec![
                (custom_key(9), config_text("Nine", "F", 9.0)),
                (custom_key(3), config_text("Three", "F", 3.0)),
                (custom_key(5), config_text("Five", "F", 5.0)),
            ],
            None,
        ));

        assert_eq!(entry_names(&workspace), ["A", "B", "Three", "Five", "Nine"]);
        let ids: Vec<_> = workspace.entries()[2..]
            .iter()
            .map(ConfigEntry::custom_id)
            .collect();
        assert_eq!(
            ids,
            [
                Some(CustomId::new(3)),
                Some(CustomId::new(5)),
                Some(CustomId::new(9))
            ]
        );
        assert!(workspace.entries()[2..].iter().all(|e| !e.is_bundled()));
        for entry in &workspace.entries()[2..] {
            let key = PersistedKey::Custom(entry.custom_id().unwrap());
            assert_eq!(baseline.get(&key), Some(&*entry.applied_text()));
        }
        // Restoring customs alone does not move the selection.
        assert_eq!(workspace.selected_id(), first_id);
    }

    #[test]
    fn restore_selects_a_custom_by_its_id() {
        let mut workspace = workspace_with_paths();

        let baseline = workspace.restore(&stored(
            vec![
                (custom_key(3), config_text("Three", "F", 3.0)),
                (custom_key(5), config_text("Five", "F", 5.0)),
            ],
            Some(custom_key(3)),
        ));

        assert_eq!(workspace.selected().name(), "Three");
        assert_eq!(workspace.selected().custom_id(), Some(CustomId::new(3)));
        assert_eq!(baseline.selected(), Some(&custom_key(3)));
    }

    #[test]
    fn restore_turns_an_orphaned_preset_into_a_custom_after_the_other_customs() {
        let mut workspace = workspace_with_paths();
        let orphan = config_text("Orphan", "F", 33.0);

        let baseline = workspace.restore(&stored(
            vec![
                (preset_key("presets/gone.toml"), orphan.clone()),
                (custom_key(8), config_text("Eight", "F", 8.0)),
                (custom_key(2), config_text("Two", "F", 2.0)),
            ],
            Some(preset_key("presets/gone.toml")),
        ));

        assert_eq!(
            entry_names(&workspace),
            ["A", "B", "Two", "Eight", "Orphan"]
        );
        let converted = workspace.entries().last().unwrap();
        assert!(converted.preset_path().is_none());
        assert!(converted.custom_id().is_none());
        assert!(!converted.is_bundled());
        assert_eq!(workspace.selected_id(), converted.id());
        // The old row is remembered so the next save deletes it.
        assert_eq!(
            baseline.get(&preset_key("presets/gone.toml")),
            Some(&*orphan)
        );
        assert_eq!(baseline.selected(), Some(&preset_key("presets/gone.toml")));
    }

    #[test]
    fn restore_skips_and_notes_an_invalid_orphaned_preset() {
        let mut workspace = workspace_with_paths();

        let baseline = workspace.restore(&stored(
            vec![(preset_key("presets/gone.toml"), "not = [valid".to_string())],
            Some(preset_key("presets/gone.toml")),
        ));

        assert_eq!(workspace.entries().len(), 2);
        assert_eq!(baseline.get(&preset_key("presets/gone.toml")), None);
        assert!(baseline.is_ignored(&preset_key("presets/gone.toml"), "not = [valid"));
        // The selection named an unusable row, so it stays on the first preset.
        assert_eq!(workspace.selected_id(), workspace.entries()[0].id());
        assert!(
            diff(&baseline, &workspace.persisted_view())
                .delete
                .is_empty()
        );
    }

    #[test]
    fn restore_leaves_a_preset_clean_when_its_stored_content_is_invalid() {
        let mut workspace = workspace_with_paths();
        // One row is not TOML, the other parses but is not a valid config.
        let not_toml = "not = [valid".to_string();
        let invalid_config = "[metadata]\nname = \"Nope\"\n".to_string();

        let baseline = workspace.restore(&stored(
            vec![
                (preset_key("presets/a.toml"), not_toml.clone()),
                (preset_key("presets/b.toml"), invalid_config.clone()),
            ],
            None,
        ));

        for entry in workspace.entries() {
            assert!(!entry.is_dirty());
            assert!(!entry.differs_from_default());
            assert!(!entry.can_reset());
        }
        assert_eq!(workspace.entries()[0].name(), "A");
        assert_eq!(workspace.entries()[1].name(), "B");
        assert_eq!(baseline.get(&preset_key("presets/a.toml")), None);
        assert_eq!(baseline.get(&preset_key("presets/b.toml")), None);
        assert!(baseline.is_ignored(&preset_key("presets/a.toml"), &not_toml));
        assert!(baseline.is_ignored(&preset_key("presets/b.toml"), &invalid_config));
        // The unusable rows are never deleted by a later save.
        assert!(
            diff(&baseline, &workspace.persisted_view())
                .delete
                .is_empty()
        );
    }

    #[test]
    fn restore_skips_an_invalid_custom_and_notes_it_as_ignored() {
        let mut workspace = workspace_with_paths();

        let baseline = workspace.restore(&stored(
            vec![
                (custom_key(4), "not = [valid".to_string()),
                (custom_key(7), config_text("Seven", "F", 7.0)),
            ],
            None,
        ));

        assert_eq!(entry_names(&workspace), ["A", "B", "Seven"]);
        assert!(
            workspace
                .entries()
                .iter()
                .all(|entry| entry.custom_id() != Some(CustomId::new(4)))
        );
        assert_eq!(baseline.get(&custom_key(4)), None);
        assert!(baseline.keys().all(|key| *key != custom_key(4)));
        assert!(baseline.is_ignored(&custom_key(4), "not = [valid"));
        assert!(baseline.get(&custom_key(7)).is_some());
        // The unusable row is never deleted by a later save.
        assert!(
            diff(&baseline, &workspace.persisted_view())
                .delete
                .is_empty()
        );
    }

    #[test]
    fn restore_does_not_record_a_preset_whose_text_equals_its_default() {
        let mut workspace = workspace_with_paths();
        let default_text = workspace.entries()[0].applied_text();

        let baseline = workspace.restore(&stored(
            vec![(preset_key("presets/a.toml"), default_text.clone())],
            None,
        ));

        // Treated as absent locally: not recorded, not ignored, and not deletable.
        assert_eq!(baseline.get(&preset_key("presets/a.toml")), None);
        assert_eq!(baseline.keys().count(), 0);
        assert_eq!(baseline.ignored_keys().count(), 0);
        let entry = &workspace.entries()[0];
        assert!(!entry.differs_from_default());
        assert!(!entry.can_reset());
        assert_eq!(entry.applied_text(), default_text);
        let view = workspace.persisted_view();
        assert!(view.entries.is_empty());
        assert!(diff(&baseline, &view).delete.is_empty());
    }

    #[test]
    fn restore_leaves_the_selection_valid_when_it_names_a_missing_custom() {
        let mut workspace = workspace_with_paths();
        let first_id = workspace.selected_id();

        let baseline = workspace.restore(&stored(vec![], Some(custom_key(42))));

        assert_eq!(workspace.selected_id(), first_id);
        assert_eq!(baseline.selected(), Some(&custom_key(42)));
        // The unresolved selection is corrected by the next save.
        let delta = diff(&baseline, &workspace.persisted_view());
        assert_eq!(
            delta.selected,
            Some(SelectionView::Key(preset_key("presets/a.toml")))
        );
    }

    #[test]
    fn restore_of_empty_storage_changes_nothing() {
        let mut workspace = workspace_with_paths();
        let first_id = workspace.selected_id();

        let baseline = workspace.restore(&StoredState::default());

        assert_eq!(workspace.entries().len(), 2);
        assert_eq!(workspace.selected_id(), first_id);
        assert_eq!(baseline.selected(), None);
        assert_eq!(baseline.keys().count(), 0);
    }

    #[test]
    fn restore_then_persisted_view_diffs_empty_against_the_returned_baseline() {
        let mut workspace = workspace_with_paths();
        let state = stored(
            vec![
                (preset_key("presets/b.toml"), edited_b_text()),
                (custom_key(9), config_text("Nine", "F", 9.0)),
                (custom_key(3), config_text("Three", "F", 3.0)),
            ],
            Some(custom_key(9)),
        );

        let baseline = workspace.restore(&state);

        assert!(diff(&baseline, &workspace.persisted_view()).is_empty());
    }

    #[test]
    fn restore_diff_is_only_the_orphan_delete_and_mint() {
        let mut workspace = workspace_with_paths();
        let state = stored(
            vec![
                (preset_key("presets/b.toml"), edited_b_text()),
                (
                    preset_key("presets/gone.toml"),
                    config_text("Orphan", "F", 33.0),
                ),
                (custom_key(3), config_text("Three", "F", 3.0)),
            ],
            Some(custom_key(3)),
        );

        let baseline = workspace.restore(&state);
        let delta = diff(&baseline, &workspace.persisted_view());

        let converted = workspace.entries().last().unwrap();
        assert!(delta.put.is_empty());
        assert_eq!(delta.delete, [preset_key("presets/gone.toml")]);
        assert_eq!(delta.mint.len(), 1);
        assert_eq!(delta.mint[0].entry, converted.id());
        assert_eq!(delta.mint[0].toml, converted.applied_text());
        assert_eq!(delta.selected, None);
    }
}
