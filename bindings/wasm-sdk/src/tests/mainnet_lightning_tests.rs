use super::*;
use crate::peer_session::has_peer_manager_hooks;
use crate::{RlnWasmSdk, RlnWasmSdkNodeHandle, RlnWasmWallet};
use wasm_bindgen_test::wasm_bindgen_test;

fn assert_mainnet_rejection<T>(result: Result<T, JsValue>) {
    let Err(error) = result else {
        panic!("mainnet Lightning call unexpectedly succeeded");
    };
    assert_eq!(
        error.as_string().as_deref(),
        Some("LightningUnsupportedOnMainnet: RLN on mainnet currently supports only on-chain methods. Lightning APIs are not supported.")
    );
}

fn configured_node(network: &str) -> RlnWasmNode {
    RlnWasmNode::new_with_node_runtime_id(
        "ws://mainnet-guard.invalid".to_string(),
        format!("mainnet-guard-{network}"),
        network.to_string(),
    )
    .expect("configured node")
}

async fn mainnet_wallet() -> RlnWasmWallet {
    let mut wallet_data: serde_json::Value =
        serde_json::from_str(&crate::test_utils::test_wallet_data_json()).unwrap();
    let keys = rgb_lib_wasm::restore_keys(
        rgb_lib_wasm::BitcoinNetwork::Mainnet,
        wallet_data["mnemonic"].as_str().unwrap().to_string(),
    )
    .expect("mainnet keys");
    wallet_data["bitcoin_network"] = serde_json::json!("Mainnet");
    wallet_data["supported_schemas"] = serde_json::json!(["Nia"]);
    wallet_data["account_xpub_vanilla"] = serde_json::json!(keys.account_xpub_vanilla);
    wallet_data["account_xpub_colored"] = serde_json::json!(keys.account_xpub_colored);
    RlnWasmWallet::create(&wallet_data.to_string())
        .await
        .expect("mainnet wallet")
}

#[wasm_bindgen_test(async)]
async fn mainnet_lightning_operations_reject_before_runtime_or_state_changes() {
    crate::test_utils::reset_wasm_runtime_state_for_tests();
    crate::runtime_store::preload_runtime_state_from_persistent_store()
        .await
        .unwrap();
    let node = configured_node("mainnet");
    assert!(node.lightning.borrow().is_none());
    let runtime_before = node.ldk_runtime_status_json().unwrap();
    let core_before = node.native_runtime_core_status_json().unwrap();

    assert_mainnet_rejection(node.connect_peer(String::new(), String::new()).await);
    assert_mainnet_rejection(node.disconnect_peer(String::new()).await);
    assert_mainnet_rejection(node.reconnect_persisted_peers_value().await);
    assert_mainnet_rejection(node.reconnect_manager_start_value());
    assert_mainnet_rejection(node.auto_drive_start_value(0));
    assert_mainnet_rejection(node.chain_sync_tick_value().await);
    assert_mainnet_rejection(node.chain_sync_start_value(String::new(), None));
    assert_mainnet_rejection(node.chain_sync_enqueue_rebroadcast_tx(String::new(), String::new()));
    assert_mainnet_rejection(node.persist_ldk_runtime_state());
    assert_mainnet_rejection(node.install_auto_peer_manager_hooks());
    assert_mainnet_rejection(
        node.configure_ldk_vss_replication(String::new(), String::new(), String::new())
            .await,
    );
    assert_mainnet_rejection(node.list_peers_value());
    assert_mainnet_rejection(node.list_channels_value());
    assert_mainnet_rejection(node.open_channel_value(String::new(), 0, false, None, None));
    assert_mainnet_rejection(node.close_channel(String::new()));
    assert_mainnet_rejection(node.get_channel_id(String::new()));
    assert_mainnet_rejection(node.list_pending_funding_requests_value());
    assert_mainnet_rejection(node.submit_funding_transaction_value(JsValue::NULL));
    assert_mainnet_rejection(node.close_all_peers().await);
    assert_mainnet_rejection(node.drain_native_runtime_queue_value());
    assert_mainnet_rejection(node.process_native_runtime_queue_value());
    assert_mainnet_rejection(node.fail_pending_payments_api());
    assert_mainnet_rejection(node.send_payment_value(String::new(), None, None, None));
    assert_mainnet_rejection(node.send_payment_live_value(String::new(), None, None, None));
    assert_mainnet_rejection(node.keysend_value(String::new(), 0, None, None));
    assert_mainnet_rejection(node.keysend_live_value(String::new(), 0, None, None));
    assert_mainnet_rejection(node.live_payment_value(String::new()));
    assert_mainnet_rejection(node.live_payments_value());
    assert_mainnet_rejection(node.list_payments_value());
    assert_mainnet_rejection(node.list_rgb_ln_transfers_value());
    assert_mainnet_rejection(node.get_payment_value(String::new()));
    assert_mainnet_rejection(node.update_payment_status(String::new(), String::new()));
    assert_mainnet_rejection(node.decode_ln_invoice_value(String::new()));
    assert_mainnet_rejection(node.create_ln_invoice_value(None, 0, None, None));
    assert_mainnet_rejection(node.create_ln_invoice_live_value(None, 0, None, None));
    assert_mainnet_rejection(node.create_hodl_ln_invoice_value(None, 0, None, None, String::new()));
    assert_mainnet_rejection(node.cancel_hodl_invoice_value(String::new()));
    assert_mainnet_rejection(node.claim_hodl_invoice_value(String::new(), String::new()));
    assert_mainnet_rejection(node.invoice_status_value(String::new()));
    assert_mainnet_rejection(node.update_payment_status_by_invoice(String::new(), String::new()));
    assert_mainnet_rejection(node.ingest_read_event_payload_hex(String::new()));
    assert_mainnet_rejection(node.ingest_runtime_transport_event_payload_hex_value(String::new()));
    assert_mainnet_rejection(node.drive_rgb_funding_work().await);
    assert_mainnet_rejection(node.process_pending_rgb_transactions().await);
    assert_mainnet_rejection(node.apay_new_value(String::new()).await);
    assert_mainnet_rejection(
        node.apay_new_with_address_value(String::new(), String::new(), String::new())
            .await,
    );

    assert_eq!(node.ldk_runtime_status_json().unwrap(), runtime_before);
    assert_eq!(node.native_runtime_core_status_json().unwrap(), core_before);
    assert!(node.lightning.borrow().is_none());
    assert!(node.channels.borrow().is_empty());
    assert!(node.payments.borrow().is_empty());
    assert!(node.runtime_events.borrow().is_empty());
    assert!(!*node.reconnect_manager_running.borrow());
    assert!(!*node.auto_drive_running.borrow());
}

#[wasm_bindgen_test(async)]
async fn mainnet_lightning_error_propagates_through_facade_and_json_handles() {
    crate::test_utils::reset_wasm_runtime_state_for_tests();
    crate::runtime_store::preload_runtime_state_from_persistent_store()
        .await
        .unwrap();
    let node = configured_node("MAINNET");
    let sdk = RlnWasmSdk::new();
    assert_mainnet_rejection(sdk.list_channels_json(&node));
    assert_mainnet_rejection(sdk.decode_ln_invoice_json(&node, String::new()));
    let handle = RlnWasmSdkNodeHandle { inner: node };
    assert_mainnet_rejection(handle.list_payments_json());
    assert_mainnet_rejection(handle.get_channel_id(String::new()));
}

#[wasm_bindgen_test(async)]
async fn non_mainnet_lightning_queries_and_validation_are_unchanged() {
    for network in ["testnet", "testnet4", "signet", "regtest"] {
        crate::test_utils::reset_wasm_runtime_state_for_tests();
        crate::runtime_store::preload_runtime_state_from_persistent_store()
            .await
            .unwrap();
        let node = configured_node(network);
        node.check_lightning_supported().expect("supported network");
        assert_eq!(node.list_channels_json().expect("list channels"), "[]");
        assert_eq!(
            node.decode_ln_invoice_value(String::new())
                .unwrap_err()
                .as_string()
                .as_deref(),
            Some(sdk_contracts::ERR_INVOICE_EMPTY)
        );
    }
}

#[wasm_bindgen_test(async)]
async fn mainnet_wallet_remains_available_and_adopted_network_restricts_lightning() {
    crate::test_utils::reset_wasm_runtime_state_for_tests();
    crate::runtime_store::preload_runtime_state_from_persistent_store()
        .await
        .unwrap();
    let wallet = mainnet_wallet().await;
    assert!(wallet
        .get_address()
        .expect("on-chain address")
        .starts_with("bc1"));
    let node = RlnWasmNode::new("ws://mainnet-wallet-guard.invalid".to_string()).unwrap();
    // Model an already-running reconnect manager without starting timers or opening sockets.
    *node.reconnect_manager_running.borrow_mut() = true;
    node.attach_wallet(&wallet).expect("adopt mainnet wallet");
    assert_eq!(node.network.borrow().as_str(), "mainnet");
    let backoff_before = *node.reconnect_manager_backoff_ms.borrow();
    assert_mainnet_rejection(node.reconnect_manager_on_resume());
    assert_eq!(*node.reconnect_manager_backoff_ms.borrow(), backoff_before);
    assert!(node.peers.borrow().is_empty());
    *node.reconnect_manager_running.borrow_mut() = false;
    {
        let _busy_wallet = wallet.inner.borrow_mut();
        assert_mainnet_rejection(node.list_channels_json());
    }
    // The node-owned cold bridge follows adopted policy without installing global hooks.
    let bridge = node.bridge.clone();
    assert_mainnet_rejection(
        bridge
            .connect_session(String::new(), String::new(), String::new())
            .await,
    );
    assert_mainnet_rejection(
        wallet
            .build_lightning_funding_tx_value(JsValue::NULL, "00".to_string(), 1, 1)
            .await,
    );
    // The on-chain decoder retains its existing validation error, rather than a Lightning error.
    assert_eq!(
        node.decode_rgb_invoice_value(String::new())
            .unwrap_err()
            .as_string()
            .as_deref(),
        Some(sdk_contracts::ERR_INVOICE_EMPTY)
    );
    assert!(wallet
        .get_address()
        .expect("on-chain address after rejection")
        .starts_with("bc1"));
}

#[wasm_bindgen_test(async)]
async fn mainnet_refuses_saved_running_chain_state_without_resuming_or_changing_it() {
    crate::test_utils::reset_wasm_runtime_state_for_tests();
    crate::runtime_store::preload_runtime_state_from_persistent_store()
        .await
        .unwrap();
    let proxy = "ws://mainnet-legacy-chain.invalid";
    let runtime_id = "mainnet-legacy-chain";
    let keys = RuntimeScopeKeys::from_runtime_scope_key(runtime_scope_key(proxy, Some(runtime_id)));
    let storage = web_sys::window().unwrap().local_storage().unwrap().unwrap();
    let saved = r#"{"network":"regtest","running":true,"indexer_url":"http://127.0.0.1:1"}"#;
    storage
        .set_item(&keys.chain_sync_storage_key, saved)
        .unwrap();
    let result = RlnWasmNode::new_with_node_runtime_id(
        proxy.to_string(),
        runtime_id.to_string(),
        "mainnet".to_string(),
    );
    let error = result.err().expect("protected saved state must refuse");
    assert!(error
        .as_string()
        .unwrap()
        .starts_with("MainnetLightningState:"));
    assert_eq!(
        storage
            .get_item(&keys.chain_sync_storage_key)
            .unwrap()
            .as_deref(),
        Some(saved)
    );
    storage.remove_item(&keys.chain_sync_storage_key).unwrap();
}

#[wasm_bindgen_test(async)]
async fn mainnet_peer_bridge_rejects_before_opening_a_socket() {
    crate::test_utils::reset_wasm_runtime_state_for_tests();
    crate::runtime_store::preload_runtime_state_from_persistent_store()
        .await
        .unwrap();
    let node = configured_node("mainnet");
    let bridge = node.bridge.clone();
    // An invalid WebSocket URL would fail during socket creation if the guard ran too late.
    assert_mainnet_rejection(
        bridge
            .connect_session(String::new(), String::new(), String::new())
            .await,
    );
    assert_mainnet_rejection(
        bridge
            .connect_session_with_options(
                String::new(),
                String::new(),
                String::new(),
                JsValue::NULL,
            )
            .await,
    );
    // Mainnet construction never installs process-global hooks.
    clear_rln_ldk_peer_manager_hooks();
    assert!(!has_peer_manager_hooks());
}

#[wasm_bindgen_test(async)]
async fn lightning_guard_does_not_borrow_busy_onchain_wallet() {
    crate::test_utils::reset_wasm_runtime_state_for_tests();
    crate::runtime_store::preload_runtime_state_from_persistent_store()
        .await
        .unwrap();
    let wallet = RlnWasmWallet::create(&crate::test_utils::test_wallet_data_json())
        .await
        .expect("regtest wallet");
    let node = RlnWasmNode::new("ws://busy-wallet-guard.invalid".to_string()).unwrap();
    node.attach_wallet(&wallet).unwrap();
    let _busy_wallet = wallet.inner.borrow_mut();
    node.check_lightning_supported()
        .expect("non-mainnet remains supported");
    assert_eq!(
        node.decode_ln_invoice_value(String::new())
            .unwrap_err()
            .as_string()
            .as_deref(),
        Some(sdk_contracts::ERR_INVOICE_EMPTY)
    );
}

#[wasm_bindgen_test(async)]
async fn mainnet_reconnect_resume_rejects_through_node_and_wrappers() {
    crate::test_utils::reset_wasm_runtime_state_for_tests();
    crate::runtime_store::preload_runtime_state_from_persistent_store()
        .await
        .unwrap();
    let node = configured_node("mainnet");
    let sdk = RlnWasmSdk::new();
    assert_mainnet_rejection(node.reconnect_manager_on_resume());
    assert_mainnet_rejection(sdk.reconnect_manager_on_resume(&node));
    let handle = RlnWasmSdkNodeHandle { inner: node };
    assert_mainnet_rejection(handle.reconnect_manager_on_resume());
    assert!(!*handle.inner.reconnect_manager_running.borrow());

    for network in ["testnet", "testnet4", "signet", "regtest"] {
        crate::test_utils::reset_wasm_runtime_state_for_tests();
        crate::runtime_store::preload_runtime_state_from_persistent_store()
            .await
            .unwrap();
        let node = configured_node(network);
        node.reconnect_manager_on_resume()
            .expect("inactive reconnect remains a no-op on supported networks");
        sdk.reconnect_manager_on_resume(&node)
            .expect("facade preserves supported-network behavior");
        let handle = RlnWasmSdkNodeHandle { inner: node };
        handle
            .reconnect_manager_on_resume()
            .expect("handle preserves supported-network behavior");
        assert!(!*handle.inner.reconnect_manager_running.borrow());
    }
}

#[wasm_bindgen_test(async)]
async fn node_peer_bridges_keep_their_network_in_both_creation_orders() {
    for mainnet_first in [false, true] {
        crate::test_utils::reset_wasm_runtime_state_for_tests();
        crate::runtime_store::preload_runtime_state_from_persistent_store()
            .await
            .unwrap();
        let (mainnet, regtest) = if mainnet_first {
            let mainnet = configured_node("mainnet");
            (mainnet, configured_node("regtest"))
        } else {
            let regtest = configured_node("regtest");
            (configured_node("mainnet"), regtest)
        };
        assert_mainnet_rejection(
            mainnet
                .bridge
                .connect_session(String::new(), String::new(), String::new())
                .await,
        );
        assert_mainnet_rejection(
            mainnet
                .bridge
                .connect_session_with_options(
                    String::new(),
                    String::new(),
                    String::new(),
                    JsValue::NULL,
                )
                .await,
        );
        for result in [
            regtest
                .bridge
                .connect_session(String::new(), String::new(), String::new())
                .await,
            regtest
                .bridge
                .connect_session_with_options(
                    String::new(),
                    String::new(),
                    String::new(),
                    JsValue::NULL,
                )
                .await,
        ] {
            let Err(error) = result else {
                panic!("empty peer pubkey unexpectedly accepted");
            };
            assert_eq!(
                error.as_string().as_deref(),
                Some(sdk_contracts::ERR_PEER_PUBKEY_EMPTY),
                "another node's mainnet policy must not replace this node's validation"
            );
        }
    }
}

#[wasm_bindgen_test(async)]
async fn mainnet_constructor_and_shared_calls_never_enter_lightning_factories() {
    crate::test_utils::reset_wasm_runtime_state_for_tests();
    crate::runtime_store::preload_runtime_state_from_persistent_store()
        .await
        .unwrap();
    let before = test_utils::startup_calls();
    let hooks_before = has_peer_manager_hooks();
    let scopes_before = KNOWN_RUNTIME_SCOPE_KEYS.with(|scopes| scopes.borrow().clone());
    let node = configured_node("mainnet");
    let info: serde_json::Value = serde_json::from_str(&node.node_info_json().unwrap()).unwrap();
    assert_eq!(info["ldk_over_websocket"], false);
    assert_eq!(info["num_channels"], 0);
    assert!(info["runtime"].as_str().unwrap().ends_with(":disabled"));
    let status: serde_json::Value =
        serde_json::from_str(&node.ldk_runtime_status_json().unwrap()).unwrap();
    assert_eq!(status["ready"], false);
    assert_eq!(status["storage_initialized"], false);
    node.ldk_runtime_components_json().unwrap();
    node.native_runtime_core_status_json().unwrap();
    node.chain_sync_status_json().unwrap();
    node.chain_sync_stop_json().unwrap();
    node.ldk_vss_backup_info_json().unwrap();
    assert!(node
        .network_info_value()
        .unwrap_err()
        .as_string()
        .unwrap()
        .starts_with("NetworkInfoUnavailable:"));
    let expected_identity =
        derive_node_signing_identity("ws://mainnet-guard.invalid", Some("mainnet-guard-mainnet"))
            .unwrap();
    let pubkey: serde_json::Value =
        serde_json::from_str(&node.node_pubkey_json().unwrap()).unwrap();
    assert_eq!(pubkey["pubkey"], expected_identity.1.to_string());
    let signed: serde_json::Value =
        serde_json::from_str(&node.sign_message_json("on-chain mainnet".into()).unwrap()).unwrap();
    let bytes = hex::decode(signed["signed_message"].as_str().unwrap()).unwrap();
    let signature = secp256k1::ecdsa::RecoverableSignature::from_compact(
        &bytes[..64],
        secp256k1::ecdsa::RecoveryId::from_i32(bytes[64] as i32).unwrap(),
    )
    .unwrap();
    let message =
        SecpMessage::from_digest_slice(&Sha256::hash(b"on-chain mainnet").to_byte_array()).unwrap();
    assert_eq!(
        Secp256k1::new()
            .recover_ecdsa(&message, &signature)
            .unwrap(),
        expected_identity.1
    );
    assert!(node.lightning.borrow().is_none());
    drop(node);
    assert_eq!(test_utils::startup_calls(), before);
    assert_eq!(has_peer_manager_hooks(), hooks_before);
    assert_eq!(
        KNOWN_RUNTIME_SCOPE_KEYS.with(|scopes| scopes.borrow().clone()),
        scopes_before
    );
}

#[wasm_bindgen_test(async)]
async fn mainnet_preload_is_required_before_constructor_or_adoption_mutates_scope() {
    crate::test_utils::reset_wasm_runtime_state_for_tests();
    let wallet = mainnet_wallet().await;
    crate::runtime_store::reset_preload_readiness_for_tests();
    let before = test_utils::startup_calls();
    let bare = RlnWasmNode::new("ws://mainnet-preload-adoption.invalid".into()).unwrap();
    assert!(bare
        .attach_wallet(&wallet)
        .unwrap_err()
        .as_string()
        .unwrap()
        .contains("preloadPersistentRuntimeState"));
    assert_eq!(bare.configured_network.borrow().as_str(), "unknown");
    assert!(bare.wallet.borrow().is_none());
    assert!(bare.lightning.borrow().is_none());
    let proxy = "ws://mainnet-preload.invalid".to_string();
    let result =
        RlnWasmNode::new_with_node_runtime_id(proxy.clone(), "preload".into(), "mainnet".into());
    assert!(result
        .err()
        .unwrap()
        .as_string()
        .unwrap()
        .contains("preloadPersistentRuntimeState"));
    assert_eq!(test_utils::startup_calls(), before);
    crate::runtime_store::preload_runtime_state_from_persistent_store()
        .await
        .unwrap();
    let node =
        RlnWasmNode::new_with_node_runtime_id(proxy, "preload".into(), "mainnet".into()).unwrap();
    assert!(node.lightning.borrow().is_none());
}

#[wasm_bindgen_test(async)]
async fn mainnet_unknown_scope_adoption_and_failed_vss_setup_stay_cold() {
    crate::test_utils::reset_wasm_runtime_state_for_tests();
    crate::runtime_store::preload_runtime_state_from_persistent_store()
        .await
        .unwrap();
    let before = test_utils::startup_calls();
    let proxy = "ws://mainnet-unknown.invalid";
    let node =
        RlnWasmNode::new_with_runtime_id_opt(proxy.into(), Some("shared".into()), None).unwrap();
    let cold_error = node
        .bridge
        .connect_session(String::new(), String::new(), String::new())
        .await
        .err()
        .unwrap();
    assert_eq!(
        cold_error.as_string().as_deref(),
        Some("Lightning runtime is not initialized")
    );
    assert_eq!(test_utils::startup_calls(), before);
    node.node_info_value().unwrap();
    node.node_pubkey_value().unwrap();
    assert!(node
        .configure_ldk_vss_replication(String::new(), String::new(), String::new())
        .await
        .is_err());
    assert_eq!(node.configured_network.borrow().as_str(), "unknown");
    let explicit =
        RlnWasmNode::new_with_node_runtime_id(proxy.into(), "shared".into(), "mainnet".into())
            .unwrap();
    assert_eq!(node.configured_network.borrow().as_str(), "mainnet");
    assert_mainnet_rejection(node.list_channels_value());
    assert!(Rc::ptr_eq(&node.runtime_scope, &explicit.runtime_scope));
    assert_eq!(test_utils::startup_calls(), before);
}

#[wasm_bindgen_test(async)]
async fn mainnet_legacy_protected_namespaces_refuse_without_modification() {
    crate::test_utils::reset_wasm_runtime_state_for_tests();
    crate::runtime_store::preload_runtime_state_from_persistent_store()
        .await
        .unwrap();
    let proxy = "ws://mainnet-legacy-namespaces.invalid";
    let id = "recovery";
    let keys = RuntimeScopeKeys::from_runtime_scope_key(runtime_scope_key(proxy, Some(id)));
    let runtime = &keys.ldk_manager_registry_key;
    let storage = web_sys::window().unwrap().local_storage().unwrap().unwrap();
    let before = test_utils::startup_calls();
    let _existing =
        RlnWasmNode::new_with_node_runtime_id(proxy.into(), id.into(), "mainnet".into()).unwrap();
    for key in [
        keys.ldk_runtime_committed_storage_key.clone(),
        format!("{}:pending", keys.ldk_runtime_committed_storage_key),
        format!("rln:wasm:ldk-monitors:{runtime}:monitor:funding"),
        format!("rln:wasm:ldk-broadcast-queue:{runtime}"),
        format!("rln:wasm:ldk-sweeps:{runtime}"),
        format!("rln:ldk-kv:{runtime}:monitors:monitor_updates:key"),
        keys.peer_sessions_storage_key.clone(),
    ] {
        storage
            .set_item(&key, "unknown-or-corrupt-legacy-state")
            .unwrap();
        let result =
            RlnWasmNode::new_with_node_runtime_id(proxy.into(), id.into(), "mainnet".into());
        assert!(result
            .err()
            .unwrap()
            .as_string()
            .unwrap()
            .starts_with("MainnetLightningState:"));
        assert_eq!(
            storage.get_item(&key).unwrap().as_deref(),
            Some("unknown-or-corrupt-legacy-state")
        );
        let inherited = RlnWasmNode::new_with_runtime_id_opt(proxy.into(), Some(id.into()), None);
        assert!(inherited
            .err()
            .unwrap()
            .as_string()
            .unwrap()
            .starts_with("MainnetLightningState:"));
        storage.remove_item(&key).unwrap();
    }
    assert_eq!(test_utils::startup_calls(), before);
}

#[wasm_bindgen_test]
fn supported_scope_reuse_never_reseeds_or_replaces_runtime_and_conflicts_are_atomic() {
    crate::test_utils::reset_wasm_runtime_state_for_tests();
    let proxy = "ws://runtime-reuse.invalid";
    let node =
        RlnWasmNode::new_with_node_runtime_id(proxy.into(), "shared".into(), "regtest".into())
            .unwrap();
    node.ensure_runtime_ready().unwrap();
    let runtime = node.lightning_runtime().unwrap();
    let before = test_utils::startup_calls();
    let reused =
        RlnWasmNode::new_with_node_runtime_id(proxy.into(), "shared".into(), "regtest".into())
            .unwrap();
    assert!(Rc::ptr_eq(&runtime, &reused.lightning_runtime().unwrap()));
    // A handle may install its own bridge hooks, but must not acquire a new manager/core/driver.
    let after = test_utils::startup_calls();
    for component in [
        "manager",
        "seed_assignment",
        "runtime_core",
        "chain_driver",
        "live_graph",
        "chain_task",
    ] {
        assert_eq!(before.get(component), after.get(component));
    }
    assert!(
        RlnWasmNode::new_with_node_runtime_id(proxy.into(), "shared".into(), "signet".into())
            .is_err()
    );
    assert_eq!(node.configured_network.borrow().as_str(), "regtest");
    drop(reused);
    assert!(runtime.ldk_runtime.status().ready);
    drop(runtime);
    drop(node);
}

#[wasm_bindgen_test(async)]
async fn failed_bare_runtime_preparation_does_not_commit_regtest_or_register_hooks() {
    use wasm_bindgen::JsCast;
    crate::test_utils::reset_wasm_runtime_state_for_tests();
    crate::runtime_store::preload_runtime_state_from_persistent_store()
        .await
        .unwrap();
    let proxy = "ws://runtime-preparation-failure.invalid";
    let node =
        RlnWasmNode::new_with_runtime_id_opt(proxy.into(), Some("rollback".into()), None).unwrap();
    let hooks_before = has_peer_manager_hooks();
    let prototype = js_sys::Reflect::get(&js_sys::global(), &JsValue::from_str("Storage")).unwrap();
    let prototype = js_sys::Reflect::get(&prototype, &JsValue::from_str("prototype")).unwrap();
    let original = js_sys::Reflect::get(&prototype, &JsValue::from_str("getItem")).unwrap();
    let replacement = js_sys::Function::new_with_args("original, blocked", "return function(key) { if (key === blocked) throw new Error('injected read failure'); return original.call(this, key); };")
        .call2(&JsValue::NULL, &original, &JsValue::from_str(&node.persistence_keys.chain_sync_storage_key)).unwrap();
    js_sys::Reflect::set(&prototype, &JsValue::from_str("getItem"), &replacement).unwrap();
    let result = node.ensure_runtime_ready();
    js_sys::Reflect::set(&prototype, &JsValue::from_str("getItem"), &original).unwrap();
    assert!(result.is_err());
    assert_eq!(node.configured_network.borrow().as_str(), "unknown");
    assert!(node.lightning.borrow().is_none());
    assert_eq!(has_peer_manager_hooks(), hooks_before);
    // No provisional node-owned lease survives the failure. Mainnet adoption remains possible.
    let mainnet =
        RlnWasmNode::new_with_node_runtime_id(proxy.into(), "rollback".into(), "mainnet".into())
            .unwrap();
    assert_eq!(node.configured_network.borrow().as_str(), "mainnet");
    assert!(mainnet.lightning.borrow().is_none());
    // Ensure the preserved method remains callable after restoration.
    let _: js_sys::Function = original.unchecked_into();
}

#[wasm_bindgen_test(async)]
async fn node_drop_releases_only_its_runtime_and_preserves_a_surviving_bridge() {
    crate::test_utils::reset_wasm_runtime_state_for_tests();
    crate::runtime_store::preload_runtime_state_from_persistent_store()
        .await
        .unwrap();
    let first = RlnWasmNode::new_with_node_runtime_id(
        "ws://drop-owner.invalid".into(),
        "first".into(),
        "regtest".into(),
    )
    .unwrap();
    first.ensure_runtime_ready().unwrap();
    let second = RlnWasmNode::new_with_node_runtime_id(
        "ws://drop-owner.invalid".into(),
        "second".into(),
        "regtest".into(),
    )
    .unwrap();
    second.ensure_runtime_ready().unwrap();
    let second_manager = Rc::clone(&second.lightning_runtime().unwrap().ldk_runtime);
    assert!(second_manager.status().ready);
    drop(second);
    assert!(!second_manager.status().ready);
    assert!(
        first
            .lightning_runtime()
            .unwrap()
            .ldk_runtime
            .status()
            .ready
    );
    assert_eq!(first.bridge.connection_hooks_ready().unwrap(), (true, true));
    let mainnet = configured_node("mainnet");
    drop(mainnet);
    assert_eq!(first.bridge.connection_hooks_ready().unwrap(), (true, true));
    // A stale callback may still retain the old manager. It must not reserve the
    // released scope or cause the replacement to reseed that manager.
    let replacement = RlnWasmNode::new_with_node_runtime_id(
        "ws://drop-owner.invalid".into(),
        "second".into(),
        "regtest".into(),
    )
    .unwrap();
    assert!(!Rc::ptr_eq(
        &second_manager,
        &replacement.lightning_runtime().unwrap().ldk_runtime
    ));
    assert!(!second_manager.status().ready);
}
