//! Autosave storage backed by browser IndexedDB.
//!
//! Autosave is a non-essential enhancement: any failure to open, read, or
//! write the database is logged and degrades silently rather than surfacing
//! as a UI error. In particular, storage never deletes or rewrites a row it
//! merely failed to understand — an unreadable row is skipped with a warning
//! and left exactly as it is, so a window can never destroy state it could
//! not itself represent.
//!
//! This module owns the entire storage representation: store names, key
//! encodings, and transactions never leave it. Callers speak only in terms
//! of [`StoredState`] and [`SaveDelta`].
//!
//! Layout (database version 2):
//!
//! - `presets` — one record per changed bundled preset, keyed by its preset
//!   path (a string), holding the entry's applied TOML.
//! - `customs` — one record per custom entry, keyed by an `autoIncrement`
//!   integer (the [`CustomId`]), holding the entry's applied TOML.
//! - `meta` — bookkeeping; the selection lives under `SELECTION_KEY` as
//!   `"preset:<path>"` or `"custom:<id>"`.

use idb::{
    Database, DatabaseEvent, Error, Factory, ObjectStoreParams, Query, Transaction,
    TransactionMode, TransactionResult,
};
use lsystem_app_model::{
    ConfigEntryId, CustomId, PersistedEntry, PersistedKey, SaveDelta, SelectionView, StoredState,
};
use wasm_bindgen::{JsCast, JsValue};

const DB_NAME: &str = "lsystem-autosave";
const DB_VERSION: u32 = 2;
/// Changed bundled presets, keyed by preset path.
const PRESETS_STORE: &str = "presets";
/// Custom entries, keyed by an `autoIncrement` integer id.
const CUSTOMS_STORE: &str = "customs";
/// Bookkeeping that is not an entry; currently only the selection.
const META_STORE: &str = "meta";
/// Key of the selection record in [`META_STORE`].
const SELECTION_KEY: &str = "selected";
/// The single-record store used before database version 2.
const LEGACY_STORE: &str = "config";

/// Largest integer a JS number represents exactly (`Number.MAX_SAFE_INTEGER`).
const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

/// Opens (creating or upgrading if necessary) the autosave database.
///
/// `on_version_change` is called if another connection — another tab, or a
/// future version of this app — needs to upgrade the database. This
/// connection closes itself first, so an open window can never block another
/// window's upgrade. Operations on the closed connection then fail and
/// degrade through the normal save/load failure path; the callback exists so
/// the app can tell the user that this window is no longer persisting.
///
/// Returns `None` on any error; callers should treat that as "autosave
/// unavailable" rather than a fatal condition.
pub async fn open(on_version_change: impl Fn() + 'static) -> Option<Database> {
    let factory = match Factory::new() {
        Ok(factory) => factory,
        Err(err) => {
            log::warn!("failed to access IndexedDB factory: {err}");
            return None;
        }
    };

    let mut open_request = match factory.open(DB_NAME, Some(DB_VERSION)) {
        Ok(request) => request,
        Err(err) => {
            log::warn!("failed to open IndexedDB database {DB_NAME}: {err}");
            return None;
        }
    };

    open_request.on_upgrade_needed(|event| {
        let database = match event.database() {
            Ok(database) => database,
            Err(err) => {
                log::warn!("failed to access database during upgrade: {err}");
                return;
            }
        };
        upgrade(&database);
    });

    // Another window holding a connection at an older version blocks this
    // upgrade. Every window closes its connection on `versionchange` (see
    // below), so this resolves as soon as that window reacts — but a window
    // that has stopped running scripts never will, which is why startup is
    // bounded rather than waiting on this open.
    open_request.on_blocked(|_event| {
        log::warn!("IndexedDB upgrade is blocked by another open connection");
    });

    let mut database = match open_request.await {
        Ok(database) => database,
        Err(err) => {
            log::warn!("failed to open IndexedDB database {DB_NAME}: {err}");
            return None;
        }
    };

    database.on_version_change(move |event| {
        log::info!("closing the autosave connection for another window's upgrade");
        // Close via the event's own target: `Database::close` would need a
        // second handle to the connection the callback is stored in.
        match event
            .target()
            .and_then(|target| target.dyn_into::<web_sys::IdbDatabase>().ok())
        {
            Some(database) => database.close(),
            None => log::warn!("failed to close the autosave connection on versionchange"),
        }
        // Reported even if the close failed: the window has stopped being a
        // reliable writer either way.
        on_version_change();
    });

    Some(database)
}

/// Brings the database up to version 2: creates any store that is missing
/// and drops the legacy single-record store.
///
/// Must be called only inside an `upgradeneeded` handler.
fn upgrade(database: &Database) {
    let existing = database.store_names();
    let has = |name: &str| existing.iter().any(|store| store == name);

    let mut customs_params = ObjectStoreParams::new();
    customs_params.auto_increment(true);
    let stores = [
        (PRESETS_STORE, ObjectStoreParams::new()),
        (CUSTOMS_STORE, customs_params),
        (META_STORE, ObjectStoreParams::new()),
    ];
    for (name, params) in stores {
        if has(name) {
            continue;
        }
        if let Err(err) = database.create_object_store(name, params) {
            log::warn!("failed to create IndexedDB object store {name}: {err}");
        }
    }

    if has(LEGACY_STORE)
        && let Err(err) = database.delete_object_store(LEGACY_STORE)
    {
        log::warn!("failed to delete legacy IndexedDB object store {LEGACY_STORE}: {err}");
    }
}

/// Loads everything stored, as of one consistent read.
///
/// `None` means *failure*, and is distinct from `Some` of an empty
/// [`StoredState`], which is a valid answer meaning nothing has been saved
/// yet or everything was removed. Individual rows that cannot be read are
/// skipped with a warning and left in storage untouched.
///
/// Entries are reported with the changed presets first, then the customs in
/// ascending [`CustomId`] — that is, in creation order.
pub async fn load(db: &Database) -> Option<StoredState> {
    let raw = match read_all(db).await {
        Ok(raw) => raw,
        Err(err) => {
            log::warn!("failed to load autosaved workspace: {err}");
            return None;
        }
    };

    // A record count that disagrees with its key count means the two reads
    // cannot be paired at all, so nothing read can be trusted: fail rather
    // than risk attributing one entry's TOML to another entry's key.
    let presets = pair(raw.preset_keys, raw.preset_values, PRESETS_STORE)?;
    let customs = pair(raw.custom_keys, raw.custom_values, CUSTOMS_STORE)?;

    let mut entries = Vec::with_capacity(presets.len() + customs.len());
    for (key, value) in presets {
        let (Some(path), Some(toml)) = (key.as_string(), value.as_string()) else {
            log::warn!("skipping a stored preset row that is not a string key and value");
            continue;
        };
        entries.push(PersistedEntry {
            key: PersistedKey::Preset(path),
            toml,
        });
    }

    let mut stored_customs = Vec::with_capacity(customs.len());
    for (key, value) in customs {
        let (Some(id), Some(toml)) = (decode_custom_id(&key), value.as_string()) else {
            log::warn!("skipping a stored custom row with an unreadable key or value");
            continue;
        };
        stored_customs.push((id, toml));
    }
    // Never rely on the order the rows came back in for the contract above.
    stored_customs.sort_by_key(|(id, _)| *id);
    entries.extend(stored_customs.into_iter().map(|(id, toml)| PersistedEntry {
        key: PersistedKey::Custom(id),
        toml,
    }));

    // A selection that cannot be read is dropped, not treated as a failure:
    // the entries are still worth restoring, and the next save rewrites it.
    let selected = raw.selection.and_then(|value| {
        let decoded = value.as_string().as_deref().and_then(decode_selection);
        if decoded.is_none() {
            log::warn!("ignoring a stored selection that could not be decoded");
        }
        decoded
    });

    Some(StoredState { entries, selected })
}

/// The keys and values of every store, as read in one transaction.
struct RawState {
    preset_keys: Vec<JsValue>,
    preset_values: Vec<JsValue>,
    custom_keys: Vec<JsValue>,
    custom_values: Vec<JsValue>,
    selection: Option<JsValue>,
}

/// Reads all three stores in one ReadOnly transaction.
///
/// Keys and values are read with separate requests; within one transaction
/// both come back in ascending key order over the same unchanging contents,
/// so they pair up positionally (the caller still checks the lengths).
async fn read_all(db: &Database) -> Result<RawState, Error> {
    let transaction = db.transaction(
        &[PRESETS_STORE, CUSTOMS_STORE, META_STORE],
        TransactionMode::ReadOnly,
    )?;

    let presets = transaction.object_store(PRESETS_STORE)?;
    let preset_keys = presets.get_all_keys(None, None)?.await?;
    let preset_values = presets.get_all(None, None)?.await?;

    let customs = transaction.object_store(CUSTOMS_STORE)?;
    let custom_keys = customs.get_all_keys(None, None)?.await?;
    let custom_values = customs.get_all(None, None)?.await?;

    let meta = transaction.object_store(META_STORE)?;
    let selection = meta.get(selection_record_key())?.await?;

    Ok(RawState {
        preset_keys,
        preset_values,
        custom_keys,
        custom_values,
        selection,
    })
}

/// Pairs a store's keys with its values, or `None` (with a warning) if the
/// two reads disagree on how many records there are.
fn pair(keys: Vec<JsValue>, values: Vec<JsValue>, store: &str) -> Option<Vec<(JsValue, JsValue)>> {
    if keys.len() != values.len() {
        log::warn!(
            "stored {store} returned {} keys but {} values; ignoring the whole load",
            keys.len(),
            values.len()
        );
        return None;
    }
    Some(keys.into_iter().zip(values).collect())
}

/// Applies `delta` in a single ReadWrite transaction, returning the id
/// minted for each [`SaveDelta::mint`] entry.
///
/// Minting happens first and in the order given, since that order is the
/// entries' creation order and the ids fix it permanently. Mints, puts,
/// deletes, and the selection share one transaction, so a failure part-way
/// through aborts and leaves no minted rows behind — a later save simply
/// mints again.
///
/// Returns `None` (logged) on any failure.
pub async fn save(db: &Database, delta: &SaveDelta) -> Option<Vec<(ConfigEntryId, CustomId)>> {
    let transaction = match db.transaction(
        &[PRESETS_STORE, CUSTOMS_STORE, META_STORE],
        TransactionMode::ReadWrite,
    ) {
        Ok(transaction) => transaction,
        Err(err) => {
            log::warn!("failed to start the autosave transaction: {err}");
            return None;
        }
    };

    let minted = match write_delta(&transaction, delta).await {
        Ok(minted) => minted,
        Err(err) => {
            log::warn!("failed to save the workspace: {err}");
            // Abort explicitly: a failure that is not a failed request (a
            // malformed minted key, say) would otherwise let the transaction
            // go idle and commit the mints made so far. A request that failed
            // has already aborted the transaction, so this call errors and
            // there is nothing left to do. It is deliberately not awaited: an
            // `abort` event that has already fired would never reach a
            // handler attached now, and waiting would hang the caller.
            let _ = transaction.abort();
            return None;
        }
    };

    let committed = match transaction.commit() {
        Ok(transaction) => transaction.await,
        Err(err) => {
            log::warn!("failed to commit the autosave transaction: {err}");
            return None;
        }
    };
    match committed {
        Ok(TransactionResult::Committed) => Some(minted),
        Ok(TransactionResult::Aborted) => {
            log::warn!("the autosave transaction was aborted; nothing was written");
            None
        }
        Err(err) => {
            log::warn!("the autosave transaction failed: {err}");
            None
        }
    }
}

/// Writes `delta` through `transaction`, awaiting each request in turn so
/// the transaction never goes idle before the last one is issued.
async fn write_delta(
    transaction: &Transaction,
    delta: &SaveDelta,
) -> Result<Vec<(ConfigEntryId, CustomId)>, Error> {
    let presets = transaction.object_store(PRESETS_STORE)?;
    let customs = transaction.object_store(CUSTOMS_STORE)?;

    let mut minted = Vec::with_capacity(delta.mint.len());
    for custom in &delta.mint {
        let key = customs.add(&JsValue::from_str(&custom.toml), None)?.await?;
        let id = decode_custom_id(&key)
            .ok_or_else(|| Error::UnexpectedJsType("a safe non-negative integer", key))?;
        minted.push((custom.entry, id));
    }

    for entry in &delta.put {
        let value = JsValue::from_str(&entry.toml);
        match &entry.key {
            PersistedKey::Preset(path) => {
                presets.put(&value, Some(&JsValue::from_str(path)))?.await?;
            }
            // Writing an explicit key into the `autoIncrement` `customs`
            // store restores a removed entry under its original id, so an
            // entry that comes back keeps its place in creation order.
            PersistedKey::Custom(id) => {
                customs.put(&value, Some(&custom_record_key(*id)))?.await?;
            }
        }
    }

    for key in &delta.delete {
        match key {
            PersistedKey::Preset(path) => {
                presets.delete(Query::Key(JsValue::from_str(path)))?.await?;
            }
            PersistedKey::Custom(id) => {
                customs.delete(Query::Key(custom_record_key(*id)))?.await?;
            }
        }
    }

    if let Some(selected) = &delta.selected {
        let key = match selected {
            SelectionView::Key(key) => Some(key.clone()),
            SelectionView::Unminted(entry) => minted
                .iter()
                .find(|(id, _)| id == entry)
                .map(|(_, id)| PersistedKey::Custom(*id)),
        };
        match key {
            Some(key) => {
                let meta = transaction.object_store(META_STORE)?;
                let value = JsValue::from_str(&encode_selection(&key));
                meta.put(&value, Some(&JsValue::from_str(SELECTION_KEY)))?
                    .await?;
            }
            // The selected entry was not among the minted ones, so there is
            // no id to name it by. Leaving the stored selection alone is
            // better than writing one that names the wrong entry; the next
            // save writes it once the entry has an id.
            None => log::warn!("skipping the selection write: the selection has no stored key"),
        }
    }

    Ok(minted)
}

/// The record key of a custom entry: its id as a JS number.
fn custom_record_key(id: CustomId) -> JsValue {
    JsValue::from_f64(id.get() as f64)
}

/// Reads a custom entry's id back from a record key, or `None` if the key is
/// not a number that can be one (a negative, fractional, or too-large
/// number, or not a number at all).
fn decode_custom_id(key: &JsValue) -> Option<CustomId> {
    let number = key.as_f64()?;
    if number < 0.0 || number.fract() != 0.0 || number > MAX_SAFE_INTEGER {
        return None;
    }
    Some(CustomId::new(number as u64))
}

/// The record key of the selection in the `meta` store.
fn selection_record_key() -> Query {
    Query::Key(JsValue::from_str(SELECTION_KEY))
}

/// Encodes the selected entry's identity as the stored selection value.
fn encode_selection(key: &PersistedKey) -> String {
    match key {
        PersistedKey::Preset(path) => format!("preset:{path}"),
        PersistedKey::Custom(id) => format!("custom:{}", id.get()),
    }
}

/// Decodes a stored selection value, or `None` if it is not one.
///
/// The split is at the *first* colon only: a preset path may contain colons
/// of its own, and all of them belong to the path.
fn decode_selection(value: &str) -> Option<PersistedKey> {
    let (kind, rest) = value.split_once(':')?;
    match kind {
        "preset" => Some(PersistedKey::Preset(rest.to_owned())),
        "custom" => rest
            .parse::<u64>()
            .ok()
            .map(|id| PersistedKey::Custom(CustomId::new(id))),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    // The crate is wasm-only (`#![cfg(target_arch = "wasm32")]`), so these
    // compile for `wasm32-unknown-unknown` and are type-checked by
    // `cargo clippy --all-targets`, but running them needs a wasm test
    // runner (`wasm-bindgen-test`), which the crate does not depend on.
    use super::*;

    #[test]
    fn selection_round_trips() {
        for key in [
            PersistedKey::Preset("presets/tree.toml".to_owned()),
            PersistedKey::Custom(CustomId::new(0)),
            PersistedKey::Custom(CustomId::new(42)),
        ] {
            assert_eq!(decode_selection(&encode_selection(&key)), Some(key));
        }
    }

    #[test]
    fn a_preset_path_may_contain_colons() {
        let key = PersistedKey::Preset("a:b:c".to_owned());
        assert_eq!(encode_selection(&key), "preset:a:b:c");
        assert_eq!(decode_selection("preset:a:b:c"), Some(key));
    }

    #[test]
    fn malformed_selections_decode_to_nothing() {
        for value in ["", "preset", "custom", "custom:", "custom:x", "other:a"] {
            assert_eq!(decode_selection(value), None);
        }
    }
}
