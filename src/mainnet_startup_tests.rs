//! Successful wallet startup against a local mainnet indexer, with unused Lightning endpoints.

use crate::{
    args::UserArgs,
    core_types::LdkChainSync,
    error::APIError,
    routes, sdk,
    utils::{start_daemon, AppState},
};
use axum::{
    routing::{get, post},
    Json, Router,
};
use bitcoin::consensus::encode::serialize_hex;
use rgb_lib::BitcoinNetwork;
use serde_json::{json, Value};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
    task::JoinHandle,
};

const PASSWORD: &str = "mainnet-test-password";
const MNEMONIC: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

pub(crate) struct Indexer {
    pub(crate) url: String,
    requests: Arc<Mutex<Vec<String>>>,
    unexpected: Arc<Mutex<Vec<String>>>,
    task: JoinHandle<()>,
}
impl Drop for Indexer {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Indexer {
    pub(crate) async fn new(network: bitcoin::Network) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("tcp://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let unexpected = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&requests);
        let errors = Arc::clone(&unexpected);
        let task = tokio::spawn(async move {
            let mut connections = tokio::task::JoinSet::new();
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let seen = Arc::clone(&seen);
                let errors = Arc::clone(&errors);
                connections.spawn(async move {
                    let (reader, mut writer) = stream.into_split();
                    let mut lines = BufReader::new(reader).lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        let request: Value = serde_json::from_str(&line).unwrap();
                        let response = |request: &Value| {
                            let method = request["method"].as_str().unwrap();
                            seen.lock().unwrap().push(method.into());
                            let genesis = bitcoin::blockdata::constants::genesis_block(network);
                            let header = serialize_hex(&genesis.header);
                            let result = match method {
                                "server.version" => json!(["fixture", "1.4"]),
                                "server.features" => json!({"server_version": "fixture", "hosts": {}, "genesis_hash": genesis.block_hash().to_string(), "hash_function": "sha256", "protocol_min": "1.4", "protocol_max": "1.4"}),
                                "blockchain.block.header" => { assert_eq!(request["params"][0], 0); json!(header) },
                                "blockchain.block.headers" => json!({"count": 1, "hex": header, "max": 2016}),
                                "blockchain.transaction.get" => if request["params"][1] == true { json!({"confirmations": 1}) } else { json!(serialize_hex(&genesis.txdata[0])) },
                                "blockchain.headers.subscribe" => json!({"height": 0, "hex": header}),
                                "blockchain.scripthash.get_history" | "blockchain.scripthash.listunspent" => json!([]),
                                "blockchain.scripthash.subscribe" => Value::Null,
                                "blockchain.estimatefee" | "blockchain.relayfee" => json!(0.00001),
                                _ => { errors.lock().unwrap().push(method.into()); Value::Null },
                            };
                            json!({"jsonrpc": "2.0", "id": request["id"], "result": result})
                        };
                        let result = if let Some(batch) = request.as_array() {
                            Value::Array(batch.iter().map(response).collect())
                        } else { response(&request) };
                        if writer.write_all(format!("{result}\n").as_bytes()).await.is_err() { break; }
                    }
                });
            }
        });
        Self {
            url,
            requests,
            unexpected,
            task,
        }
    }
}

struct Fixture {
    state: Arc<AppState>,
    _directory: tempfile::TempDir,
    indexer: Indexer,
    proxy: String,
    proxy_task: JoinHandle<()>,
    peer: TcpListener,
    backend: TcpListener,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.proxy_task.abort();
    }
}
impl Fixture {
    async fn new() -> Self {
        let fixture = Self::uninitialized().await;
        sdk::init(
            fixture.state.clone(),
            PASSWORD.into(),
            Some(MNEMONIC.into()),
        )
        .await
        .unwrap();
        fixture
    }
    async fn uninitialized() -> Self {
        Self::uninitialized_for_network(BitcoinNetwork::Mainnet).await
    }
    async fn uninitialized_for_network(network: BitcoinNetwork) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let indexer = Indexer::new(network.into()).await;
        let peer = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let backend = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy = format!("rpc://{}/json-rpc", proxy_listener.local_addr().unwrap());
        let proxy_router = Router::new().route("/json-rpc", post(|Json(request): Json<Value>| async move {
            assert_eq!(request["method"], "server.info");
            Json(json!({"jsonrpc": "2.0", "id": request["id"], "result": {"protocol_version": "0.2", "version": "fixture", "uptime": 1}}))
        }));
        let proxy_task =
            tokio::spawn(async move { axum::serve(proxy_listener, proxy_router).await.unwrap() });
        let state = start_daemon(&UserArgs {
            storage_dir_path: directory.path().to_path_buf(),
            daemon_listening_port: 0,
            ldk_peer_listening_port: if network == BitcoinNetwork::Mainnet {
                peer.local_addr().unwrap().port()
            } else {
                0
            },
            network,
            max_media_upload_size_mb: 1,
            max_aggregated_media_size_per_channel_mb: 1,
            max_pending_consignments: 10,
            max_media_files_per_channel: 10,
            root_public_key: None,
            enable_virtual_channels_v0: false,
            virtual_peer_pubkeys: vec![],
            lsp_base_url: None,
            lsp_bearer_token: None,
            vss_url: None,
            vss_allow_empty_restore: false,
            reuse_addresses: false,
            remote_signer_listen_addr: None,
            config: Default::default(),
        })
        .await
        .unwrap();
        Self {
            state,
            _directory: directory,
            indexer,
            proxy,
            proxy_task,
            peer,
            backend,
        }
    }

    fn request(&self) -> sdk::UnlockRequest {
        sdk::UnlockRequest {
            password: PASSWORD.into(),
            indexer_url: Some(self.indexer.url.clone()),
            eth_rpc_url: None,
            proxy_endpoint: Some(self.proxy.clone()),
            announce_addresses: vec![],
            announce_alias: None,
            gossip_rgs_server_url: Some(format!("http://{}", self.backend.local_addr().unwrap())),
            ldk_chain_sync: self.chain_sync(),
        }
    }
    fn chain_sync(&self) -> LdkChainSync {
        #[cfg(feature = "block-sync")]
        {
            LdkChainSync::BlockSync {
                bitcoind_rpc_username: "unused".into(),
                bitcoind_rpc_password: "unused".into(),
                bitcoind_rpc_host: "127.0.0.1".into(),
                bitcoind_rpc_port: self.backend.local_addr().unwrap().port(),
            }
        }
        #[cfg(not(feature = "block-sync"))]
        {
            LdkChainSync::TransactionSync {
                indexer_url: format!("tcp://{}", self.backend.local_addr().unwrap()),
            }
        }
    }
    async fn assert_no_lightning(&self) {
        let state = self.state.unlocked_app_state.lock().await;
        assert!(state.as_ref().unwrap().lightning.is_none());
        assert!(self.state.ldk_background_services.lock().unwrap().is_none());
        assert!(
            tokio::time::timeout(Duration::from_millis(20), self.peer.accept())
                .await
                .is_err()
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(20), self.backend.accept())
                .await
                .is_err()
        );
        assert!(
            self.indexer.unexpected.lock().unwrap().is_empty(),
            "{:?}",
            self.indexer.unexpected.lock().unwrap()
        );
        assert!(!self
            .indexer
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|method| method == "blockchain.transaction.broadcast"));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mainnet_sdk_unlock_runs_wallet_without_lightning_and_restarts() {
    let fixture = Fixture::new().await;
    let mut wrong = fixture.request();
    wrong.password = "wrong-password".into();
    assert!(matches!(
        sdk::unlock(fixture.state.clone(), wrong).await,
        Err(APIError::WrongPassword)
    ));
    assert!(!*fixture.state.changing_state.lock().unwrap());
    sdk::unlock(fixture.state.clone(), fixture.request())
        .await
        .unwrap();
    fixture.assert_no_lightning().await;
    let info = sdk::node_info(fixture.state.clone()).await.unwrap();
    assert_eq!(
        (info.num_channels, info.num_peers, info.local_balance_sat),
        (0, 0, 0)
    );
    assert_eq!(
        (
            info.network_nodes,
            info.network_channels,
            info.latest_rgs_snapshot_timestamp
        ),
        (0, 0, None)
    );
    let address = sdk::address(fixture.state.clone()).await.unwrap().address;
    assert!(address.starts_with("bc1"));
    let balance = sdk::btc_balance(fixture.state.clone(), false)
        .await
        .unwrap();
    assert_eq!((balance.vanilla.settled, balance.colored.settled), (0, 0));
    let assets = sdk::list_assets(fixture.state.clone(), vec![])
        .await
        .unwrap();
    assert!(assets.nia.unwrap().is_empty());
    assert!(sdk::list_transactions(fixture.state.clone(), true, None)
        .await
        .unwrap()
        .is_empty());
    assert!(sdk::list_unspents(fixture.state.clone(), true)
        .await
        .unwrap()
        .is_empty());
    let invoice = sdk::rgb_invoice(
        fixture.state.clone(),
        sdk::RgbInvoiceRequestData {
            asset_id: None,
            assignment_kind: None,
            assignment_amount: None,
            duration_seconds: None,
            min_confirmations: 1,
            witness: true,
        },
    )
    .await
    .unwrap();
    let decoded = sdk::decode_rgb_invoice(fixture.state.clone(), invoice.invoice)
        .await
        .unwrap();
    assert_eq!(decoded.network, BitcoinNetwork::Mainnet);
    let signature = sdk::sign_message(fixture.state.clone(), "mainnet wallet".into())
        .await
        .unwrap();
    assert!(sdk::verify_message(
        fixture.state.clone(),
        "mainnet wallet".into(),
        signature.signed_message
    )
    .await
    .unwrap());
    assert_eq!(
        sdk::network_info(fixture.state.clone())
            .await
            .unwrap()
            .height,
        0
    );
    assert!(matches!(
        sdk::list_peers(fixture.state.clone()).await,
        Err(APIError::LightningUnsupportedOnMainnet)
    ));
    fixture.assert_no_lightning().await;
    let _ = routes::lock(axum::extract::State(fixture.state.clone()))
        .await
        .unwrap();
    assert!(matches!(
        routes::lock(axum::extract::State(fixture.state.clone())).await,
        Err(APIError::LockedNode)
    ));
    sdk::unlock(fixture.state.clone(), fixture.request())
        .await
        .unwrap();
    assert_eq!(
        sdk::node_info(fixture.state.clone()).await.unwrap().pubkey,
        info.pubkey
    );
    fixture.assert_no_lightning().await;
    let _ = routes::lock(axum::extract::State(fixture.state.clone()))
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(20), fixture.peer.accept())
            .await
            .is_err()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mainnet_rest_unlock_rejects_wrong_indexer_then_serves_wallet() {
    let fixture = Fixture::new().await;
    let wrong_indexer = Indexer::new(bitcoin::Network::Regtest).await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let router = Router::new()
        .route("/unlock", post(routes::unlock))
        .route("/lock", post(routes::lock))
        .route("/address", post(routes::address))
        .route("/nodeinfo", get(routes::node_info))
        .route("/networkinfo", get(routes::network_info))
        .with_state(fixture.state.clone());
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap();
    let mut body = json!({"password": PASSWORD, "ldk_chain_sync": fixture.chain_sync(), "indexer_url": wrong_indexer.url, "proxy_endpoint": fixture.proxy, "announce_addresses": []});
    let response = client
        .post(format!("{base}/unlock"))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
    assert_eq!(
        response.json::<Value>().await.unwrap()["name"],
        "InvalidIndexer"
    );
    assert!(fixture.state.unlocked_app_state.lock().await.is_none());
    assert!(!*fixture.state.changing_state.lock().unwrap());
    body["indexer_url"] = json!(fixture.indexer.url);
    let response = client
        .post(format!("{base}/unlock"))
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = response.status();
    let result = response.text().await.unwrap();
    assert_eq!(status, reqwest::StatusCode::OK, "{result}");
    fixture.assert_no_lightning().await;
    let address: Value = client
        .post(format!("{base}/address"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(address["address"].as_str().unwrap().starts_with("bc1"));
    let info: Value = client
        .get(format!("{base}/nodeinfo"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(info["num_channels"], 0);
    let info: Value = client
        .get(format!("{base}/networkinfo"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(info["height"], 0);
    assert_eq!(
        client
            .post(format!("{base}/lock"))
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::OK
    );
    task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mainnet_legacy_state_refusal_preserves_bytes_and_does_not_contact_indexer() {
    use lightning::util::persist::KVStoreSync;
    let fixture = Fixture::new().await;
    let store = crate::kv_store::SeaOrmKvStore::from_connection(fixture.state.db());
    store
        .write("", "", "manager", b"opaque previous snapshot".to_vec())
        .unwrap();
    for _ in 0..2 {
        let result = sdk::unlock(fixture.state.clone(), fixture.request()).await;
        assert!(matches!(result, Err(APIError::MainnetLightningState(_))));
        assert!(fixture.state.unlocked_app_state.lock().await.is_none());
        assert!(!*fixture.state.changing_state.lock().unwrap());
        assert_eq!(
            store.read("", "", "manager").unwrap(),
            b"opaque previous snapshot"
        );
        assert!(fixture.indexer.requests.lock().unwrap().is_empty());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn canceled_sdk_caller_does_not_abandon_wallet_startup() {
    let fixture = Fixture::new().await;
    let state = fixture.state.clone();
    let request = fixture.request();
    let caller = tokio::spawn(async move { sdk::unlock(state, request).await });
    tokio::time::timeout(Duration::from_secs(10), async {
        while fixture.indexer.requests.lock().unwrap().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    caller.abort();
    tokio::time::timeout(Duration::from_secs(10), async {
        while *fixture.state.changing_state.lock().unwrap() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    fixture.assert_no_lightning().await;
    let _ = routes::lock(axum::extract::State(fixture.state.clone()))
        .await
        .unwrap();
}

#[cfg(all(feature = "uniffi", feature = "vls"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mainnet_attached_and_native_signer_unlock_keep_strict_policy_without_channel_calls() {
    use crate::uniffi_api::{
        ExternalSignerHost, NativeExternalSigner, RlnError, SdkLdkChainSync, SdkNode,
    };
    struct Host(Arc<NativeExternalSigner>);
    impl ExternalSignerHost for Host {
        fn call(&self, bytes: Vec<u8>) -> Result<Vec<u8>, RlnError> {
            let request = crate::signer::proto::decode_signer_request(&bytes).unwrap();
            assert!(
                !matches!(
                    request,
                    crate::signer::types::ExternalSignerRequest::Channel(_)
                ),
                "unexpected channel signing request"
            );
            self.0.call(bytes)
        }
    }
    let fixture = Fixture::uninitialized().await;
    let signer = NativeExternalSigner::new("42".repeat(32), "mainnet".into(), Some(false)).unwrap();
    let node = SdkNode {
        handle: crate::NodeHandle::from_app_state(fixture.state.clone()),
    };
    node.init_with_native_external_signer(signer.clone())
        .unwrap();
    node.attach_external_signer(Arc::new(Host(signer.clone())), signer.bootstrap().unwrap())
        .unwrap();
    sdk::unlock_with_attached_external_signer(fixture.state.clone(), fixture.request())
        .await
        .unwrap();
    fixture.assert_no_lightning().await;
    assert_eq!(
        node.node_info().unwrap().pubkey.to_string(),
        signer.bootstrap().unwrap().node_id
    );
    assert!(node.address().unwrap().address.starts_with("bc1"));
    let _ = routes::lock(axum::extract::State(fixture.state.clone()))
        .await
        .unwrap();
    let backend = fixture.backend.local_addr().unwrap();
    let chain = SdkLdkChainSync::BlockSync {
        bitcoind_rpc_username: "unused".into(),
        bitcoind_rpc_password: "unused".into(),
        bitcoind_rpc_host: "127.0.0.1".into(),
        bitcoind_rpc_port: backend.port(),
    };
    node.unlock_with_native_external_signer(
        signer,
        chain,
        Some(fixture.indexer.url.clone()),
        Some(fixture.proxy.clone()),
        vec![],
        None,
    )
    .unwrap();
    fixture.assert_no_lightning().await;
    node.shutdown();
    assert!(fixture.state.unlocked_app_state.lock().await.is_none());
    assert!(!*fixture.state.changing_state.lock().unwrap());
}

/// Exercises real LDK construction and an empty persisted-manager restart without external services.
/// Funded channels, payments, and chain advancement still require the regtest integration suite.
#[cfg(feature = "transaction-sync")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn regtest_starts_lightning_and_restores_after_lock() {
    use lightning::util::persist::{
        KVStoreSync, CHANNEL_MANAGER_PERSISTENCE_KEY,
        CHANNEL_MANAGER_PERSISTENCE_PRIMARY_NAMESPACE,
        CHANNEL_MANAGER_PERSISTENCE_SECONDARY_NAMESPACE,
    };

    let fixture = Fixture::uninitialized_for_network(BitcoinNetwork::Regtest).await;
    sdk::init(
        fixture.state.clone(),
        PASSWORD.into(),
        Some(MNEMONIC.into()),
    )
    .await
    .unwrap();
    let request = || {
        let mut request = fixture.request();
        request.gossip_rgs_server_url = None;
        request.ldk_chain_sync = LdkChainSync::TransactionSync {
            indexer_url: fixture.indexer.url.clone(),
        };
        request
    };
    let mut node_id = None;
    for _ in 0..2 {
        tokio::time::timeout(
            Duration::from_secs(30),
            sdk::unlock(fixture.state.clone(), request()),
        )
        .await
        .expect("regtest startup timed out")
        .unwrap();
        assert!(fixture
            .state
            .unlocked_app_state
            .lock()
            .await
            .as_ref()
            .unwrap()
            .lightning
            .is_some());
        assert!(fixture
            .state
            .ldk_background_services
            .lock()
            .unwrap()
            .is_some());
        let info = sdk::node_info(fixture.state.clone()).await.unwrap();
        if let Some(expected) = &node_id {
            assert_eq!(&info.pubkey, expected);
        } else {
            node_id = Some(info.pubkey);
        }
        assert!(sdk::list_peers(fixture.state.clone())
            .await
            .unwrap()
            .is_empty());
        assert!(sdk::list_channels(fixture.state.clone())
            .await
            .unwrap()
            .is_empty());
        assert!(sdk::list_payments(fixture.state.clone())
            .await
            .unwrap()
            .is_empty());
        assert!(sdk::address(fixture.state.clone())
            .await
            .unwrap()
            .address
            .starts_with("bcrt1"));
        let _ = tokio::time::timeout(
            Duration::from_secs(30),
            routes::lock(axum::extract::State(fixture.state.clone())),
        )
        .await
        .expect("regtest shutdown timed out")
        .unwrap();
        assert!(fixture.state.unlocked_app_state.lock().await.is_none());
        assert!(fixture
            .state
            .ldk_background_services
            .lock()
            .unwrap()
            .is_none());
        let store = crate::kv_store::SeaOrmKvStore::from_connection(fixture.state.db());
        assert!(!store
            .read(
                CHANNEL_MANAGER_PERSISTENCE_PRIMARY_NAMESPACE,
                CHANNEL_MANAGER_PERSISTENCE_SECONDARY_NAMESPACE,
                CHANNEL_MANAGER_PERSISTENCE_KEY,
            )
            .unwrap()
            .is_empty());
    }
    assert!(
        fixture.indexer.unexpected.lock().unwrap().is_empty(),
        "{:?}",
        fixture.indexer.unexpected.lock().unwrap()
    );
    assert!(!fixture
        .indexer
        .requests
        .lock()
        .unwrap()
        .iter()
        .any(|method| method == "blockchain.transaction.broadcast"));
}
