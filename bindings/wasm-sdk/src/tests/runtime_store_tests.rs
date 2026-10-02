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
    assert_eq!(RUNTIME_STATE_HYDRATE_PREFIXES, must_include);
    for prefix in must_include {
        assert!(
            RUNTIME_STATE_HYDRATE_PREFIXES.contains(&prefix),
            "missing hydrate prefix: {prefix}"
        );
    }
}

#[cfg(target_arch = "wasm32")]
mod browser {
    use super::super::*;
    use wasm_bindgen_test::wasm_bindgen_test;

    fn scope(name: &str) -> crate::wasm_node_persistence::RuntimeScopeKeys {
        crate::wasm_node_persistence::RuntimeScopeKeys::from_runtime_scope_key(format!(
            "ws://runtime-inventory-{name}.invalid#runtime:identity"
        ))
    }

    fn snapshot_value(keys: &[&str]) -> JsValue {
        let raw_keys = Array::new();
        let entries = Array::new();
        for key in keys {
            raw_keys.push(&JsValue::from_str(key));
            let pair = Array::new();
            pair.push(&JsValue::from_str(key));
            pair.push(&JsValue::from_str("preserved bytes"));
            entries.push(&pair);
        }
        let value = js_sys::Object::new();
        js_sys::Reflect::set(&value, &"keys".into(), &raw_keys).unwrap();
        js_sys::Reflect::set(&value, &"entries".into(), &entries).unwrap();
        js_sys::Reflect::set(&value, &"complete".into(), &JsValue::TRUE).unwrap();
        value.into()
    }

    fn mainnet_error(keys: &crate::wasm_node_persistence::RuntimeScopeKeys) -> String {
        check_mainnet_runtime_state(keys)
            .unwrap_err()
            .as_string()
            .unwrap()
    }

    #[wasm_bindgen_test(async)]
    async fn mainnet_preflight_detects_durable_kv_and_sweeps_without_hydrating_them() {
        let keys = scope("protected");
        let protected = [
            format!(
                "rln:ldk-kv:{}:monitors:monitor_updates:pending",
                keys.ldk_manager_registry_key
            ),
            format!("rln:wasm:ldk-sweeps:{}", keys.ldk_manager_registry_key),
        ];
        let storage = web_sys::window().unwrap().local_storage().unwrap().unwrap();
        for key in &protected {
            indexed_db_set_item(key, "preserve-remote-format-bytes")
                .await
                .unwrap();
            storage.remove_item(key).unwrap();
        }
        reset_preload_readiness_for_tests();
        preload_runtime_state_from_persistent_store().await.unwrap();
        for key in &protected {
            assert_eq!(storage.get_item(key).unwrap(), None);
        }
        assert!(mainnet_error(&keys).starts_with("MainnetLightningState:"));
        let durable = indexed_db_list_entries().await.unwrap();
        for key in &protected {
            assert!(durable.entries.iter().any(|entry| {
                let pair = Array::from(entry);
                pair.get(0).as_string().as_ref() == Some(key)
                    && pair.get(1).as_string().as_deref() == Some("preserve-remote-format-bytes")
            }));
            indexed_db_delete_item(key).await.unwrap();
        }
        check_mainnet_runtime_state(&keys).unwrap();
    }

    #[wasm_bindgen_test]
    fn mainnet_preflight_uses_inventory_when_best_effort_copy_fails() {
        let keys = scope("copy-error");
        let protected = keys.chain_sync_storage_key.clone();
        let allowed = "rln:wasm:media:mainnet-copy-error";
        let value = snapshot_value(&[allowed, &protected]);
        reset_preload_readiness_for_tests();
        let mut attempted = Vec::new();
        finish_preload(
            parse_indexed_db_snapshot(&value),
            RUNTIME_STATE_HYDRATE_PREFIXES,
            false,
            0,
            |key, _| {
                attempted.push(key.to_owned());
                Err(JsValue::from_str("simulated quota failure"))
            },
        );
        assert_eq!(attempted, [allowed.to_owned(), protected]);
        assert!(RUNTIME_STATE_PRELOADED.with(|loaded| *loaded.borrow()));
        assert!(mainnet_error(&keys).contains("protected browser runtime state"));

        // Copy failures for unrelated data must not become a new Mainnet refusal.
        reset_preload_readiness_for_tests();
        finish_preload(
            parse_indexed_db_snapshot(&snapshot_value(&[allowed])),
            RUNTIME_STATE_HYDRATE_PREFIXES,
            false,
            0,
            |_, _| Err(JsValue::from_str("simulated quota failure")),
        );
        check_mainnet_runtime_state(&keys).unwrap();
    }

    #[wasm_bindgen_test]
    fn mainnet_preflight_rejects_malformed_or_incomplete_inventory() {
        let keys = scope("invalid-inventory");
        let missing = js_sys::Object::new().into();
        let incomplete = snapshot_value(&[]);
        js_sys::Reflect::set(&incomplete, &"complete".into(), &JsValue::FALSE).unwrap();
        let mismatched_count = snapshot_value(&["valid-key"]);
        js_sys::Reflect::set(&mismatched_count, &"entries".into(), &Array::new()).unwrap();
        let non_string_key = snapshot_value(&["valid-key"]);
        let raw_keys = Array::new();
        raw_keys.push(&JsValue::from_f64(7.0));
        js_sys::Reflect::set(&non_string_key, &"keys".into(), &raw_keys).unwrap();
        let malformed_entry = snapshot_value(&["valid-key"]);
        let entries = Array::new();
        entries.push(&JsValue::NULL);
        js_sys::Reflect::set(&malformed_entry, &"entries".into(), &entries).unwrap();
        let inconsistent_key = snapshot_value(&["valid-key"]);
        let raw_keys = Array::new();
        raw_keys.push(&JsValue::from_str("different-key"));
        js_sys::Reflect::set(&inconsistent_key, &"keys".into(), &raw_keys).unwrap();
        for value in [
            missing,
            incomplete,
            mismatched_count,
            non_string_key,
            malformed_entry,
            inconsistent_key,
        ] {
            reset_preload_readiness_for_tests();
            finish_preload(
                parse_indexed_db_snapshot(&value),
                RUNTIME_STATE_HYDRATE_PREFIXES,
                false,
                0,
                |_, _| Ok(()),
            );
            assert!(RUNTIME_STATE_PRELOADED.with(|loaded| *loaded.borrow()));
            assert!(mainnet_error(&keys).contains("incomplete durable browser state inventory"));
        }
        reset_preload_readiness_for_tests();
        assert!(mainnet_error(&keys).contains("preloadPersistentRuntimeState"));
    }

    #[wasm_bindgen_test(async)]
    async fn mainnet_inventory_tracks_successful_durable_writes_and_deletes() {
        let keys = scope("mutations");
        let key = format!("rln:ldk-kv:{}:manager", keys.ldk_manager_registry_key);
        reset_preload_readiness_for_tests();
        preload_runtime_state_from_persistent_store().await.unwrap();
        check_mainnet_runtime_state(&keys).unwrap();
        indexed_db_set_durable(key.clone(), "new durable state".into())
            .await
            .unwrap();
        assert!(mainnet_error(&keys).contains("protected browser runtime state"));
        indexed_db_delete_item(&key).await.unwrap();
        check_mainnet_runtime_state(&keys).unwrap();
    }

    #[wasm_bindgen_test(async)]
    async fn mainnet_inventory_merges_mutations_during_overlapping_preloads() {
        let keys = scope("concurrent-write");
        let key = format!("rln:ldk-kv:{}:manager", keys.ldk_manager_registry_key);
        reset_preload_readiness_for_tests();
        let first = InventoryRead::begin();
        let first_snapshot = indexed_db_list_entries().await.unwrap();
        indexed_db_set_durable(key.clone(), "first committed write".into())
            .await
            .unwrap();
        let second = InventoryRead::begin();
        let second_snapshot = indexed_db_list_entries().await.unwrap();
        indexed_db_delete_item(&key).await.unwrap();
        finish_preload(
            second_snapshot,
            RUNTIME_STATE_HYDRATE_PREFIXES,
            false,
            second.revision,
            |_, _| Ok(()),
        );
        drop(second);
        // The write-then-delete overlay removes the key from the second snapshot.
        check_mainnet_runtime_state(&keys).unwrap();
        assert_eq!(
            DURABLE_STATE_INVENTORY.with(|inventory| inventory.borrow().reads_in_flight),
            1
        );

        indexed_db_set_durable(key.clone(), "write after deletion".into())
            .await
            .unwrap();
        finish_preload(
            first_snapshot,
            RUNTIME_STATE_HYDRATE_PREFIXES,
            true,
            first.revision,
            |_, _| panic!("an inventory-only refresh must not hydrate again"),
        );
        drop(first);
        // The delete-then-write overlay preserves the latest committed protected key,
        // even though the first snapshot predates both writes.
        assert!(mainnet_error(&keys).contains("protected browser runtime state"));
        DURABLE_STATE_INVENTORY.with(|inventory| {
            let inventory = inventory.borrow();
            assert_eq!(inventory.reads_in_flight, 0);
            assert!(
                inventory.mutations.is_empty(),
                "completed reads must not retain tombstones"
            );
        });
        indexed_db_delete_item(&key).await.unwrap();
        check_mainnet_runtime_state(&keys).unwrap();
    }

    #[wasm_bindgen_test]
    fn mainnet_preflight_checks_current_local_storage_and_scope_boundaries() {
        let keys = scope("local-state");
        let protected = format!("rln:ldk-kv:{}", keys.ldk_manager_registry_key);
        let neighbor = format!("{protected}-other-scope:manager");
        reset_preload_readiness_for_tests();
        finish_preload(
            parse_indexed_db_snapshot(&snapshot_value(&[&neighbor])),
            RUNTIME_STATE_HYDRATE_PREFIXES,
            false,
            0,
            |_, _| Ok(()),
        );
        check_mainnet_runtime_state(&keys).unwrap();
        let storage = web_sys::window().unwrap().local_storage().unwrap().unwrap();
        storage
            .set_item(&protected, "new local-only state")
            .unwrap();
        assert!(mainnet_error(&keys).contains("protected browser runtime state"));
        assert_eq!(
            storage.get_item(&protected).unwrap().as_deref(),
            Some("new local-only state")
        );
        storage.remove_item(&protected).unwrap();
    }
}
