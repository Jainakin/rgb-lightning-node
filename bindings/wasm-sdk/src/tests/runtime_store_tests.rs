use super::RUNTIME_STATE_HYDRATE_PREFIXES;

#[test]
fn hydrate_prefixes_cover_runtime_state_domains() {
    let must_include = [
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
    for prefix in must_include {
        assert!(
            RUNTIME_STATE_HYDRATE_PREFIXES.contains(&prefix),
            "missing hydrate prefix: {prefix}"
        );
    }
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen_test::wasm_bindgen_test(async)]
async fn mainnet_preflight_hydrates_indexeddb_only_kv_and_retains_legacy_state() {
    use super::*;
    let scope = "ws://mainnet-idb-recovery.invalid#runtime:identity";
    let keys = crate::wasm_node_persistence::RuntimeScopeKeys::from_runtime_scope_key(scope.into());
    let key = format!(
        "rln:ldk-kv:{}:monitors:monitor_updates:pending",
        keys.ldk_manager_registry_key
    );
    let storage = web_sys::window().unwrap().local_storage().unwrap().unwrap();
    indexed_db_set_item(&key, "preserve-remote-format-bytes")
        .await
        .unwrap();
    storage.remove_item(&key).unwrap();
    reset_preload_readiness_for_tests();
    preload_runtime_state_from_persistent_store().await.unwrap();
    assert_eq!(
        storage.get_item(&key).unwrap().as_deref(),
        Some("preserve-remote-format-bytes")
    );
    assert!(check_mainnet_runtime_state(&keys)
        .unwrap_err()
        .as_string()
        .unwrap()
        .starts_with("MainnetLightningState:"));
    assert_eq!(
        storage.get_item(&key).unwrap().as_deref(),
        Some("preserve-remote-format-bytes")
    );
    indexed_db_delete_item(&key).await.unwrap();
    storage.remove_item(&key).unwrap();
}
