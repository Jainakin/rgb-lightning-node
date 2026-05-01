use super::proto::{decode_signer_response, encode_signer_request};
use super::transport::ExternalSignerTransport;
use super::types::{
    BootstrapData, ChannelPublicKeys, ExternalChannelRequest, ExternalNodeRequest,
    ExternalNodeResponse, ExternalSignerRequest, ExternalSignerResponse, RlnSignerError,
    WalletInputMetadata,
};
use std::sync::Arc;

/// RLN-facing backend boundary for external signer operations.
///
/// Runtime code depends on this trait and RLN-local types only. The concrete
/// VLS protocol and crate specifics stay encapsulated in the adapter.
pub(crate) trait ExternalSignerBackend: Send + Sync {
    fn call(
        &self,
        request: ExternalSignerRequest,
    ) -> Result<ExternalSignerResponse, RlnSignerError>;
    fn bootstrap(&self) -> Result<BootstrapData, RlnSignerError>;
    fn node_get_node_id(&self, recipient: String) -> Result<String, RlnSignerError>;
    fn node_get_destination_script(
        &self,
        channel_keys_id_hex: String,
    ) -> Result<String, RlnSignerError>;
    fn node_get_shutdown_scriptpubkey(&self) -> Result<String, RlnSignerError>;
    fn node_get_secure_random_bytes(&self) -> Result<String, RlnSignerError>;
    fn generate_channel_keys_id(
        &self,
        inbound: bool,
        channel_value_satoshis: u64,
        user_channel_id: u128,
    ) -> Result<String, RlnSignerError>;
    fn derive_channel_signer(
        &self,
        channel_value_satoshis: u64,
        channel_keys_id_hex: String,
    ) -> Result<(String, ChannelPublicKeys), RlnSignerError>;
    fn sign_spendable_outputs_psbt(
        &self,
        descriptors: Vec<String>,
        psbt: String,
    ) -> Result<String, RlnSignerError>;
    fn sign_rgb_psbt(
        &self,
        descriptors: Vec<String>,
        psbt: String,
    ) -> Result<String, RlnSignerError>;
    fn get_wallet_input_metadata(
        &self,
        txid_hex: String,
        vout: u32,
        script_pubkey_hex: Option<String>,
        amount_sat: Option<u64>,
    ) -> Result<Option<WalletInputMetadata>, RlnSignerError>;
}

/// Adapter that maps RLN operations to the external signer protocol contract
/// currently aligned with the VLS-backed signer-external service.
#[derive(Clone)]
pub(crate) struct VlsSignerAdapter {
    transport: Arc<dyn ExternalSignerTransport>,
}

impl VlsSignerAdapter {
    pub(crate) fn new(transport: Arc<dyn ExternalSignerTransport>) -> Self {
        Self { transport }
    }
}

impl ExternalSignerBackend for VlsSignerAdapter {
    fn call(
        &self,
        request: ExternalSignerRequest,
    ) -> Result<ExternalSignerResponse, RlnSignerError> {
        let wire_payload = encode_signer_request(&request)?;
        let response_wire_payload = self.transport.call(&wire_payload)?;
        decode_signer_response(&response_wire_payload)
    }

    fn bootstrap(&self) -> Result<BootstrapData, RlnSignerError> {
        match self.call(ExternalSignerRequest::Bootstrap)? {
            ExternalSignerResponse::Bootstrap(data) => Ok(data),
            other => Err(RlnSignerError::Protocol(format!(
                "unexpected response for bootstrap: {other:?}"
            ))),
        }
    }

    fn node_get_node_id(&self, recipient: String) -> Result<String, RlnSignerError> {
        match self.call(ExternalSignerRequest::Node(
            ExternalNodeRequest::GetNodeId { recipient },
        ))? {
            ExternalSignerResponse::Node(ExternalNodeResponse::NodeId { node_id_hex }) => {
                Ok(node_id_hex)
            }
            other => Err(RlnSignerError::Protocol(format!(
                "unexpected response for get_node_id: {other:?}"
            ))),
        }
    }

    fn node_get_destination_script(
        &self,
        channel_keys_id_hex: String,
    ) -> Result<String, RlnSignerError> {
        match self.call(ExternalSignerRequest::Node(
            ExternalNodeRequest::GetDestinationScript {
                channel_keys_id_hex,
            },
        ))? {
            ExternalSignerResponse::Node(ExternalNodeResponse::Script { script_hex }) => {
                Ok(script_hex)
            }
            other => Err(RlnSignerError::Protocol(format!(
                "unexpected response for get_destination_script: {other:?}"
            ))),
        }
    }

    fn node_get_shutdown_scriptpubkey(&self) -> Result<String, RlnSignerError> {
        match self.call(ExternalSignerRequest::Node(
            ExternalNodeRequest::GetShutdownScriptpubkey,
        ))? {
            ExternalSignerResponse::Node(ExternalNodeResponse::Script { script_hex }) => {
                Ok(script_hex)
            }
            other => Err(RlnSignerError::Protocol(format!(
                "unexpected response for get_shutdown_scriptpubkey: {other:?}"
            ))),
        }
    }

    fn node_get_secure_random_bytes(&self) -> Result<String, RlnSignerError> {
        match self.call(ExternalSignerRequest::Node(
            ExternalNodeRequest::GetSecureRandomBytes,
        ))? {
            ExternalSignerResponse::Node(ExternalNodeResponse::RandomBytes { bytes_hex }) => {
                Ok(bytes_hex)
            }
            other => Err(RlnSignerError::Protocol(format!(
                "unexpected response for get_secure_random_bytes: {other:?}"
            ))),
        }
    }

    fn generate_channel_keys_id(
        &self,
        inbound: bool,
        channel_value_satoshis: u64,
        user_channel_id: u128,
    ) -> Result<String, RlnSignerError> {
        match self.call(ExternalSignerRequest::Channel(
            ExternalChannelRequest::GenerateChannelKeysId {
                inbound,
                channel_value_satoshis,
                user_channel_id,
            },
        ))? {
            ExternalSignerResponse::Channel(
                super::types::ExternalChannelResponse::GeneratedChannelKeysId {
                    channel_keys_id_hex,
                },
            ) => Ok(channel_keys_id_hex),
            other => Err(RlnSignerError::Protocol(format!(
                "unexpected response for generate_channel_keys_id: {other:?}"
            ))),
        }
    }

    fn derive_channel_signer(
        &self,
        channel_value_satoshis: u64,
        channel_keys_id_hex: String,
    ) -> Result<(String, ChannelPublicKeys), RlnSignerError> {
        match self.call(ExternalSignerRequest::Channel(
            ExternalChannelRequest::DeriveChannelSigner {
                channel_value_satoshis,
                channel_keys_id_hex,
            },
        ))? {
            ExternalSignerResponse::Channel(
                super::types::ExternalChannelResponse::ChannelSignerData {
                    channel_signer_state_hex,
                    channel_pubkeys,
                },
            ) => Ok((channel_signer_state_hex, channel_pubkeys)),
            other => Err(RlnSignerError::Protocol(format!(
                "unexpected response for derive_channel_signer: {other:?}"
            ))),
        }
    }

    fn sign_rgb_psbt(
        &self,
        descriptors: Vec<String>,
        psbt: String,
    ) -> Result<String, RlnSignerError> {
        match self.call(ExternalSignerRequest::SignRgbPsbt { descriptors, psbt })? {
            ExternalSignerResponse::SignedPsbt { psbt } => Ok(psbt),
            other => Err(RlnSignerError::Protocol(format!(
                "unexpected response for sign_rgb_psbt: {other:?}"
            ))),
        }
    }

    fn sign_spendable_outputs_psbt(
        &self,
        descriptors: Vec<String>,
        psbt: String,
    ) -> Result<String, RlnSignerError> {
        match self.call(ExternalSignerRequest::SignSpendableOutputsPsbt { descriptors, psbt })? {
            ExternalSignerResponse::SignedPsbt { psbt } => Ok(psbt),
            other => Err(RlnSignerError::Protocol(format!(
                "unexpected response for sign_spendable_outputs_psbt: {other:?}"
            ))),
        }
    }

    fn get_wallet_input_metadata(
        &self,
        txid_hex: String,
        vout: u32,
        script_pubkey_hex: Option<String>,
        amount_sat: Option<u64>,
    ) -> Result<Option<WalletInputMetadata>, RlnSignerError> {
        match self.call(ExternalSignerRequest::GetWalletInputMetadata {
            txid_hex,
            vout,
            script_pubkey_hex,
            amount_sat,
        })? {
            ExternalSignerResponse::WalletInputMetadata { metadata } => Ok(metadata),
            other => Err(RlnSignerError::Protocol(format!(
                "unexpected response for get_wallet_input_metadata: {other:?}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signer::proto::{decode_signer_request, encode_signer_response};
    use crate::signer::types::{ExternalChannelResponse, SignerIdentity};
    use std::sync::Mutex;

    #[derive(Default)]
    struct MockTransport {
        last_request: Mutex<Option<Vec<u8>>>,
    }

    impl MockTransport {
        fn recorded_request(&self) -> ExternalSignerRequest {
            let req = self
                .last_request
                .lock()
                .expect("lock")
                .clone()
                .expect("request");
            decode_signer_request(&req).expect("decode request")
        }
    }

    impl ExternalSignerTransport for MockTransport {
        fn call(&self, request: &[u8]) -> Result<Vec<u8>, RlnSignerError> {
            *self.last_request.lock().expect("lock") = Some(request.to_vec());
            let req = decode_signer_request(request).map_err(|e| {
                RlnSignerError::Protocol(format!("decode request envelope in mock failed: {e}"))
            })?;
            let response = match req {
                ExternalSignerRequest::Bootstrap => {
                    ExternalSignerResponse::Bootstrap(BootstrapData {
                        identity: SignerIdentity {
                            node_id: "02".repeat(33),
                            account_xpub_vanilla: "xpub-v".to_string(),
                            account_xpub_colored: "xpub-c".to_string(),
                            master_fingerprint: "f00dbabe".to_string(),
                        },
                        protocol_version: "v1".to_string(),
                        api_level: 1,
                    })
                }
                ExternalSignerRequest::Node(ExternalNodeRequest::GetNodeId { .. }) => {
                    ExternalSignerResponse::Node(ExternalNodeResponse::NodeId {
                        node_id_hex: "03".repeat(33),
                    })
                }
                ExternalSignerRequest::Node(ExternalNodeRequest::GetDestinationScript {
                    ..
                })
                | ExternalSignerRequest::Node(ExternalNodeRequest::GetShutdownScriptpubkey) => {
                    ExternalSignerResponse::Node(ExternalNodeResponse::Script {
                        script_hex: "0014".to_string() + &"11".repeat(20),
                    })
                }
                ExternalSignerRequest::Node(ExternalNodeRequest::GetSecureRandomBytes) => {
                    ExternalSignerResponse::Node(ExternalNodeResponse::RandomBytes {
                        bytes_hex: "ab".repeat(32),
                    })
                }
                ExternalSignerRequest::Channel(ExternalChannelRequest::GenerateChannelKeysId {
                    ..
                }) => ExternalSignerResponse::Channel(
                    ExternalChannelResponse::GeneratedChannelKeysId {
                        channel_keys_id_hex: "cd".repeat(32),
                    },
                ),
                ExternalSignerRequest::Channel(ExternalChannelRequest::DeriveChannelSigner {
                    ..
                })
                | ExternalSignerRequest::Channel(ExternalChannelRequest::ReadChannelSigner {
                    ..
                }) => ExternalSignerResponse::Channel(ExternalChannelResponse::ChannelSignerData {
                    channel_signer_state_hex: "ef".repeat(64),
                    channel_pubkeys: ChannelPublicKeys {
                        funding_pubkey_hex: "02".repeat(33),
                        revocation_basepoint_hex: "03".repeat(33),
                        payment_point_hex: "04".repeat(33),
                        delayed_payment_basepoint_hex: "05".repeat(33),
                        htlc_basepoint_hex: "06".repeat(33),
                    },
                }),
                ExternalSignerRequest::SignRgbPsbt { psbt, .. } => {
                    ExternalSignerResponse::SignedPsbt { psbt }
                }
                _ => ExternalSignerResponse::Channel(ExternalChannelResponse::Signature {
                    signature_hex: "aa".repeat(64),
                }),
            };
            encode_signer_response(&response)
        }
    }

    #[test]
    fn adapter_node_methods_use_expected_envelope() {
        let transport = Arc::new(MockTransport::default());
        let adapter = VlsSignerAdapter::new(transport.clone());

        let node_id = adapter
            .node_get_node_id("node".to_string())
            .expect("node id");
        assert_eq!(node_id.len(), 66);
        assert!(matches!(
            transport.recorded_request(),
            ExternalSignerRequest::Node(ExternalNodeRequest::GetNodeId { .. })
        ));

        let _ = adapter
            .node_get_destination_script("11".repeat(32))
            .expect("destination script");
        assert!(matches!(
            transport.recorded_request(),
            ExternalSignerRequest::Node(ExternalNodeRequest::GetDestinationScript { .. })
        ));

        let _ = adapter
            .node_get_shutdown_scriptpubkey()
            .expect("shutdown script");
        assert!(matches!(
            transport.recorded_request(),
            ExternalSignerRequest::Node(ExternalNodeRequest::GetShutdownScriptpubkey)
        ));

        let random = adapter
            .node_get_secure_random_bytes()
            .expect("random bytes");
        assert_eq!(random.len(), 64);
        assert!(matches!(
            transport.recorded_request(),
            ExternalSignerRequest::Node(ExternalNodeRequest::GetSecureRandomBytes)
        ));
    }

    #[test]
    fn adapter_channel_and_rgb_methods_use_expected_envelope() {
        let transport = Arc::new(MockTransport::default());
        let adapter = VlsSignerAdapter::new(transport.clone());

        let _ = adapter
            .generate_channel_keys_id(false, 1000, 42)
            .expect("keys id");
        assert!(matches!(
            transport.recorded_request(),
            ExternalSignerRequest::Channel(ExternalChannelRequest::GenerateChannelKeysId { .. })
        ));

        let _ = adapter
            .derive_channel_signer(1000, "22".repeat(32))
            .expect("derive signer");
        assert!(matches!(
            transport.recorded_request(),
            ExternalSignerRequest::Channel(ExternalChannelRequest::DeriveChannelSigner { .. })
        ));

        let psbt = adapter
            .sign_rgb_psbt(vec!["d".to_string()], "psbt_hex".to_string())
            .expect("sign rgb");
        assert_eq!(psbt, "psbt_hex");
        assert!(matches!(
            transport.recorded_request(),
            ExternalSignerRequest::SignRgbPsbt { .. }
        ));
    }
}
