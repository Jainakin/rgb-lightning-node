use crate::helpers::*;
use rgb_lib::utils::get_account_data;
use rgb_lib::BitcoinNetwork;
use serde_json::{json, Value};
use serial_test::serial;
use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;

const NODE_ID_HEX: &str = "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";

const ENVELOPE_VERSION_V1: u32 = 1;
const ENCODING_JSON: u32 = 1;

fn encode_varint(mut v: u64, out: &mut Vec<u8>) {
    while v >= 0x80 {
        out.push((v as u8 & 0x7f) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn decode_varint(input: &[u8], pos: &mut usize) -> Result<u64, String> {
    let mut shift = 0u32;
    let mut value = 0u64;
    loop {
        if *pos >= input.len() {
            return Err("unexpected eof in varint".to_string());
        }
        let b = input[*pos];
        *pos += 1;
        value |= ((b & 0x7f) as u64) << shift;
        if b & 0x80 == 0 {
            return Ok(value);
        }
        shift += 7;
        if shift > 63 {
            return Err("varint overflow".to_string());
        }
    }
}

fn encode_request_payload(json_payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(json_payload.len() + 16);
    out.push(0x08);
    encode_varint(ENVELOPE_VERSION_V1 as u64, &mut out);
    out.push(0x10);
    encode_varint(ENCODING_JSON as u64, &mut out);
    out.push(0x1a);
    encode_varint(json_payload.len() as u64, &mut out);
    out.extend_from_slice(json_payload);
    out
}

fn decode_response_payload(wire_payload: &[u8]) -> Result<Vec<u8>, String> {
    let mut pos = 0usize;
    let mut version = None::<u32>;
    let mut encoding = None::<u32>;
    let mut payload = None::<Vec<u8>>;

    while pos < wire_payload.len() {
        let key = decode_varint(wire_payload, &mut pos)?;
        let field = (key >> 3) as u32;
        let wire = (key & 0x07) as u8;
        match (field, wire) {
            (1, 0) => {
                version = Some(decode_varint(wire_payload, &mut pos)? as u32);
            }
            (2, 0) => {
                encoding = Some(decode_varint(wire_payload, &mut pos)? as u32);
            }
            (3, 2) => {
                let len = decode_varint(wire_payload, &mut pos)? as usize;
                if pos + len > wire_payload.len() {
                    return Err("invalid payload length".to_string());
                }
                payload = Some(wire_payload[pos..pos + len].to_vec());
                pos += len;
            }
            (_, 0) => {
                let _ = decode_varint(wire_payload, &mut pos)?;
            }
            (_, 2) => {
                let len = decode_varint(wire_payload, &mut pos)? as usize;
                if pos + len > wire_payload.len() {
                    return Err("invalid skip length".to_string());
                }
                pos += len;
            }
            _ => return Err(format!("unsupported wire type: {wire}")),
        }
    }

    if version != Some(ENVELOPE_VERSION_V1) {
        return Err(format!("unsupported version: {version:?}"));
    }
    if encoding != Some(ENCODING_JSON) {
        return Err(format!("unsupported encoding: {encoding:?}"));
    }
    payload.ok_or_else(|| "missing payload".to_string())
}

fn clone_bootstrap(b: &SdkExternalSignerBootstrap) -> SdkExternalSignerBootstrap {
    SdkExternalSignerBootstrap {
        node_id: b.node_id.clone(),
        account_xpub_vanilla: b.account_xpub_vanilla.clone(),
        account_xpub_colored: b.account_xpub_colored.clone(),
        master_fingerprint: b.master_fingerprint.clone(),
        protocol_version: b.protocol_version.clone(),
        api_level: b.api_level,
    }
}

struct MockSignerHost {
    bootstrap: SdkExternalSignerBootstrap,
    available: Arc<AtomicBool>,
}

impl MockSignerHost {
    fn new(bootstrap: SdkExternalSignerBootstrap) -> Self {
        Self {
            bootstrap,
            available: Arc::new(AtomicBool::new(true)),
        }
    }

    fn set_available(&self, available: bool) {
        self.available.store(available, Ordering::Relaxed);
    }
}

impl rgb_lightning_node::ExternalSignerHost for MockSignerHost {
    fn call(&self, request: Vec<u8>) -> Result<Vec<u8>, rgb_lightning_node::RlnError> {
        if !self.available.load(Ordering::Relaxed) {
            return Err(rgb_lightning_node::RlnError::Internal);
        }
        let json_body = decode_response_payload(&request)
            .map_err(|_| rgb_lightning_node::RlnError::Internal)?;
        let req: Value = serde_json::from_slice(&json_body)
            .map_err(|_| rgb_lightning_node::RlnError::Internal)?;
        let resp = mock_response(&req, &self.bootstrap);
        let resp_body_json =
            serde_json::to_vec(&resp).map_err(|_| rgb_lightning_node::RlnError::Internal)?;
        Ok(encode_request_payload(&resp_body_json))
    }
}

fn attach_external_signer_host(
    node: &SdkNode,
    host: Arc<dyn rgb_lightning_node::ExternalSignerHost>,
    bootstrap: &SdkExternalSignerBootstrap,
) {
    node.attach_external_signer(
        host,
        bootstrap.node_id.clone(),
        bootstrap.account_xpub_vanilla.clone(),
        bootstrap.account_xpub_colored.clone(),
        bootstrap.master_fingerprint.clone(),
        bootstrap.protocol_version.clone(),
        bootstrap.api_level,
    )
    .expect("attach external signer");
}

fn unlock_with_attached_external_signer(
    node: &SdkNode,
    bootstrap: &SdkExternalSignerBootstrap,
    announce_alias: &str,
) {
    node.unlock_with_attached_external_signer(
        bootstrap.node_id.clone(),
        bootstrap.account_xpub_vanilla.clone(),
        bootstrap.account_xpub_colored.clone(),
        bootstrap.master_fingerprint.clone(),
        bootstrap.protocol_version.clone(),
        bootstrap.api_level,
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

fn test_bootstrap() -> SdkExternalSignerBootstrap {
    let mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".to_string();
    let (_, xpub_vanilla, _) =
        get_account_data(&BitcoinNetwork::Regtest, &mnemonic, false).expect("vanilla xpub");
    let (_, xpub_colored, master_fingerprint) =
        get_account_data(&BitcoinNetwork::Regtest, &mnemonic, true).expect("colored xpub");
    SdkExternalSignerBootstrap {
        node_id: NODE_ID_HEX.to_string(),
        account_xpub_vanilla: xpub_vanilla.to_string(),
        account_xpub_colored: xpub_colored.to_string(),
        master_fingerprint: master_fingerprint.to_string(),
        protocol_version: "1".to_string(),
        api_level: 1,
    }
}

fn make_native_signer(seed_byte: u8) -> Arc<rgb_lightning_node::NativeExternalSigner> {
    rgb_lightning_node::NativeExternalSigner::new(
        format!("{seed_byte:02x}").repeat(32),
        "regtest".to_string(),
        Some(true),
    )
    .expect("create native signer")
}

fn mock_response(req: &Value, bootstrap: &SdkExternalSignerBootstrap) -> Value {
    let sig64 = format!("{}01{}01", "00".repeat(31), "00".repeat(31));
    let script_hex = format!("0014{}", "11".repeat(20));
    if req == "Bootstrap" || req.get("Bootstrap").is_some() {
        return json!({
            "Bootstrap": {
                "identity": {
                    "node_id": bootstrap.node_id,
                    "account_xpub_vanilla": bootstrap.account_xpub_vanilla,
                    "account_xpub_colored": bootstrap.account_xpub_colored,
                    "master_fingerprint": bootstrap.master_fingerprint,
                },
                "protocol_version": bootstrap.protocol_version,
                "api_level": bootstrap.api_level
            }
        });
    }

    if let Some(node_req) = req.get("Node") {
        if node_req.get("GetNodeId").is_some() {
            return json!({"Node": {"NodeId": {"node_id_hex": NODE_ID_HEX}}});
        }
        if node_req.get("GetDestinationScript").is_some()
            || node_req.get("GetShutdownScriptpubkey").is_some()
        {
            return json!({"Node": {"Script": {"script_hex": script_hex}}});
        }
        if node_req.get("GetSecureRandomBytes").is_some() {
            return json!({"Node": {"RandomBytes": {"bytes_hex": "ab".repeat(32)}}});
        }
        if node_req.get("Ecdh").is_some() {
            return json!({"Node": {"Ecdh": {"shared_secret_hex": "22".repeat(32)}}});
        }
        if node_req.get("SignInvoice").is_some() {
            return json!({"Node": {"RecoverableSignature": {"signature_hex": sig64, "recovery_id": 0}}});
        }
        if node_req.get("SignBolt12Invoice").is_some()
            || node_req.get("SignGossipMessage").is_some()
            || node_req.get("SignMessage").is_some()
        {
            return json!({"Node": {"Signature": {"signature_hex": sig64}}});
        }
    }

    if let Some(ch_req) = req.get("Channel") {
        if ch_req.get("GenerateChannelKeysId").is_some() {
            return json!({"Channel": {"GeneratedChannelKeysId": {"channel_keys_id_hex": "cd".repeat(32)}}});
        }
        if ch_req.get("DeriveChannelSigner").is_some() || ch_req.get("ReadChannelSigner").is_some()
        {
            return json!({
                "Channel": {
                    "ChannelSignerData": {
                        "channel_signer_state_hex": "ef".repeat(64),
                        "channel_pubkeys": {
                            "funding_pubkey_hex": NODE_ID_HEX,
                            "revocation_basepoint_hex": NODE_ID_HEX,
                            "payment_point_hex": NODE_ID_HEX,
                            "delayed_payment_basepoint_hex": NODE_ID_HEX,
                            "htlc_basepoint_hex": NODE_ID_HEX
                        }
                    }
                }
            });
        }
        if let Some(op) = ch_req.get("Op") {
            if let Some(op_data) = op.get("op") {
                if op_data.get("GetPerCommitmentPoint").is_some() {
                    return json!({"Channel": {"PerCommitmentPoint": {"point_hex": NODE_ID_HEX}}});
                }
                if op_data.get("ReleaseCommitmentSecret").is_some() {
                    return json!({"Channel": {"CommitmentSecret": {"secret_hex": "11".repeat(32)}}});
                }
                if op_data.get("SignCounterpartyCommitment").is_some() {
                    return json!({"Channel": {"SignatureWithHtlcs": {"signature_hex": sig64, "htlc_signatures_hex": []}}});
                }
            }
            return json!({"Channel": {"Signature": {"signature_hex": sig64}}});
        }
    }

    if req.get("ChannelOp").is_some() {
        return json!({"Channel": {"Signature": {"signature_hex": sig64}}});
    }
    if let Some(v) = req.get("SignRgbPsbt") {
        return json!({"SignedPsbt": {"psbt": v.get("psbt").cloned().unwrap_or(json!(""))}});
    }
    if let Some(v) = req.get("SignSpendableOutputsPsbt") {
        return json!({"SignedPsbt": {"psbt": v.get("psbt").cloned().unwrap_or(json!(""))}});
    }
    json!({"Node": {"RandomBytes": {"bytes_hex": "ab".repeat(32)}}})
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
#[ignore = "native toggleable signer harness not implemented yet"]
fn external_signer_connection_loss_and_restore_mock() {
    // TODO: replace with a native toggleable signer test harness now that the
    // supported signer path is in-process UniFFI rather than a foreign-host callback.
    ensure_regtest_available();
    let _guard = env_lock().lock().expect("env lock");

    let test_dir = test_dir("sdk_external_signer_connection_loss_restore_mock");
    if test_dir.exists() {
        fs::remove_dir_all(&test_dir).expect("remove previous lib_sdk test dir");
    }
    fs::create_dir_all(&test_dir).expect("create lib_sdk test dir");
    let node_dir = test_dir.join("node_a");

    let host = Arc::new(MockSignerHost::new(test_bootstrap()));

    let node = make_node(&node_dir, NODE_A_DAEMON_PORT + 121, NODE_A_PEER_PORT + 121);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let bootstrap = test_bootstrap();
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
