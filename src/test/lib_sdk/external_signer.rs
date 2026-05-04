//! External-signer `lib_sdk` tests use [`NativeExternalSigner`] (real PSBT signing) behind a small
//! [`ExternalSignerHost`] wrapper for availability / `SignRgbPsbt` failure injection. The wire format
//! matches production (`rgb_lightning_node::signer_integration_wire` → `signer::proto`).
use crate::helpers::*;
use rgb_lightning_node::signer_integration_wire::{decode_signer_request_wire, SignerRequest};
use serial_test::serial;
use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;

fn clone_bootstrap(b: &SdkExternalSignerBootstrap) -> SdkExternalSignerBootstrap {
    SdkExternalSignerBootstrap {
        node_id: b.node_id.clone(),
        account_xpub_vanilla: b.account_xpub_vanilla.clone(),
        account_xpub_colored: b.account_xpub_colored.clone(),
        master_fingerprint: b.master_fingerprint.clone(),
        protocol_version: b.protocol_version.clone(),
        api_level: b.api_level,
        ldk_inbound_payment_key_hex: b.ldk_inbound_payment_key_hex.clone(),
        ldk_peer_storage_key_hex: b.ldk_peer_storage_key_hex.clone(),
        ldk_receive_auth_key_hex: b.ldk_receive_auth_key_hex.clone(),
        async_payments_root_seed_hex: b.async_payments_root_seed_hex.clone(),
    }
}

/// Wraps [`NativeExternalSigner`] so tests can simulate signer outage or `SignRgbPsbt` rejection.
struct TunableNativeSignerHost {
    inner: Arc<rgb_lightning_node::NativeExternalSigner>,
    available: Arc<AtomicBool>,
    /// When set, [`SignerRequest::SignRgbPsbt`] returns a host error (simulates signer / transport failure).
    fail_sign_rgb_psbt: Arc<AtomicBool>,
}

impl TunableNativeSignerHost {
    fn new(inner: Arc<rgb_lightning_node::NativeExternalSigner>) -> Self {
        Self {
            inner,
            available: Arc::new(AtomicBool::new(true)),
            fail_sign_rgb_psbt: Arc::new(AtomicBool::new(false)),
        }
    }

    fn set_available(&self, available: bool) {
        self.available.store(available, Ordering::Relaxed);
    }

    fn set_fail_sign_rgb_psbt(&self, fail: bool) {
        self.fail_sign_rgb_psbt.store(fail, Ordering::Relaxed);
    }
}

impl rgb_lightning_node::ExternalSignerHost for TunableNativeSignerHost {
    fn call(&self, request: Vec<u8>) -> Result<Vec<u8>, rgb_lightning_node::RlnError> {
        if !self.available.load(Ordering::Relaxed) {
            return Err(rgb_lightning_node::RlnError::Internal);
        }
        if self.fail_sign_rgb_psbt.load(Ordering::Relaxed) {
            let req = decode_signer_request_wire(&request)
                .map_err(|_| rgb_lightning_node::RlnError::Internal)?;
            if matches!(&req, SignerRequest::SignRgbPsbt { .. }) {
                return Err(rgb_lightning_node::RlnError::Internal);
            }
        }
        self.inner.call(request)
    }
}

fn attach_external_signer_host(
    node: &SdkNode,
    host: Arc<dyn rgb_lightning_node::ExternalSignerHost>,
    bootstrap: &SdkExternalSignerBootstrap,
) {
    node.attach_external_signer(host, clone_bootstrap(bootstrap))
        .expect("attach external signer");
}

fn unlock_with_attached_external_signer(
    node: &SdkNode,
    bootstrap: &SdkExternalSignerBootstrap,
    announce_alias: &str,
) {
    node.unlock_with_attached_external_signer(
        clone_bootstrap(bootstrap),
        "user".to_string(),
        "password".to_string(),
        "localhost".to_string(),
        18443,
        Some("127.0.0.1:50001".to_string()),
        Some(PROXY_ENDPOINT_LOCAL.to_string()),
        vec![],
        Some(announce_alias.to_string()),
    )
    .expect("unlock with attached external signer");
}

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn make_native_signer(seed_byte: u8) -> Arc<rgb_lightning_node::NativeExternalSigner> {
    rgb_lightning_node::NativeExternalSigner::new(
        format!("{seed_byte:02x}").repeat(32),
        "regtest".to_string(),
        Some(true),
    )
    .expect("create native signer")
}

#[test]
#[serial]
fn external_init_unlock_and_restart_same_signer() {
    ensure_regtest_available();
    let _guard = env_lock().lock().expect("env lock");

    let test_dir = test_dir("sdk_external_signer_init_unlock_restart");
    if test_dir.exists() {
        fs::remove_dir_all(&test_dir).expect("remove previous lib_sdk test dir");
    }
    fs::create_dir_all(&test_dir).expect("create lib_sdk test dir");
    let node_dir = test_dir.join("node_a");

    let signer = make_native_signer(0x11);
    let bootstrap = signer.bootstrap().expect("bootstrap");

    let node = make_node(&node_dir, NODE_A_DAEMON_PORT + 110, NODE_A_PEER_PORT + 110);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        node.init_with_native_external_signer(signer.clone())
            .expect("external init");
        node.unlock_with_native_external_signer(
            signer.clone(),
            "user".to_string(),
            "password".to_string(),
            "localhost".to_string(),
            18443,
            Some("127.0.0.1:50001".to_string()),
            Some(PROXY_ENDPOINT_LOCAL.to_string()),
            vec![],
            Some("RLN_external".to_string()),
        )
        .expect("unlock with native external signer");

        let info = node.node_info().expect("node_info");
        assert_eq!(info.pubkey.to_string(), bootstrap.node_id);

        node.shutdown();
        thread::sleep(Duration::from_millis(500));

        let restarted = make_node(&node_dir, NODE_A_DAEMON_PORT + 110, NODE_A_PEER_PORT + 110);
        restarted
            .unlock_with_native_external_signer(
                signer.clone(),
                "user".to_string(),
                "password".to_string(),
                "localhost".to_string(),
                18443,
                Some("127.0.0.1:50001".to_string()),
                Some(PROXY_ENDPOINT_LOCAL.to_string()),
                vec![],
                Some("RLN_external".to_string()),
            )
            .expect("unlock after restart");
        let info2 = restarted.node_info().expect("node_info after restart");
        assert_eq!(info2.pubkey.to_string(), bootstrap.node_id);
        restarted.shutdown();
    }));

    if result.is_err() {
        panic!("external signer SDK test failed");
    }
}

#[test]
#[serial]
fn external_restart_with_mismatched_signer_fails_unlock() {
    ensure_regtest_available();
    let _guard = env_lock().lock().expect("env lock");

    let test_dir = test_dir("sdk_external_signer_restart_mismatch");
    if test_dir.exists() {
        fs::remove_dir_all(&test_dir).expect("remove previous lib_sdk test dir");
    }
    fs::create_dir_all(&test_dir).expect("create lib_sdk test dir");
    let node_dir = test_dir.join("node_a");

    let signer_a = make_native_signer(0x11);
    let node = make_node(&node_dir, NODE_A_DAEMON_PORT + 111, NODE_A_PEER_PORT + 111);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        node.init_with_native_external_signer(signer_a.clone())
            .expect("external init");
        node.unlock_with_native_external_signer(
            signer_a.clone(),
            "user".to_string(),
            "password".to_string(),
            "localhost".to_string(),
            18443,
            Some("127.0.0.1:50001".to_string()),
            Some(PROXY_ENDPOINT_LOCAL.to_string()),
            vec![],
            Some("RLN_external".to_string()),
        )
        .expect("unlock");
        node.shutdown();
        thread::sleep(Duration::from_millis(500));

        let signer_b = make_native_signer(0x22);

        let restarted = make_node(&node_dir, NODE_A_DAEMON_PORT + 111, NODE_A_PEER_PORT + 111);
        let err = restarted
            .unlock_with_native_external_signer(
                signer_b,
                "user".to_string(),
                "password".to_string(),
                "localhost".to_string(),
                18443,
                Some("127.0.0.1:50001".to_string()),
                Some(PROXY_ENDPOINT_LOCAL.to_string()),
                vec![],
                Some("RLN_external".to_string()),
            )
            .expect_err("unlock must fail for mismatched signer");
        let msg = err.to_string().to_lowercase();
        assert!(
            msg.contains("conflict") || msg.contains("mismatch"),
            "unexpected error for mismatched signer: {msg}"
        );
        restarted.shutdown();
    }));

    if result.is_err() {
        panic!("external signer mismatch SDK test failed");
    }
}

#[test]
#[serial]
fn external_signer_connection_loss_and_restore_mock() {
    ensure_regtest_available();
    let _guard = env_lock().lock().expect("env lock");

    let test_dir = test_dir("sdk_external_signer_connection_loss_restore_mock");
    if test_dir.exists() {
        fs::remove_dir_all(&test_dir).expect("remove previous lib_sdk test dir");
    }
    fs::create_dir_all(&test_dir).expect("create lib_sdk test dir");
    let node_dir = test_dir.join("node_a");

    let signer = make_native_signer(0x44);
    let bootstrap = signer.bootstrap().expect("bootstrap");
    let host = Arc::new(TunableNativeSignerHost::new(signer));

    let node = make_node(&node_dir, NODE_A_DAEMON_PORT + 121, NODE_A_PEER_PORT + 121);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        node.init_with_external_signer(clone_bootstrap(&bootstrap))
            .expect("external init");
        attach_external_signer_host(&node, host.clone(), &bootstrap);
        unlock_with_attached_external_signer(&node, &bootstrap, "RLN_external_conn_loss");

        fund_and_create_utxos(&node, "node A mock");
        let self_addr = node.address().expect("node address").address;

        host.set_available(false);
        let err = node
            .sendbtc(SdkSendBtcRequest {
                address: self_addr.clone(),
                amount: 1_000,
                fee_rate: CREATE_UTXOS_FEE_RATE,
                skip_sync: false,
            })
            .err()
            .expect("send_btc must fail while signer is unavailable");
        let msg = err.to_string().to_lowercase();
        assert!(
            msg.contains("external signer")
                || msg.contains("status")
                || msg.contains("transport")
                || msg.contains("unavailable")
                || msg.contains("internal"),
            "unexpected error while signer unavailable: {msg}"
        );

        host.set_available(true);
        node.sendbtc(SdkSendBtcRequest {
            address: self_addr,
            amount: 1_000,
            fee_rate: CREATE_UTXOS_FEE_RATE,
            skip_sync: false,
        })
        .expect("send_btc must recover after signer is back");
        node.shutdown();
    }));

    if result.is_err() {
        panic!("external signer connection loss/restore mock test failed");
    }
}

/// When the host rejects [`SignerRequest::SignRgbPsbt`], the node must surface the error (no silent
/// success via a local RGB wallet PSBT path). Uses `sendbtc` after `createutxos` so the flow reaches
/// `rgb_sign_psbt` (a second identical `createutxos` can fail earlier with insufficient coins before
/// signing).
#[test]
#[serial]
fn external_signer_sign_rgb_psbt_failure_surfaces_on_send_btc_mock() {
    ensure_regtest_available();
    let _guard = env_lock().lock().expect("env lock");

    let test_dir = test_dir("sdk_external_signer_sign_rgb_psbt_fail_mock");
    if test_dir.exists() {
        fs::remove_dir_all(&test_dir).expect("remove previous lib_sdk test dir");
    }
    fs::create_dir_all(&test_dir).expect("create lib_sdk test dir");
    let node_dir = test_dir.join("node_a");

    let signer = make_native_signer(0x55);
    let bootstrap = signer.bootstrap().expect("bootstrap");
    let host = Arc::new(TunableNativeSignerHost::new(signer));

    let node = make_node(&node_dir, NODE_A_DAEMON_PORT + 131, NODE_A_PEER_PORT + 131);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        node.init_with_external_signer(clone_bootstrap(&bootstrap))
            .expect("external init");
        attach_external_signer_host(&node, host.clone(), &bootstrap);
        unlock_with_attached_external_signer(&node, &bootstrap, "RLN_external_sign_rgb_fail");

        fund_and_create_utxos(&node, "node A mock");
        let self_addr = node.address().expect("node address").address;

        host.set_fail_sign_rgb_psbt(true);
        let err = node
            .sendbtc(SdkSendBtcRequest {
                address: self_addr.clone(),
                amount: 1_000,
                fee_rate: CREATE_UTXOS_FEE_RATE,
                skip_sync: false,
            })
            .err()
            .expect("sendbtc must fail when SignRgbPsbt is rejected");
        let msg = err.to_string().to_lowercase();
        assert!(
            msg.contains("unexpected")
                || msg.contains("rgb")
                || msg.contains("external")
                || msg.contains("transport")
                || msg.contains("sign")
                || msg.contains("internal")
                || msg.contains("psbt"),
            "unexpected error for SignRgbPsbt host failure: {msg}"
        );

        host.set_fail_sign_rgb_psbt(false);
        node.sendbtc(SdkSendBtcRequest {
            address: self_addr,
            amount: 1_000,
            fee_rate: CREATE_UTXOS_FEE_RATE,
            skip_sync: false,
        })
        .expect("sendbtc must succeed again after clearing SignRgbPsbt failure");
        node.shutdown();
    }));

    if result.is_err() {
        panic!("external signer SignRgbPsbt failure lib_sdk test failed");
    }
}
