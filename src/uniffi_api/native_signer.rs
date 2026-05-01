use super::{ExternalSignerHost, RlnError, SdkExternalSignerBootstrap};
use crate::signer::proto::{decode_signer_request, encode_signer_response};
use anyhow::Context;
use bitcoin::hex::FromHex;
use bitcoin::Network;
use signer_contract::{BootstrapData, ExternalSignerBackend, SignerRequest};
use signer_vls_adapter::vls_real::RealVlsClient;
use signer_vls_adapter::VlsSignerAdapter;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use vls_protocol::msgs;
use vls_protocol_client::{Error as VlsClientError, Transport};
use vls_protocol_signer::approver::WarningPositiveApprover;
use vls_protocol_signer::handler::{Handler, InitHandler, RootHandler};
use vls_protocol_signer::lightning_signer;
use vls_protocol_signer::lightning_signer::node::NodeServices;
use vls_protocol_signer::lightning_signer::persist::{DummyPersister, Persist};
use vls_protocol_signer::lightning_signer::policy::filter::PolicyFilter;
use vls_protocol_signer::lightning_signer::policy::simple_validator::{
    make_default_simple_policy, SimpleValidatorFactory,
};
use vls_protocol_signer::lightning_signer::signer::derive::KeyDerivationStyle;
use vls_protocol_signer::lightning_signer::signer::ClockStartingTimeFactory;
use vls_protocol_signer::lightning_signer::util::clock::StandardClock;

struct VlsTransportState {
    init_handler: Option<InitHandler>,
    root_handler: Option<RootHandler>,
    channel_handlers: HashMap<(u64, [u8; 33]), vls_protocol_signer::handler::ChannelHandler>,
    cached_hsmd_init2_reply: Option<Vec<u8>>,
}

struct InProcessVlsTransport {
    state: Mutex<VlsTransportState>,
}

impl InProcessVlsTransport {
    fn new(network: Network, seed: [u8; 32], permissive_policy: bool) -> anyhow::Result<Self> {
        let persister: Arc<dyn Persist> = Arc::new(DummyPersister {});
        let validator_factory: Arc<dyn lightning_signer::policy::validator::ValidatorFactory> =
            if permissive_policy {
                let mut policy = make_default_simple_policy(network);
                policy.filter = PolicyFilter::new_permissive();
                Arc::new(SimpleValidatorFactory::new_with_policy(policy))
            } else {
                Arc::new(SimpleValidatorFactory::new())
            };
        let services = NodeServices {
            validator_factory,
            starting_time_factory: ClockStartingTimeFactory::new(),
            persister,
            clock: Arc::new(StandardClock()),
            trusted_oracle_pubkeys: vec![],
        };

        let config = lightning_signer::node::NodeConfig {
            network,
            key_derivation_style: KeyDerivationStyle::Ldk,
            use_checkpoints: true,
            allow_deep_reorgs: true,
        };
        let node = Arc::new(lightning_signer::node::Node::new(
            config,
            &seed,
            vec![],
            services,
        ));
        let approver: Arc<dyn vls_protocol_signer::approver::Approve> = if permissive_policy {
            Arc::new(WarningPositiveApprover())
        } else {
            Arc::new(vls_protocol_signer::approver::NegativeApprover())
        };
        let handler = InitHandler::new(1, node, approver, msgs::DEFAULT_MAX_PROTOCOL_VERSION);
        Ok(Self {
            state: Mutex::new(VlsTransportState {
                init_handler: Some(handler),
                root_handler: None,
                channel_handlers: HashMap::new(),
                cached_hsmd_init2_reply: None,
            }),
        })
    }
}

impl Transport for InProcessVlsTransport {
    fn node_call(&self, message: Vec<u8>) -> Result<Vec<u8>, VlsClientError> {
        let msg_name = msgs::message_name_from_vec(&message);
        let msg = msgs::from_vec(message).map_err(|e| VlsClientError::Protocol(e.into()))?;
        let mut state = self.state.lock().map_err(|_| VlsClientError::Transport)?;

        if state.root_handler.is_none() {
            let init = state
                .init_handler
                .as_mut()
                .ok_or(VlsClientError::Transport)?;
            let (done, reply_opt) = init.handle(msg).map_err(|_| VlsClientError::Transport)?;
            let reply = reply_opt.ok_or(VlsClientError::Transport)?;
            let reply_vec = reply.as_vec();
            if msg_name == "HsmdInit2" {
                state.cached_hsmd_init2_reply = Some(reply_vec.clone());
            }
            if done {
                let init_taken = state.init_handler.take().ok_or(VlsClientError::Transport)?;
                state.root_handler = Some(init_taken.into());
            }
            return Ok(reply_vec);
        }

        if msg_name == "HsmdInit2" {
            return state
                .cached_hsmd_init2_reply
                .clone()
                .ok_or(VlsClientError::Transport);
        }

        let root = state
            .root_handler
            .as_ref()
            .cloned()
            .ok_or(VlsClientError::Transport)?;
        let reply = root.handle(msg).map_err(|_| VlsClientError::Transport)?;
        Ok(reply.as_vec())
    }

    fn call(
        &self,
        dbid: u64,
        peer_id: vls_protocol::model::PubKey,
        message: Vec<u8>,
    ) -> Result<Vec<u8>, VlsClientError> {
        let msg_name = msgs::message_name_from_vec(&message);
        let msg = msgs::from_vec(message).map_err(|e| VlsClientError::Protocol(e.into()))?;
        let mut state = self.state.lock().map_err(|_| VlsClientError::Transport)?;
        let root = state
            .root_handler
            .as_ref()
            .cloned()
            .ok_or(VlsClientError::Transport)?;

        if matches!(msg_name.as_str(), "NewChannel" | "GetChannelBasepoints") {
            let reply = root.handle(msg).map_err(|_| VlsClientError::Transport)?;
            return Ok(reply.as_vec());
        }

        let key = (dbid, peer_id.0);
        let handler = state
            .channel_handlers
            .entry(key)
            .or_insert_with(|| root.for_new_client(1, peer_id, dbid));
        let reply = handler.handle(msg).map_err(|_| VlsClientError::Transport)?;
        Ok(reply.as_vec())
    }
}

#[derive(uniffi::Object)]
pub struct NativeExternalSigner {
    backend: Arc<dyn ExternalSignerBackend>,
}

impl NativeExternalSigner {
    fn parse_network(network: &str) -> Result<Network, RlnError> {
        match network.to_lowercase().as_str() {
            "mainnet" | "bitcoin" => Ok(Network::Bitcoin),
            "testnet" | "testnet4" => Ok(Network::Testnet),
            "signet" => Ok(Network::Signet),
            "regtest" => Ok(Network::Regtest),
            _ => Err(RlnError::InvalidRequest),
        }
    }

    fn parse_seed_hex(seed_hex: &str) -> Result<[u8; 32], RlnError> {
        let seed_vec = Vec::<u8>::from_hex(seed_hex).map_err(|_| RlnError::InvalidRequest)?;
        let seed: [u8; 32] = seed_vec.try_into().map_err(|_| RlnError::InvalidRequest)?;
        Ok(seed)
    }

    fn map_bootstrap(data: BootstrapData) -> SdkExternalSignerBootstrap {
        SdkExternalSignerBootstrap {
            node_id: data.identity.node_id,
            account_xpub_vanilla: data.identity.account_xpub_vanilla,
            account_xpub_colored: data.identity.account_xpub_colored,
            master_fingerprint: data.identity.master_fingerprint,
            protocol_version: data.protocol_version,
            api_level: data.api_level,
        }
    }
}

#[uniffi::export]
impl NativeExternalSigner {
    #[uniffi::constructor]
    pub fn new(
        seed_hex: String,
        network: String,
        permissive_policy: Option<bool>,
    ) -> Result<Arc<Self>, RlnError> {
        let network = Self::parse_network(&network)?;
        let seed = Self::parse_seed_hex(&seed_hex)?;
        let transport = Arc::new(
            InProcessVlsTransport::new(network, seed, permissive_policy.unwrap_or(true))
                .context("native signer transport init failed")
                .map_err(|_| RlnError::Internal)?,
        );
        let backend: Arc<dyn ExternalSignerBackend> = Arc::new(VlsSignerAdapter::new(
            RealVlsClient::new_with_network_and_seed(transport, network.to_string(), Some(seed)),
        ));
        Ok(Arc::new(Self { backend }))
    }

    pub fn bootstrap(&self) -> Result<SdkExternalSignerBootstrap, RlnError> {
        let bootstrap = match self.backend.call(SignerRequest::Bootstrap).map_err(|e| {
            tracing::error!(error = ?e, "native external signer bootstrap failed");
            RlnError::Internal
        })? {
            signer_contract::SignerResponse::Bootstrap(data) => data,
            other => {
                tracing::error!(response = ?other, "native external signer returned non-bootstrap response");
                return Err(RlnError::Internal);
            }
        };
        Ok(Self::map_bootstrap(bootstrap))
    }
}

impl ExternalSignerHost for NativeExternalSigner {
    fn call(&self, request: Vec<u8>) -> Result<Vec<u8>, RlnError> {
        let signer_request: SignerRequest = decode_signer_request(&request).map_err(|e| {
            tracing::error!(error = ?e, "native external signer protobuf decode failed");
            RlnError::Internal
        })?;
        let signer_response = self.backend.call(signer_request).map_err(|e| {
            tracing::error!(error = ?e, "native external signer backend call failed");
            RlnError::Internal
        })?;
        encode_signer_response(&signer_response).map_err(|e| {
            tracing::error!(error = %e, "native external signer response encode failed");
            RlnError::Internal
        })
    }
}
