//! Persistence-facing view of a [`ConfigWorkspace`].
//!
//! These types describe *what* a storage layer holds and *what* a window would like it to
//! hold, keyed by stable identity rather than by position or authored name. They carry no
//! storage, serialization, or window/tab notions: a storage layer maps them to and from its
//! own representation.

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
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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
