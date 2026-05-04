use super::proto::{decode_signer_request, encode_signer_response};
use super::transport::ExternalSignerTransport;
use super::types::{ExternalSignerRequest, RlnSignerError};
use super::vls_adapter::ExternalSignerBackend;
use std::sync::Arc;

/// In-memory transport for attached external signer sessions.
///
/// This preserves the canonical sync byte-call boundary while avoiding any
/// network or HTTP dependency. Requests are decoded from the RLN signer
/// envelope, dispatched to an attached backend, then re-encoded into the same
/// envelope for the caller.
#[derive(Clone)]
pub(crate) struct InProcessExternalSignerTransport {
    backend: Arc<dyn ExternalSignerBackend>,
}

impl InProcessExternalSignerTransport {
    pub(crate) fn new(backend: Arc<dyn ExternalSignerBackend>) -> Self {
        Self { backend }
    }
}

impl ExternalSignerTransport for InProcessExternalSignerTransport {
    fn call(&self, request: &[u8]) -> Result<Vec<u8>, RlnSignerError> {
        let signer_request: ExternalSignerRequest = decode_signer_request(request)?;
        let signer_response = self.backend.call(signer_request)?;
        encode_signer_response(&signer_response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signer::proto::{decode_signer_response, encode_signer_request};
    use crate::signer::types::{
        BootstrapData, ExternalChannelRequest, ExternalNodeRequest, ExternalNodeResponse,
        ExternalSignerResponse, SignerIdentity, WalletInputMetadata,
    };
    use crate::signer::vls_adapter::ExternalSignerBackend;

    #[derive(Default)]
    struct TestBackend;

    impl ExternalSignerBackend for TestBackend {
        fn call(
            &self,
            request: ExternalSignerRequest,
        ) -> Result<ExternalSignerResponse, RlnSignerError> {
            match request {
                ExternalSignerRequest::Bootstrap => {
                    let seed = [8u8; 32];
                    let (inb, peer, recv) =
                        signer_external::ldk_keys_manager_material::derive_ldk_keys_manager_auxiliary_secret_bytes(
                            &seed,
                        )
                        .expect("derive");
                    Ok(ExternalSignerResponse::Bootstrap(BootstrapData {
                        identity: SignerIdentity {
                            node_id: "node".to_string(),
                            account_xpub_vanilla: "xpub-v".to_string(),
                            account_xpub_colored: "xpub-c".to_string(),
                            master_fingerprint: "deadbeef".to_string(),
                        },
                        protocol_version: "1".to_string(),
                        api_level: 1,
                        ldk_inbound_payment_key_hex: crate::signer::types::hex_encode_lower(&inb),
                        ldk_peer_storage_key_hex: crate::signer::types::hex_encode_lower(&peer),
                        ldk_receive_auth_key_hex: crate::signer::types::hex_encode_lower(&recv),
                        async_payments_root_seed_hex: crate::signer::types::hex_encode_lower(&seed),
                    }))
                }
                ExternalSignerRequest::Node(ExternalNodeRequest::GetNodeId { .. }) => {
                    Ok(ExternalSignerResponse::Node(ExternalNodeResponse::NodeId {
                        node_id_hex: "02".repeat(33),
                    }))
                }
                ExternalSignerRequest::Channel(ExternalChannelRequest::GenerateChannelKeysId {
                    ..
                }) => Err(RlnSignerError::Transport("boom".to_string())),
                _ => Err(RlnSignerError::Unsupported("unused".to_string())),
            }
        }

        fn bootstrap(&self) -> Result<BootstrapData, RlnSignerError> {
            match self.call(ExternalSignerRequest::Bootstrap)? {
                ExternalSignerResponse::Bootstrap(data) => Ok(data),
                other => Err(RlnSignerError::Protocol(format!(
                    "unexpected bootstrap response: {other:?}"
                ))),
            }
        }

        fn node_get_node_id(&self, _recipient: String) -> Result<String, RlnSignerError> {
            Ok("02".repeat(33))
        }

        fn node_get_destination_script(
            &self,
            _channel_keys_id_hex: String,
        ) -> Result<String, RlnSignerError> {
            Err(RlnSignerError::Unsupported("unused".to_string()))
        }

        fn node_get_shutdown_scriptpubkey(&self) -> Result<String, RlnSignerError> {
            Err(RlnSignerError::Unsupported("unused".to_string()))
        }

        fn generate_channel_keys_id(
            &self,
            _inbound: bool,
            _channel_value_satoshis: u64,
            _user_channel_id: u128,
        ) -> Result<String, RlnSignerError> {
            Err(RlnSignerError::Transport("boom".to_string()))
        }

        fn derive_channel_signer(
            &self,
            _channel_value_satoshis: u64,
            _channel_keys_id_hex: String,
        ) -> Result<(String, super::super::types::ChannelPublicKeys), RlnSignerError> {
            Err(RlnSignerError::Unsupported("unused".to_string()))
        }

        fn sign_spendable_outputs_psbt(
            &self,
            _utxos: Vec<super::super::types::SpendableOutputUtxo>,
            _psbt: String,
        ) -> Result<String, RlnSignerError> {
            Err(RlnSignerError::Unsupported("unused".to_string()))
        }

        fn sign_rgb_psbt(
            &self,
            _descriptors: Vec<String>,
            _psbt: String,
        ) -> Result<String, RlnSignerError> {
            Err(RlnSignerError::Unsupported("unused".to_string()))
        }

        fn get_wallet_input_metadata(
            &self,
            _txid_hex: String,
            _vout: u32,
            _script_pubkey_hex: Option<String>,
            _amount_sat: Option<u64>,
        ) -> Result<Option<WalletInputMetadata>, RlnSignerError> {
            Ok(None)
        }
    }

    #[test]
    fn in_process_transport_roundtrips_bootstrap() {
        let transport = InProcessExternalSignerTransport::new(Arc::new(TestBackend));
        let request_wire =
            encode_signer_request(&ExternalSignerRequest::Bootstrap).expect("encode wire");
        let response_wire = transport.call(&request_wire).expect("transport call");
        let response: ExternalSignerResponse =
            decode_signer_response(&response_wire).expect("decode response");
        match response {
            ExternalSignerResponse::Bootstrap(data) => {
                assert_eq!(data.identity.node_id, "node");
                assert_eq!(data.api_level, 1);
                assert_eq!(data.ldk_inbound_payment_key_hex.len(), 64);
                assert_eq!(data.ldk_peer_storage_key_hex.len(), 64);
                assert_eq!(data.ldk_receive_auth_key_hex.len(), 64);
            }
            other => panic!("unexpected response: {other:?}"),
        }
    }

    #[test]
    fn in_process_transport_propagates_backend_errors() {
        let transport = InProcessExternalSignerTransport::new(Arc::new(TestBackend));
        let request_wire = encode_signer_request(&ExternalSignerRequest::Channel(
            ExternalChannelRequest::GenerateChannelKeysId {
                inbound: false,
                channel_value_satoshis: 1,
                user_channel_id: 2,
            },
        ))
        .expect("encode wire");
        let err = transport.call(&request_wire).expect_err("must fail");
        match err {
            RlnSignerError::Transport(msg) => assert_eq!(msg, "boom"),
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
