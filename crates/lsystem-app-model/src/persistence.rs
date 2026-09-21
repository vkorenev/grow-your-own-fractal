//! Persistence-facing view of a [`ConfigWorkspace`].
//!
//! These types describe *what* a storage layer holds and *what* a window would like it to
//! hold, keyed by stable identity rather than by position or authored name. They carry no
//! storage, serialization, or window/tab notions: a storage layer maps them to and from its
//! own representation.

use std::collections::{HashMap, HashSet};

use crate::config_workspace::{ConfigEntryId, ConfigWorkspace};

/// Stable identity of a custom (copied or imported) entry in storage.
///
/// Minted by the storage layer, never by the workspace: the workspace only carries the id
/// once storage has assigned one (see [`ConfigWorkspace::assign_custom_id`]). Ids are
/// unrelated to [`ConfigEntryId`], which is meaningful only within one workspace instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CustomId(u64);

impl CustomId {
    pub fn new(id: u64) -> Self {
        Self(id)
    }

    pub fn get(self) -> u64 {
        self.0
    }
}

/// Identity of a persisted entry: a bundled preset by its path, or a custom entry by its
/// storage-minted id. A preset is never identified by its `metadata.name` or its position.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PersistedKey {
    Preset(String),
    Custom(CustomId),
}

#[derive(Debug, Clone, PartialEq)]
pub struct PersistedEntry {
    pub key: PersistedKey,
    pub toml: String,
}

/// What storage holds. Customs are in ascending [`CustomId`] order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StoredState {
    pub entries: Vec<PersistedEntry>,
    pub selected: Option<PersistedKey>,
}

/// A custom entry that storage has not yet minted an id for.
#[derive(Debug, Clone, PartialEq)]
pub struct UnmintedCustom {
    pub entry: ConfigEntryId,
    pub toml: String,
}

/// The selected entry, named either by persisted identity or, when it is a custom entry
/// without an id yet, by its in-workspace id.
#[derive(Debug, Clone, PartialEq)]
pub enum SelectionView {
    Key(PersistedKey),
    Unminted(ConfigEntryId),
}

/// This window's live persisted view.
#[derive(Debug, Clone, PartialEq)]
pub struct PersistedView {
    /// Changed presets and customs that have ids, in workspace order.
    pub entries: Vec<PersistedEntry>,
    /// Customs with no id yet, in workspace order.
    pub unminted: Vec<UnmintedCustom>,
    pub selected: SelectionView,
}

impl ConfigWorkspace {
    /// The persisted view of this workspace.
    ///
    /// A preset entry is included iff it differs from its bundled default, keyed by its
    /// path. A custom entry with an id is always included; one without goes to
    /// [`PersistedView::unminted`]. Only applied text is reported: unapplied drafts are
    /// never persisted.
    pub fn persisted_view(&self) -> PersistedView {
        let mut entries = Vec::new();
        let mut unminted = Vec::new();
        for entry in self.entries() {
            if let Some(path) = entry.preset_path() {
                if entry.differs_from_default() {
                    entries.push(PersistedEntry {
                        key: PersistedKey::Preset(path.to_string()),
                        toml: entry.applied_text(),
                    });
                }
            } else if let Some(id) = entry.custom_id() {
                entries.push(PersistedEntry {
                    key: PersistedKey::Custom(id),
                    toml: entry.applied_text(),
                });
            } else {
                unminted.push(UnmintedCustom {
                    entry: entry.id(),
                    toml: entry.applied_text(),
                });
            }
        }

        let selected = self.selected();
        let selected = match (selected.preset_path(), selected.custom_id()) {
            (Some(path), _) => SelectionView::Key(PersistedKey::Preset(path.to_string())),
            (None, Some(id)) => SelectionView::Key(PersistedKey::Custom(id)),
            (None, None) => SelectionView::Unminted(selected.id()),
        };

        PersistedView {
            entries,
            unminted,
            selected,
        }
    }
}

/// What one window believes storage holds, as of its last load, refresh, or committed save.
///
/// It is recorded in the same representation as [`PersistedView`]: a preset only while it
/// differs from its bundled default, a custom always. Callers must uphold that when they
/// [`set`](Self::set) an entry (a stored preset whose text equals its bundled default is
/// treated as absent locally, so it must not be recorded), because comparing different
/// representations would leave the entry permanently out of step with the live view.
///
/// Stored content this window could not parse or validate is remembered separately, with its
/// text, in the *ignored* map. A key is in at most one of the two maps. Deletes are computed
/// only from the recorded entries, so ignored rows are never deleted.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PersistedBaseline {
    /// What this window last loaded or saved, by key.
    entries: HashMap<PersistedKey, String>,
    /// The selection last saved or loaded.
    selected: Option<PersistedKey>,
    /// Stored content this window could not apply, with that content.
    ignored: HashMap<PersistedKey, String>,
}

impl PersistedBaseline {
    /// The recorded content for `key`, if any.
    pub fn get(&self, key: &PersistedKey) -> Option<&str> {
        self.entries.get(key).map(String::as_str)
    }

    /// Records `toml` as the content of `key`, replacing any earlier record and forgetting
    /// any ignored content for the same key.
    ///
    /// For a preset key the text must differ from the bundled default; see the type docs.
    pub fn set(&mut self, key: PersistedKey, toml: String) {
        self.ignored.remove(&key);
        self.entries.insert(key, toml);
    }

    /// Forgets the recorded content for `key`, returning it.
    pub fn remove(&mut self, key: &PersistedKey) -> Option<String> {
        self.entries.remove(key)
    }

    /// The recorded entry keys, in no particular order.
    pub fn keys(&self) -> impl Iterator<Item = &PersistedKey> {
        self.entries.keys()
    }

    /// The selection last saved or loaded.
    pub fn selected(&self) -> Option<&PersistedKey> {
        self.selected.as_ref()
    }

    pub fn set_selected(&mut self, selected: Option<PersistedKey>) {
        self.selected = selected;
    }

    /// Remembers stored `toml` for `key` as content this window cannot apply, replacing any
    /// earlier ignored content and dropping the recorded entry for the key (if any), so no
    /// later diff deletes the stored row.
    pub fn ignore(&mut self, key: PersistedKey, toml: String) {
        self.entries.remove(&key);
        self.ignored.insert(key, toml);
    }

    /// The ignored content for `key`, if any.
    pub fn ignored(&self, key: &PersistedKey) -> Option<&str> {
        self.ignored.get(key).map(String::as_str)
    }

    /// Whether `key` is ignored with content exactly equal to `toml`. Once the stored
    /// content changes, the entry is no longer ignored.
    pub fn is_ignored(&self, key: &PersistedKey, toml: &str) -> bool {
        self.ignored(key) == Some(toml)
    }

    /// Stops ignoring `key`, returning the content that was ignored.
    pub fn clear_ignored(&mut self, key: &PersistedKey) -> Option<String> {
        self.ignored.remove(key)
    }

    /// Updates the baseline to reflect a save that has been committed.
    ///
    /// Applies `delta`'s puts and deletes, records each minted custom's written content under
    /// its new key (even if the entry has since been removed locally, so the next diff
    /// deletes that row), and records the written selection, resolving an
    /// [`SelectionView::Unminted`] selection through `minted`. An unresolvable selection
    /// clears the recorded one, so the next diff writes it again.
    pub fn apply_saved(&mut self, delta: &SaveDelta, minted: &[(ConfigEntryId, CustomId)]) {
        for put in &delta.put {
            self.set(put.key.clone(), put.toml.clone());
        }
        for key in &delta.delete {
            self.remove(key);
        }
        let minted_key = |entry: ConfigEntryId| {
            minted
                .iter()
                .find(|(id, _)| *id == entry)
                .map(|(_, custom)| PersistedKey::Custom(*custom))
        };
        for custom in &delta.mint {
            if let Some(key) = minted_key(custom.entry) {
                self.set(key, custom.toml.clone());
            }
        }
        match &delta.selected {
            None => {}
            Some(SelectionView::Key(key)) => self.selected = Some(key.clone()),
            Some(SelectionView::Unminted(entry)) => self.selected = minted_key(*entry),
        }
    }
}

/// The writes a window must make to bring storage in line with its live view.
#[derive(Debug, Clone, PartialEq)]
pub struct SaveDelta {
    /// Entries to write, in workspace order.
    pub put: Vec<PersistedEntry>,
    /// Rows to remove: keys this window recorded that its view no longer has.
    pub delete: Vec<PersistedKey>,
    /// Customs to write under freshly minted ids, in workspace order. Storage must not
    /// reorder them: mint order fixes creation order.
    pub mint: Vec<UnmintedCustom>,
    /// The selection, present only if it must be written.
    pub selected: Option<SelectionView>,
}

impl SaveDelta {
    /// Whether there is nothing to write.
    pub fn is_empty(&self) -> bool {
        self.put.is_empty()
            && self.delete.is_empty()
            && self.mint.is_empty()
            && self.selected.is_none()
    }
}

/// What must be written to make storage reflect `view`, given what `baseline` says it holds.
///
/// Deletes come only from `baseline`'s recorded entries, in [`PersistedKey`] order, never from
/// a sweep of storage; ignored keys are not recorded and so are never deleted.
pub fn diff(baseline: &PersistedBaseline, view: &PersistedView) -> SaveDelta {
    let put = view
        .entries
        .iter()
        .filter(|entry| baseline.get(&entry.key) != Some(entry.toml.as_str()))
        .cloned()
        .collect();

    let live: HashSet<&PersistedKey> = view.entries.iter().map(|entry| &entry.key).collect();
    let mut delete: Vec<PersistedKey> = baseline
        .keys()
        .filter(|key| !live.contains(key))
        .cloned()
        .collect();
    delete.sort();

    let selected = match &view.selected {
        SelectionView::Key(key) if baseline.selected() == Some(key) => None,
        selected => Some(selected.clone()),
    };

    SaveDelta {
        put,
        delete,
        mint: view.unminted.clone(),
        selected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preset(path: &str) -> PersistedKey {
        PersistedKey::Preset(path.to_string())
    }

    fn custom(id: u64) -> PersistedKey {
        PersistedKey::Custom(CustomId::new(id))
    }

    fn entry(key: PersistedKey, toml: &str) -> PersistedEntry {
        PersistedEntry {
            key,
            toml: toml.to_string(),
        }
    }

    fn entry_id(id: u64) -> ConfigEntryId {
        id.to_string().parse().unwrap()
    }

    fn unminted(id: u64, toml: &str) -> UnmintedCustom {
        UnmintedCustom {
            entry: entry_id(id),
            toml: toml.to_string(),
        }
    }

    fn view(
        entries: Vec<PersistedEntry>,
        unminted: Vec<UnmintedCustom>,
        selected: SelectionView,
    ) -> PersistedView {
        PersistedView {
            entries,
            unminted,
            selected,
        }
    }

    /// A baseline holding `entries` and the given selection.
    fn baseline(
        entries: &[(PersistedKey, &str)],
        selected: Option<PersistedKey>,
    ) -> PersistedBaseline {
        let mut baseline = PersistedBaseline::default();
        for (key, toml) in entries {
            baseline.set(key.clone(), toml.to_string());
        }
        baseline.set_selected(selected);
        baseline
    }

    #[test]
    fn empty_baseline_puts_everything_and_always_writes_selection() {
        let view = view(
            vec![entry(preset("a"), "A1"), entry(custom(3), "C3")],
            vec![],
            SelectionView::Key(preset("a")),
        );

        let delta = diff(&PersistedBaseline::default(), &view);

        assert_eq!(delta.put, view.entries);
        assert!(delta.delete.is_empty());
        assert!(delta.mint.is_empty());
        assert_eq!(delta.selected, Some(SelectionView::Key(preset("a"))));
        assert!(!delta.is_empty());
    }

    #[test]
    fn identical_view_yields_an_empty_delta() {
        let baseline = baseline(&[(preset("a"), "A1"), (custom(3), "C3")], Some(preset("a")));
        let view = view(
            vec![entry(preset("a"), "A1"), entry(custom(3), "C3")],
            vec![],
            SelectionView::Key(preset("a")),
        );

        let delta = diff(&baseline, &view);

        assert!(delta.is_empty());
        assert_eq!(
            delta,
            SaveDelta {
                put: vec![],
                delete: vec![],
                mint: vec![],
                selected: None
            }
        );
    }

    #[test]
    fn one_changed_preset_is_the_only_put() {
        let baseline = baseline(
            &[(preset("a"), "A1"), (preset("b"), "B1")],
            Some(preset("a")),
        );
        let view = view(
            vec![entry(preset("a"), "A1"), entry(preset("b"), "B2")],
            vec![],
            SelectionView::Key(preset("a")),
        );

        let delta = diff(&baseline, &view);

        assert_eq!(delta.put, vec![entry(preset("b"), "B2")]);
        assert!(delta.delete.is_empty());
        assert!(delta.mint.is_empty());
        assert_eq!(delta.selected, None);
    }

    #[test]
    fn a_reset_preset_absent_from_the_view_is_a_delete() {
        let baseline = baseline(&[(preset("a"), "A1")], Some(preset("a")));
        let view = view(vec![], vec![], SelectionView::Key(preset("a")));

        let delta = diff(&baseline, &view);

        assert_eq!(delta.delete, vec![preset("a")]);
        assert!(delta.put.is_empty());
        assert!(delta.mint.is_empty());
        assert_eq!(delta.selected, None);
    }

    #[test]
    fn a_locally_removed_custom_is_a_delete() {
        let baseline = baseline(&[(custom(3), "C3"), (custom(4), "C4")], Some(preset("a")));
        let view = view(
            vec![entry(custom(4), "C4")],
            vec![],
            SelectionView::Key(preset("a")),
        );

        let delta = diff(&baseline, &view);

        assert_eq!(delta.delete, vec![custom(3)]);
        assert!(delta.put.is_empty());
    }

    #[test]
    fn deletes_are_reported_in_a_deterministic_order() {
        let baseline = baseline(
            &[
                (custom(9), "C9"),
                (preset("b"), "B"),
                (custom(2), "C2"),
                (preset("a"), "A"),
            ],
            None,
        );
        let view = view(vec![], vec![], SelectionView::Key(preset("z")));

        let delta = diff(&baseline, &view);

        assert_eq!(
            delta.delete,
            vec![preset("a"), preset("b"), custom(2), custom(9)]
        );
    }

    #[test]
    fn a_new_custom_is_a_mint() {
        let baseline = baseline(&[], Some(preset("a")));
        let view = view(
            vec![],
            vec![unminted(7, "N7")],
            SelectionView::Key(preset("a")),
        );

        let delta = diff(&baseline, &view);

        assert_eq!(delta.mint, vec![unminted(7, "N7")]);
        assert!(delta.put.is_empty());
        assert!(delta.delete.is_empty());
        assert_eq!(delta.selected, None);
    }

    #[test]
    fn unminted_customs_are_minted_in_the_order_given() {
        let baseline = baseline(&[], Some(preset("a")));
        let view = view(
            vec![],
            vec![unminted(9, "N9"), unminted(2, "N2"), unminted(5, "N5")],
            SelectionView::Key(preset("a")),
        );

        let delta = diff(&baseline, &view);

        assert_eq!(
            delta.mint,
            vec![unminted(9, "N9"), unminted(2, "N2"), unminted(5, "N5")]
        );
    }

    #[test]
    fn a_selection_only_change_writes_only_the_selection() {
        let baseline = baseline(&[(preset("a"), "A1")], Some(preset("a")));
        let view = view(
            vec![entry(preset("a"), "A1")],
            vec![],
            SelectionView::Key(preset("b")),
        );

        let delta = diff(&baseline, &view);

        assert!(delta.put.is_empty());
        assert!(delta.delete.is_empty());
        assert!(delta.mint.is_empty());
        assert_eq!(delta.selected, Some(SelectionView::Key(preset("b"))));
        assert!(!delta.is_empty());
    }

    #[test]
    fn an_unminted_selection_is_always_written() {
        let baseline = baseline(&[], Some(preset("a")));
        let view = view(
            vec![],
            vec![unminted(7, "N7")],
            SelectionView::Unminted(entry_id(7)),
        );

        let delta = diff(&baseline, &view);

        assert_eq!(delta.selected, Some(SelectionView::Unminted(entry_id(7))));
    }

    #[test]
    fn ignored_keys_are_never_deleted() {
        let mut baseline = baseline(&[(preset("a"), "A1")], Some(preset("a")));
        baseline.ignore(preset("bad"), "not toml".to_string());
        baseline.ignore(custom(5), "also not toml".to_string());
        let view = view(
            vec![entry(preset("a"), "A1")],
            vec![],
            SelectionView::Key(preset("a")),
        );

        let delta = diff(&baseline, &view);

        assert!(delta.is_empty());
    }

    #[test]
    fn a_custom_removed_before_its_first_save_yields_an_empty_delta() {
        // It was never minted, so it is in neither the view nor the baseline.
        let baseline = baseline(&[], Some(preset("a")));
        let view = view(vec![], vec![], SelectionView::Key(preset("a")));

        assert!(diff(&baseline, &view).is_empty());
    }

    #[test]
    fn apply_saved_makes_an_immediate_rediff_empty() {
        let mut baseline = baseline(
            &[(preset("a"), "A1"), (preset("gone"), "G")],
            Some(preset("a")),
        );
        let before = view(
            vec![entry(preset("a"), "A2"), entry(custom(3), "C3")],
            vec![unminted(7, "N7"), unminted(8, "N8")],
            SelectionView::Unminted(entry_id(8)),
        );
        let delta = diff(&baseline, &before);
        let minted = [
            (entry_id(7), CustomId::new(10)),
            (entry_id(8), CustomId::new(11)),
        ];

        baseline.apply_saved(&delta, &minted);

        // The workspace now carries the minted ids and reports the same content.
        let after = view(
            vec![
                entry(preset("a"), "A2"),
                entry(custom(3), "C3"),
                entry(custom(10), "N7"),
                entry(custom(11), "N8"),
            ],
            vec![],
            SelectionView::Key(custom(11)),
        );
        assert!(diff(&baseline, &after).is_empty());
        assert_eq!(baseline.get(&preset("gone")), None);
        assert_eq!(baseline.selected(), Some(&custom(11)));
    }

    #[test]
    fn apply_saved_keeps_a_minted_custom_removed_between_diff_and_commit_deletable() {
        let mut baseline = baseline(&[], Some(preset("a")));
        let before = view(
            vec![],
            vec![unminted(7, "N7")],
            SelectionView::Key(preset("a")),
        );
        let delta = diff(&baseline, &before);

        baseline.apply_saved(&delta, &[(entry_id(7), CustomId::new(10))]);

        // The user removed the entry after the delta was computed; storage still has the row.
        let after = view(vec![], vec![], SelectionView::Key(preset("a")));
        let next = diff(&baseline, &after);
        assert_eq!(next.delete, vec![custom(10)]);
        assert!(next.put.is_empty());
        assert!(next.mint.is_empty());
        assert_eq!(next.selected, None);
    }

    #[test]
    fn apply_saved_applies_puts_deletes_and_key_selection() {
        let mut baseline = baseline(&[(preset("a"), "A1"), (custom(3), "C3")], Some(preset("a")));
        let view = view(
            vec![entry(preset("a"), "A2")],
            vec![],
            SelectionView::Key(preset("b")),
        );
        let delta = diff(&baseline, &view);

        baseline.apply_saved(&delta, &[]);

        assert_eq!(baseline.get(&preset("a")), Some("A2"));
        assert_eq!(baseline.get(&custom(3)), None);
        assert_eq!(baseline.selected(), Some(&preset("b")));
    }

    #[test]
    fn apply_saved_leaves_selection_alone_when_the_delta_does_not_write_it() {
        let mut baseline = baseline(&[(preset("a"), "A1")], Some(preset("a")));
        let view = view(
            vec![entry(preset("a"), "A2")],
            vec![],
            SelectionView::Key(preset("a")),
        );
        let delta = diff(&baseline, &view);
        assert_eq!(delta.selected, None);

        baseline.apply_saved(&delta, &[]);

        assert_eq!(baseline.selected(), Some(&preset("a")));
    }

    #[test]
    fn apply_saved_forgets_an_ignored_row_it_overwrote() {
        let mut baseline = baseline(&[], Some(preset("a")));
        baseline.ignore(preset("a"), "bad".to_string());
        let view = view(
            vec![entry(preset("a"), "A2")],
            vec![],
            SelectionView::Key(preset("a")),
        );
        let delta = diff(&baseline, &view);

        baseline.apply_saved(&delta, &[]);

        assert_eq!(baseline.get(&preset("a")), Some("A2"));
        assert_eq!(baseline.ignored(&preset("a")), None);
    }

    #[test]
    fn accessors_record_read_and_remove_entries() {
        let mut baseline = PersistedBaseline::default();
        assert_eq!(baseline.get(&preset("a")), None);
        assert_eq!(baseline.selected(), None);

        baseline.set(preset("a"), "A1".to_string());
        baseline.set(custom(3), "C3".to_string());
        baseline.set(preset("a"), "A2".to_string());
        baseline.set_selected(Some(custom(3)));

        assert_eq!(baseline.get(&preset("a")), Some("A2"));
        assert_eq!(baseline.selected(), Some(&custom(3)));
        let mut keys: Vec<_> = baseline.keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, vec![preset("a"), custom(3)]);

        assert_eq!(baseline.remove(&preset("a")), Some("A2".to_string()));
        assert_eq!(baseline.remove(&preset("a")), None);
        assert_eq!(baseline.get(&preset("a")), None);
        assert_eq!(baseline.keys().count(), 1);

        baseline.set_selected(None);
        assert_eq!(baseline.selected(), None);
    }

    #[test]
    fn ignored_content_is_matched_by_key_and_exact_text() {
        let mut baseline = PersistedBaseline::default();
        baseline.ignore(preset("a"), "bad".to_string());

        assert!(baseline.is_ignored(&preset("a"), "bad"));
        assert!(!baseline.is_ignored(&preset("a"), "changed"));
        assert!(!baseline.is_ignored(&preset("b"), "bad"));
        assert_eq!(baseline.ignored(&preset("a")), Some("bad"));

        // Ignoring again replaces the remembered content.
        baseline.ignore(preset("a"), "worse".to_string());
        assert!(!baseline.is_ignored(&preset("a"), "bad"));
        assert!(baseline.is_ignored(&preset("a"), "worse"));

        assert_eq!(
            baseline.clear_ignored(&preset("a")),
            Some("worse".to_string())
        );
        assert_eq!(baseline.clear_ignored(&preset("a")), None);
        assert!(!baseline.is_ignored(&preset("a"), "worse"));
        assert_eq!(baseline.ignored(&preset("a")), None);
    }

    #[test]
    fn a_key_is_either_recorded_or_ignored_never_both() {
        let mut baseline = PersistedBaseline::default();
        baseline.set(preset("a"), "A1".to_string());

        baseline.ignore(preset("a"), "bad".to_string());
        assert_eq!(baseline.get(&preset("a")), None);
        assert!(baseline.is_ignored(&preset("a"), "bad"));

        baseline.set(preset("a"), "A2".to_string());
        assert_eq!(baseline.get(&preset("a")), Some("A2"));
        assert_eq!(baseline.ignored(&preset("a")), None);
    }
}
