//! Autosave storage backed by browser IndexedDB.
//!
//! Autosave is a non-essential enhancement: any failure to open, read, or
//! write the database is logged and degrades silently rather than surfacing
//! as a UI error.

use idb::{Database, DatabaseEvent, Error, Factory, ObjectStoreParams, Query, TransactionMode};
use wasm_bindgen::JsValue;

const DB_NAME: &str = "lsystem-autosave";
const DB_VERSION: u32 = 1;
const STORE_NAME: &str = "config";
const RECORD_KEY: &str = "current";

/// Opens (creating if necessary) the autosave database, adding the `config`
/// object store on first creation.
///
/// Returns `None` on any error; callers should treat that as "autosave
/// unavailable" rather than a fatal condition.
pub async fn open() -> Option<Database> {
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
        if let Err(err) = database.create_object_store(STORE_NAME, ObjectStoreParams::new()) {
            log::warn!("failed to create IndexedDB object store {STORE_NAME}: {err}");
        }
    });

    match open_request.await {
        Ok(database) => Some(database),
        Err(err) => {
            log::warn!("failed to open IndexedDB database {DB_NAME}: {err}");
            None
        }
    }
}

/// Loads the autosaved config text, if any.
///
/// Returns `None` (with a logged warning) on any error, and also `None`
/// (without a warning) if there is simply no saved record yet — callers
/// must treat both cases identically as "nothing to restore".
pub async fn load(db: &Database) -> Option<String> {
    let value = match load_value(db).await {
        Ok(value) => value,
        Err(err) => {
            log::warn!("failed to load autosaved config: {err}");
            return None;
        }
    };
    value.and_then(|value| value.as_string())
}

async fn load_value(db: &Database) -> Result<Option<JsValue>, Error> {
    let transaction = db.transaction(&[STORE_NAME], TransactionMode::ReadOnly)?;
    let store = transaction.object_store(STORE_NAME)?;
    let query: Query = JsValue::from_str(RECORD_KEY).into();
    store.get(query)?.await
}

/// Saves `text` as the autosaved config, overwriting any existing record.
///
/// Logs and returns on any error rather than panicking; a failed autosave
/// write must never crash the app.
// TODO(task-4): drop these `expect`s once autosave wiring calls `save`; until
// then it (and the helper it calls) is unreachable from outside the crate
// and triggers `dead_code`.
#[expect(dead_code)]
pub async fn save(db: &Database, text: &str) {
    if let Err(err) = save_value(db, text).await {
        log::warn!("failed to save autosaved config: {err}");
    }
}

#[expect(dead_code)]
async fn save_value(db: &Database, text: &str) -> Result<(), Error> {
    let transaction = db.transaction(&[STORE_NAME], TransactionMode::ReadWrite)?;
    let store = transaction.object_store(STORE_NAME)?;
    let key = JsValue::from_str(RECORD_KEY);
    store.put(&JsValue::from_str(text), Some(&key))?.await?;
    transaction.commit()?.await?;
    Ok(())
}
