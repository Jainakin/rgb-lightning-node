#[cfg(target_arch = "wasm32")]
use js_sys::Array;
use wasm_bindgen::prelude::JsValue;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_futures::{spawn_local, JsFuture};

#[cfg(test)]
#[path = "tests/runtime_store_tests.rs"]
mod tests;

pub(crate) trait RuntimeStateStore {
    fn get(&self, key: &str) -> Result<Option<String>, JsValue>;
    fn set(&self, key: &str, value: &str) -> Result<(), JsValue>;
    fn delete(&self, key: &str) -> Result<(), JsValue>;
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct BrowserPersistentStateStore;

impl RuntimeStateStore for BrowserPersistentStateStore {
    fn get(&self, key: &str) -> Result<Option<String>, JsValue> {
        local_storage_get_item(key)
    }

    fn set(&self, key: &str, value: &str) -> Result<(), JsValue> {
        local_storage_set_item(key, value)?;
        persist_to_indexed_db_background(key.to_string(), value.to_string());
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<(), JsValue> {
        local_storage_remove_item(key)?;
        remove_from_indexed_db_background(key.to_string());
        Ok(())
    }
}

pub(crate) fn browser_persistent_state_store() -> BrowserPersistentStateStore {
    BrowserPersistentStateStore
}

pub(crate) const RUNTIME_STATE_HYDRATE_PREFIXES: &[&str] = &[
    crate::wasm_node_persistence::WASM_LDK_RUNTIME_STORAGE_PREFIX,
    "rln:wasm:swap-runtime:",
    "rln:wasm:media:",
    "rln:wasm:wallet-rgb-proxy:",
    crate::wasm_node_persistence::WASM_LN_RUNTIME_CORE_STORAGE_PREFIX,
    crate::wasm_node_persistence::WASM_CHAIN_SYNC_STORAGE_PREFIX,
    crate::wasm_node_persistence::WASM_LDK_BROADCAST_QUEUE_STORAGE_PREFIX,
    crate::wasm_node_persistence::WASM_LDK_MONITORS_STORAGE_PREFIX,
    crate::wasm_node_persistence::WASM_RUNTIME_EVENTS_STORAGE_PREFIX,
    crate::wasm_node_persistence::WASM_RGB_LN_TRANSFERS_STORAGE_PREFIX,
    crate::wasm_node_persistence::WASM_VIRTUAL_CHANNELS_V0_STORAGE_PREFIX,
    crate::wasm_node_persistence::WASM_PEER_SESSIONS_STORAGE_PREFIX,
];

pub(crate) async fn preload_runtime_state_from_persistent_store() -> Result<(), JsValue> {
    hydrate_local_storage_from_indexed_db_prefixes(RUNTIME_STATE_HYDRATE_PREFIXES).await
}

#[cfg(target_arch = "wasm32")]
#[derive(Default)]
struct DurableStateInventory {
    revision: u64,
    keys: Option<Result<std::collections::BTreeSet<String>, String>>,
    reads_in_flight: usize,
    mutations: std::collections::BTreeMap<String, (u64, bool)>,
}

#[cfg(target_arch = "wasm32")]
struct InventoryRead {
    revision: u64,
}

#[cfg(target_arch = "wasm32")]
impl InventoryRead {
    fn begin() -> Self {
        DURABLE_STATE_INVENTORY.with(|inventory| {
            let mut inventory = inventory.borrow_mut();
            inventory.reads_in_flight += 1;
            Self {
                revision: inventory.revision,
            }
        })
    }
}

#[cfg(target_arch = "wasm32")]
impl Drop for InventoryRead {
    fn drop(&mut self) {
        DURABLE_STATE_INVENTORY.with(|inventory| {
            let mut inventory = inventory.borrow_mut();
            inventory.reads_in_flight -= 1;
            if inventory.reads_in_flight == 0 {
                inventory.mutations.clear();
            }
        });
    }
}

#[cfg(target_arch = "wasm32")]
struct IndexedDbSnapshot {
    entries: Vec<JsValue>,
    keys: Result<std::collections::BTreeSet<String>, String>,
}

#[cfg(target_arch = "wasm32")]
thread_local! {
    static RUNTIME_STATE_PRELOADED: std::cell::RefCell<bool> = const { std::cell::RefCell::new(false) };
    static DURABLE_STATE_INVENTORY: std::cell::RefCell<DurableStateInventory> = const {
        std::cell::RefCell::new(DurableStateInventory {
            revision: 0,
            keys: None,
            reads_in_flight: 0,
            mutations: std::collections::BTreeMap::new(),
        })
    };
}

#[cfg(target_arch = "wasm32")]
async fn hydrate_local_storage_from_indexed_db_prefixes(prefixes: &[&str]) -> Result<(), JsValue> {
    let already = RUNTIME_STATE_PRELOADED.with(|loaded| *loaded.borrow());
    let complete =
        DURABLE_STATE_INVENTORY.with(|inventory| matches!(inventory.borrow().keys, Some(Ok(_))));
    if already && complete {
        return Ok(());
    }

    let read = InventoryRead::begin();
    let snapshot = match indexed_db_list_entries().await {
        Ok(snapshot) => snapshot,
        Err(error) => {
            DURABLE_STATE_INVENTORY.with(|inventory| {
                inventory.borrow_mut().keys = Some(Err("IndexedDB listing failed".into()));
            });
            // An inventory retry must not change the old one-shot hydration contract.
            return if already { Ok(()) } else { Err(error) };
        }
    };
    finish_preload(
        snapshot,
        prefixes,
        already,
        read.revision,
        local_storage_set_item,
    );
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn finish_preload(
    snapshot: IndexedDbSnapshot,
    prefixes: &[&str],
    already: bool,
    revision: u64,
    mut write: impl FnMut(&str, &str) -> Result<(), JsValue>,
) {
    if !already {
        for entry in snapshot.entries {
            if !Array::is_array(&entry) {
                continue;
            }
            let pair = Array::from(&entry);
            if pair.length() != 2 {
                continue;
            }
            let key = pair.get(0).as_string();
            let value = pair.get(1).as_string();
            let (Some(key), Some(value)) = (key, value) else {
                continue;
            };
            if prefixes.iter().any(|prefix| key.starts_with(prefix)) {
                // Preserve best-effort hydration on every network. Mainnet inspects the
                // durable key inventory directly, even when this copy fails or is skipped.
                let _ = write(&key, &value);
            }
        }
    }
    DURABLE_STATE_INVENTORY.with(|inventory| {
        let mut inventory = inventory.borrow_mut();
        inventory.keys = Some(snapshot.keys.map(|mut keys| {
            // A listing is an atomic readonly transaction, but successful writes can
            // finish while its promise is pending. Overlay the last committed operation
            // for each key since this read began; parallel reads retain the journal.
            for (key, (changed_at, present)) in &inventory.mutations {
                if *changed_at > revision {
                    if *present {
                        keys.insert(key.clone());
                    } else {
                        keys.remove(key);
                    }
                }
            }
            keys
        }));
    });
    RUNTIME_STATE_PRELOADED.with(|loaded| *loaded.borrow_mut() = true);
}

#[cfg(target_arch = "wasm32")]
fn record_durable_mutation(key: &str, present: bool) {
    DURABLE_STATE_INVENTORY.with(|inventory| {
        let mut inventory = inventory.borrow_mut();
        inventory.revision = inventory.revision.wrapping_add(1);
        if inventory.reads_in_flight != 0 {
            let revision = inventory.revision;
            inventory
                .mutations
                .insert(key.to_owned(), (revision, present));
        }
        if let Some(Ok(keys)) = inventory.keys.as_mut() {
            if present {
                keys.insert(key.to_owned());
            } else {
                keys.remove(key);
            }
        }
    });
}

/// Conservative, read-only preflight. Synchronous node constructors can inspect durable
/// state only after the caller has completed the existing asynchronous preload. The
/// inventory is session-local; it does not discover writes made later by another tab.
pub(crate) fn check_mainnet_runtime_state(
    keys: &crate::wasm_node_persistence::RuntimeScopeKeys,
) -> Result<(), JsValue> {
    #[cfg(target_arch = "wasm32")]
    {
        if !RUNTIME_STATE_PRELOADED.with(|loaded| *loaded.borrow()) {
            return Err(JsValue::from_str(
                "Mainnet node initialization requires await sdk.preloadPersistentRuntimeState() before construction or wallet attachment",
            ));
        }
        let storage = web_sys::window()
            .ok_or_else(|| JsValue::from_str("browser window unavailable"))?
            .local_storage()?
            .ok_or_else(|| {
                JsValue::from_str("localStorage unavailable for mainnet recovery review")
            })?;
        let runtime = &keys.ldk_manager_registry_key;
        let protected = [
            keys.ldk_runtime_committed_storage_key.clone(),
            keys.native_ln_runtime_core_storage_base.clone(),
            keys.chain_sync_storage_key.clone(),
            keys.runtime_events_storage_key.clone(),
            keys.rgb_ln_transfers_storage_key.clone(),
            keys.peer_sessions_storage_key.clone(),
            format!("rln:wasm:ldk-broadcast-queue:{runtime}"),
            format!("rln:wasm:ldk-monitors:{runtime}"),
            format!("rln:wasm:ldk-sweeps:{runtime}"),
            format!("rln:ldk-kv:{runtime}"),
        ];
        let is_protected = |key: &str| {
            protected.iter().any(|prefix| {
                key == prefix.as_str()
                    || key
                        .strip_prefix(prefix.as_str())
                        .is_some_and(|suffix| suffix.starts_with(':'))
            })
        };
        let durable_protected = DURABLE_STATE_INVENTORY.with(|inventory| {
            let inventory = inventory.borrow();
            match inventory.keys.as_ref() {
                Some(Ok(keys)) => Ok(keys.iter().any(|key| is_protected(key))),
                Some(Err(reason)) => Err(JsValue::from_str(&format!(
                    "MainnetLightningState: Existing Lightning state requires recovery review before starting this mainnet wallet without Lightning: incomplete durable browser state inventory ({reason})"
                ))),
                None => Err(JsValue::from_str(
                    "Mainnet node initialization requires await sdk.preloadPersistentRuntimeState() before construction or wallet attachment",
                )),
            }
        })?;
        if durable_protected {
            return Err(JsValue::from_str(
                "MainnetLightningState: Existing Lightning state requires recovery review before starting this mainnet wallet without Lightning: protected browser runtime state is present",
            ));
        }
        for index in 0..storage.length()? {
            if let Some(key) = storage.key(index)? {
                if is_protected(&key) {
                    return Err(JsValue::from_str(
                        "MainnetLightningState: Existing Lightning state requires recovery review before starting this mainnet wallet without Lightning: protected browser runtime state is present",
                    ));
                }
            }
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = keys;
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
async fn hydrate_local_storage_from_indexed_db_prefixes(_prefixes: &[&str]) -> Result<(), JsValue> {
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn persist_to_indexed_db_background(key: String, value: String) {
    spawn_local(async move {
        if let Err(err) = indexed_db_set_item(&key, &value).await {
            web_sys::console::warn_1(&JsValue::from_str(&format!(
                "background IndexedDB persist failed for {key}: {err:?}"
            )));
        }
    });
}

/// Durable, awaitable IndexedDB write. Unlike `set`, the caller can observe the
/// result (used to complete deferred LDK monitor persists when localStorage fails).
#[cfg(target_arch = "wasm32")]
pub(crate) async fn indexed_db_set_durable(key: String, value: String) -> Result<(), JsValue> {
    indexed_db_set_item(&key, &value).await
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) async fn indexed_db_set_durable(_key: String, _value: String) -> Result<(), JsValue> {
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn remove_from_indexed_db_background(key: String) {
    spawn_local(async move {
        let _ = indexed_db_delete_item(&key).await;
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn persist_to_indexed_db_background(_key: String, _value: String) {}

#[cfg(not(target_arch = "wasm32"))]
fn remove_from_indexed_db_background(_key: String) {}

#[cfg(target_arch = "wasm32")]
fn local_storage_get_item(key: &str) -> Result<Option<String>, JsValue> {
    let Some(window) = web_sys::window() else {
        return Ok(None);
    };
    let Some(storage) = window.local_storage()? else {
        return Ok(None);
    };
    storage.get_item(key)
}

#[cfg(not(target_arch = "wasm32"))]
fn local_storage_get_item(_key: &str) -> Result<Option<String>, JsValue> {
    Ok(None)
}

#[cfg(target_arch = "wasm32")]
fn local_storage_set_item(key: &str, value: &str) -> Result<(), JsValue> {
    let Some(window) = web_sys::window() else {
        return Ok(());
    };
    let Some(storage) = window.local_storage()? else {
        return Ok(());
    };
    storage.set_item(key, value)
}

#[cfg(target_arch = "wasm32")]
fn local_storage_remove_item(key: &str) -> Result<(), JsValue> {
    let Some(window) = web_sys::window() else {
        return Ok(());
    };
    let Some(storage) = window.local_storage()? else {
        return Ok(());
    };
    storage.remove_item(key)
}

#[cfg(not(target_arch = "wasm32"))]
fn local_storage_set_item(_key: &str, _value: &str) -> Result<(), JsValue> {
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn local_storage_remove_item(_key: &str) -> Result<(), JsValue> {
    Ok(())
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(inline_js = r#"
export function __rln_runtime_idb_set(key, value) {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open("rln_wasm_sdk_runtime", 1);
    req.onupgradeneeded = () => {
      const db = req.result;
      if (!db.objectStoreNames.contains("state")) db.createObjectStore("state");
    };
    req.onerror = () => reject(req.error || new Error("indexedDB open failed"));
    req.onsuccess = () => {
      const db = req.result;
      const tx = db.transaction("state", "readwrite");
      const store = tx.objectStore("state");
      const putReq = store.put(value, key);
      putReq.onerror = () => reject(putReq.error || new Error("indexedDB put failed"));
      tx.oncomplete = () => { db.close(); resolve(undefined); };
      tx.onerror = () => { db.close(); reject(tx.error || new Error("indexedDB tx failed")); };
      tx.onabort = () => { db.close(); reject(tx.error || new Error("indexedDB tx aborted")); };
    };
  });
}

export function __rln_runtime_idb_entries() {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open("rln_wasm_sdk_runtime", 1);
    req.onupgradeneeded = () => {
      const db = req.result;
      if (!db.objectStoreNames.contains("state")) db.createObjectStore("state");
    };
    req.onerror = () => reject(req.error || new Error("indexedDB open failed"));
    req.onsuccess = () => {
      const db = req.result;
      const tx = db.transaction("state", "readonly");
      const store = tx.objectStore("state");
      const entriesReq = store.getAll();
      const keysReq = store.getAllKeys();
      entriesReq.onerror = () => reject(entriesReq.error || new Error("indexedDB getAll failed"));
      keysReq.onerror = () => reject(keysReq.error || new Error("indexedDB getAllKeys failed"));
      tx.oncomplete = () => {
        const keys = keysReq.result || [];
        const vals = entriesReq.result || [];
        const complete = Array.isArray(keysReq.result) && Array.isArray(entriesReq.result)
          && keys.length === vals.length;
        const out = [];
        for (let i = 0; i < keys.length; i += 1) {
          out.push([String(keys[i]), typeof vals[i] === "string" ? vals[i] : ""]);
        }
        db.close();
        resolve({ entries: out, keys, complete });
      };
      tx.onerror = () => { db.close(); reject(tx.error || new Error("indexedDB tx failed")); };
      tx.onabort = () => { db.close(); reject(tx.error || new Error("indexedDB tx aborted")); };
    };
  });
}

export function __rln_runtime_idb_delete(key) {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open("rln_wasm_sdk_runtime", 1);
    req.onupgradeneeded = () => {
      const db = req.result;
      if (!db.objectStoreNames.contains("state")) db.createObjectStore("state");
    };
    req.onerror = () => reject(req.error || new Error("indexedDB open failed"));
    req.onsuccess = () => {
      const db = req.result;
      const tx = db.transaction("state", "readwrite");
      const store = tx.objectStore("state");
      const delReq = store.delete(key);
      delReq.onerror = () => reject(delReq.error || new Error("indexedDB delete failed"));
      tx.oncomplete = () => { db.close(); resolve(undefined); };
      tx.onerror = () => { db.close(); reject(tx.error || new Error("indexedDB tx failed")); };
      tx.onabort = () => { db.close(); reject(tx.error || new Error("indexedDB tx aborted")); };
    };
  });
}
"#)]
extern "C" {
    fn __rln_runtime_idb_set(key: &str, value: &str) -> js_sys::Promise;
    fn __rln_runtime_idb_entries() -> js_sys::Promise;
    fn __rln_runtime_idb_delete(key: &str) -> js_sys::Promise;
}

#[cfg(target_arch = "wasm32")]
async fn indexed_db_set_item(key: &str, value: &str) -> Result<(), JsValue> {
    let promise = __rln_runtime_idb_set(key, value);
    let _ = JsFuture::from(promise).await?;
    record_durable_mutation(key, true);
    Ok(())
}

#[cfg(target_arch = "wasm32")]
async fn indexed_db_list_entries() -> Result<IndexedDbSnapshot, JsValue> {
    let promise = __rln_runtime_idb_entries();
    let value = JsFuture::from(promise).await?;
    Ok(parse_indexed_db_snapshot(&value))
}

#[cfg(target_arch = "wasm32")]
fn parse_indexed_db_snapshot(value: &JsValue) -> IndexedDbSnapshot {
    let entries =
        js_sys::Reflect::get(value, &JsValue::from_str("entries")).unwrap_or(JsValue::UNDEFINED);
    let entries_valid = Array::is_array(&entries);
    let entries = if entries_valid {
        Array::from(&entries).to_vec()
    } else {
        Vec::new()
    };
    let raw_keys =
        js_sys::Reflect::get(value, &JsValue::from_str("keys")).unwrap_or(JsValue::UNDEFINED);
    let complete = js_sys::Reflect::get(value, &JsValue::from_str("complete"))
        .ok()
        .and_then(|value| value.as_bool())
        == Some(true);
    let keys = (|| {
        if !complete || !entries_valid || !Array::is_array(&raw_keys) {
            return Err("malformed or incomplete IndexedDB listing".into());
        }
        let raw_keys = Array::from(&raw_keys);
        if raw_keys.length() as usize != entries.len() {
            return Err("IndexedDB key and value counts differ".into());
        }
        let mut keys = std::collections::BTreeSet::new();
        for (index, entry) in entries.iter().enumerate() {
            let key = raw_keys
                .get(index as u32)
                .as_string()
                .ok_or("non-string IndexedDB key")?;
            if !Array::is_array(entry) {
                return Err("malformed IndexedDB entry".into());
            }
            let pair = Array::from(entry);
            if pair.length() != 2
                || pair.get(0).as_string().as_ref() != Some(&key)
                || pair.get(1).as_string().is_none()
                || !keys.insert(key)
            {
                return Err("malformed or inconsistent IndexedDB entry".into());
            }
        }
        Ok(keys)
    })();
    IndexedDbSnapshot { entries, keys }
}

#[cfg(target_arch = "wasm32")]
async fn indexed_db_delete_item(key: &str) -> Result<(), JsValue> {
    let promise = __rln_runtime_idb_delete(key);
    let _ = JsFuture::from(promise).await?;
    record_durable_mutation(key, false);
    Ok(())
}

#[cfg(all(test, target_arch = "wasm32"))]
pub(crate) fn reset_preload_readiness_for_tests() {
    RUNTIME_STATE_PRELOADED.with(|loaded| *loaded.borrow_mut() = false);
    DURABLE_STATE_INVENTORY
        .with(|inventory| *inventory.borrow_mut() = DurableStateInventory::default());
}
