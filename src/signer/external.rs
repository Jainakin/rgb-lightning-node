use std::sync::Arc;

use bitcoin::hex::DisplayHex;
use bitcoin::hex::FromHex;
use bitcoin::psbt::ExtractTxError;
use bitcoin::script::ScriptBuf;
use bitcoin::secp256k1::ecdh::SharedSecret;
use bitcoin::secp256k1::ecdsa::{RecoverableSignature, RecoveryId, Signature};
use bitcoin::secp256k1::{PublicKey, Scalar};
use bitcoin::Psbt;
use lightning::ln::msgs::UnsignedGossipMessage;
use lightning::ln::script::ShutdownScript;
use lightning::offers::invoice::UnsignedBolt12Invoice;
use lightning::sign::{
    EntropySource, NodeSigner, OutputSpender, PeerStorageKey, Recipient, SignerProvider,
    SpendableOutputDescriptor,
};
use lightning::util::ser::Writeable;
use lightning_invoice::RawBolt11Invoice;
use serde_json::json;
use std::str::FromStr;

use super::channel_signer::ExternalChannelSigner;
use super::transport::ExternalSignerTransport;
use super::types::{
    BootstrapData, ExternalNodeRequest, ExternalNodeResponse, ExternalSignerRequest,
    ExternalSignerResponse, RgbWalletAccountInfo, RlnSignerError, WalletInputMetadata,
};
use super::vls_adapter::{ExternalSignerBackend, VlsSignerAdapter};
use super::RlnKeysInterface;

/// Transport-backed entrypoint for external signer protocol calls.
///
/// LDK trait implementations are added in the next integration step; this
/// type currently provides a typed request/response boundary over raw bytes.
#[derive(Clone)]
pub(crate) struct ExternalSigner {
    backend: Arc<dyn ExternalSignerBackend>,
}

#[derive(Clone)]
pub(crate) struct ExternalSignerAttachment {
    pub(crate) bootstrap: BootstrapData,
    pub(crate) transport: Arc<dyn ExternalSignerTransport>,
}

impl ExternalSigner {
    fn parse_signature(signature_hex: &str) -> Result<Signature, ()> {
        let bytes = Vec::<u8>::from_hex(signature_hex).map_err(|_| ())?;
        Signature::from_der(&bytes)
            .or_else(|_| Signature::from_compact(&bytes))
            .map_err(|_| ())
    }

    pub(crate) fn with_vls_adapter(transport: Arc<dyn ExternalSignerTransport>) -> Self {
        let backend: Arc<dyn ExternalSignerBackend> =
            Arc::new(VlsSignerAdapter::new(Arc::clone(&transport)));
        Self { backend }
    }

    pub(crate) fn from_attachment(attachment: &ExternalSignerAttachment) -> Self {
        Self::with_vls_adapter(Arc::clone(&attachment.transport))
    }

    pub(crate) fn bootstrap(&self) -> Result<BootstrapData, RlnSignerError> {
        self.backend.bootstrap()
    }

    pub(crate) fn generate_channel_keys_id(
        &self,
        inbound: bool,
        channel_value_satoshis: u64,
        user_channel_id: u128,
    ) -> Result<String, RlnSignerError> {
        self.backend
            .generate_channel_keys_id(inbound, channel_value_satoshis, user_channel_id)
    }

    pub(crate) fn derive_channel_signer(
        &self,
        channel_value_satoshis: u64,
        channel_keys_id_hex: String,
    ) -> Result<ExternalChannelSigner, RlnSignerError> {
        let (channel_signer_state_hex, channel_pubkeys) = self
            .backend
            .derive_channel_signer(channel_value_satoshis, channel_keys_id_hex.clone())?;
        Ok(ExternalChannelSigner::new(
            Arc::clone(&self.backend),
            channel_keys_id_hex,
            channel_signer_state_hex,
            channel_pubkeys,
        ))
    }

    pub(crate) fn sign_rgb_psbt(
        &self,
        descriptors: Vec<String>,
        psbt: String,
    ) -> Result<String, RlnSignerError> {
        self.backend.sign_rgb_psbt(descriptors, psbt)
    }

    pub(crate) fn get_wallet_input_metadata(
        &self,
        txid_hex: String,
        vout: u32,
        script_pubkey_hex: Option<String>,
        amount_sat: Option<u64>,
    ) -> Result<Option<WalletInputMetadata>, RlnSignerError> {
        self.backend
            .get_wallet_input_metadata(txid_hex, vout, script_pubkey_hex, amount_sat)
    }

    fn seed_material(&self) -> [u8; 32] {
        // Deterministic local seed material for LDK helper keys. This is not a signing secret;
        // channel/node signatures are delegated to external signer calls.
        let bootstrap = self.bootstrap().ok();
        let mut out = [0u8; 32];
        if let Some(data) = bootstrap {
            let mut acc = [0u8; 32];
            for (i, b) in data.identity.node_id.as_bytes().iter().enumerate() {
                acc[i % 32] ^= *b;
            }
            out = acc;
        }
        out
    }

    fn recipient_label(recipient: Recipient) -> &'static str {
        match recipient {
            Recipient::Node => "node",
            Recipient::PhantomNode => "phantom",
        }
    }
}

impl EntropySource for ExternalSigner {
    fn get_secure_random_bytes(&self) -> [u8; 32] {
        let bytes_hex = self
            .backend
            .node_get_secure_random_bytes()
            .unwrap_or_else(|_| "00".repeat(32));
        let bytes = Vec::<u8>::from_hex(&bytes_hex).unwrap_or_else(|_| vec![0u8; 32]);
        let mut out = [0u8; 32];
        if bytes.len() >= 32 {
            out.copy_from_slice(&bytes[..32]);
        }
        out
    }
}

impl NodeSigner for ExternalSigner {
    fn get_expanded_key(&self) -> lightning::ln::inbound_payment::ExpandedKey {
        lightning::ln::inbound_payment::ExpandedKey::new(self.seed_material())
    }

    fn get_peer_storage_key(&self) -> PeerStorageKey {
        PeerStorageKey {
            inner: self.seed_material(),
        }
    }

    fn get_receive_auth_key(&self) -> lightning::sign::ReceiveAuthKey {
        lightning::sign::ReceiveAuthKey(self.seed_material())
    }

    fn get_node_id(&self, recipient: Recipient) -> Result<PublicKey, ()> {
        let node_id_hex = self
            .backend
            .node_get_node_id(Self::recipient_label(recipient).to_string())
            .map_err(|_| ())?;
        let bytes = Vec::<u8>::from_hex(&node_id_hex).map_err(|_| ())?;
        PublicKey::from_slice(&bytes).map_err(|_| ())
    }

    fn ecdh(
        &self,
        recipient: Recipient,
        other_key: &PublicKey,
        tweak: Option<&Scalar>,
    ) -> Result<SharedSecret, ()> {
        let req = ExternalSignerRequest::Node(ExternalNodeRequest::Ecdh {
            recipient: Self::recipient_label(recipient).to_string(),
            other_key: other_key.serialize().to_lower_hex_string(),
            tweak: tweak.map(|t| t.to_be_bytes().to_lower_hex_string()),
        });
        let resp = self.backend.call(req).map_err(|_| ())?;
        let ExternalSignerResponse::Node(ExternalNodeResponse::Ecdh { shared_secret_hex }) = resp
        else {
            return Err(());
        };
        let bytes = Vec::<u8>::from_hex(&shared_secret_hex).map_err(|_| ())?;
        let arr: [u8; 32] = bytes.try_into().map_err(|_| ())?;
        Ok(SharedSecret::from_slice(&arr).map_err(|_| ())?)
    }

    fn sign_invoice(
        &self,
        invoice: &RawBolt11Invoice,
        _recipient: Recipient,
    ) -> Result<RecoverableSignature, ()> {
        let hrp = invoice.hrp.to_string();
        let req = ExternalSignerRequest::Node(ExternalNodeRequest::SignInvoice {
            hrp,
            u5bytes_hex: invoice.signable_hash().to_lower_hex_string(),
        });
        let resp = self.backend.call(req).map_err(|_| ())?;
        let ExternalSignerResponse::Node(ExternalNodeResponse::RecoverableSignature {
            signature_hex,
            recovery_id,
        }) = resp
        else {
            return Err(());
        };
        let sig_bytes = Vec::<u8>::from_hex(&signature_hex).map_err(|_| ())?;
        let sig_arr: [u8; 64] = sig_bytes.try_into().map_err(|_| ())?;
        let rec_id = RecoveryId::from_i32(recovery_id as i32).map_err(|_| ())?;
        RecoverableSignature::from_compact(&sig_arr, rec_id).map_err(|_| ())
    }

    fn sign_bolt12_invoice(
        &self,
        invoice: &UnsignedBolt12Invoice,
    ) -> Result<bitcoin::secp256k1::schnorr::Signature, ()> {
        let req = ExternalSignerRequest::Node(ExternalNodeRequest::SignBolt12Invoice {
            invoice: invoice.encode().to_lower_hex_string(),
        });
        let resp = self.backend.call(req).map_err(|_| ())?;
        let ExternalSignerResponse::Node(ExternalNodeResponse::Signature { signature_hex }) = resp
        else {
            return Err(());
        };
        let bytes = Vec::<u8>::from_hex(&signature_hex).map_err(|_| ())?;
        bitcoin::secp256k1::schnorr::Signature::from_slice(&bytes).map_err(|_| ())
    }

    fn sign_gossip_message(&self, msg: UnsignedGossipMessage) -> Result<Signature, ()> {
        let req = ExternalSignerRequest::Node(ExternalNodeRequest::SignGossipMessage {
            message_hex: msg.encode().to_lower_hex_string(),
        });
        let resp = self.backend.call(req).map_err(|_| ())?;
        let ExternalSignerResponse::Node(ExternalNodeResponse::Signature { signature_hex }) = resp
        else {
            return Err(());
        };
        Self::parse_signature(&signature_hex)
    }

    fn sign_message(&self, msg: &[u8]) -> Result<String, ()> {
        let req = ExternalSignerRequest::Node(ExternalNodeRequest::SignMessage {
            message: String::from_utf8(msg.to_vec()).map_err(|_| ())?,
        });
        let resp = self.backend.call(req).map_err(|_| ())?;
        let ExternalSignerResponse::Node(ExternalNodeResponse::Signature { signature_hex }) = resp
        else {
            return Err(());
        };
        Ok(signature_hex)
    }
}

impl SignerProvider for ExternalSigner {
    type EcdsaSigner = ExternalChannelSigner;

    fn generate_channel_keys_id(&self, inbound: bool, user_channel_id: u128) -> [u8; 32] {
        let keys_hex = self
            .generate_channel_keys_id(inbound, 0, user_channel_id)
            .unwrap_or_else(|_| "00".repeat(32));
        let bytes = Vec::<u8>::from_hex(&keys_hex).unwrap_or_else(|_| vec![0u8; 32]);
        let mut out = [0u8; 32];
        if bytes.len() >= 32 {
            out.copy_from_slice(&bytes[..32]);
        }
        out
    }

    fn derive_channel_signer(&self, channel_keys_id: [u8; 32]) -> Self::EcdsaSigner {
        self.derive_channel_signer(0, channel_keys_id.to_lower_hex_string())
            .unwrap_or_else(|_| {
                ExternalChannelSigner::new(
                    Arc::clone(&self.backend),
                    channel_keys_id.to_lower_hex_string(),
                    String::new(),
                    super::types::ChannelPublicKeys {
                        funding_pubkey_hex: "02".repeat(33),
                        revocation_basepoint_hex: "02".repeat(33),
                        payment_point_hex: "02".repeat(33),
                        delayed_payment_basepoint_hex: "02".repeat(33),
                        htlc_basepoint_hex: "02".repeat(33),
                    },
                )
            })
    }

    fn get_destination_script(&self, channel_keys_id: [u8; 32]) -> Result<ScriptBuf, ()> {
        let script_hex = self
            .backend
            .node_get_destination_script(channel_keys_id.to_lower_hex_string())
            .map_err(|_| ())?;
        let bytes = Vec::<u8>::from_hex(&script_hex).map_err(|_| ())?;
        Ok(ScriptBuf::from_bytes(bytes))
    }

    fn get_shutdown_scriptpubkey(&self) -> Result<ShutdownScript, ()> {
        let script_hex = self
            .backend
            .node_get_shutdown_scriptpubkey()
            .map_err(|_| ())?;
        let bytes = Vec::<u8>::from_hex(&script_hex).map_err(|_| ())?;
        let script = ScriptBuf::from_bytes(bytes);
        ShutdownScript::try_from(script).map_err(|_| ())
    }
}

impl OutputSpender for ExternalSigner {
    fn spend_spendable_outputs(
        &self,
        descriptors: &[&SpendableOutputDescriptor],
        outputs: Vec<bitcoin::TxOut>,
        change_destination_script: ScriptBuf,
        feerate_sat_per_1000_weight: u32,
        locktime: Option<bitcoin::locktime::absolute::LockTime>,
        secp_ctx: &bitcoin::secp256k1::Secp256k1<bitcoin::secp256k1::All>,
    ) -> Result<bitcoin::Transaction, ()> {
        let (psbt, _expected_weight) = SpendableOutputDescriptor::create_spendable_outputs_psbt(
            secp_ctx,
            descriptors,
            outputs,
            change_destination_script,
            feerate_sat_per_1000_weight,
            locktime,
        )
        .map_err(|_| ())?;
        let signed = self.sign_spendable_outputs_psbt(descriptors, psbt, secp_ctx)?;
        match signed.extract_tx() {
            Ok(tx) => Ok(tx),
            Err(ExtractTxError::MissingInputValue { tx }) => Ok(tx),
            Err(_) => Err(()),
        }
    }
}

impl RlnKeysInterface for ExternalSigner {
    fn sign_spendable_outputs_psbt(
        &self,
        descriptors: &[&SpendableOutputDescriptor],
        psbt: Psbt,
        _secp_ctx: &bitcoin::secp256k1::Secp256k1<bitcoin::secp256k1::All>,
    ) -> Result<Psbt, ()> {
        let descriptor_payload = descriptors
            .iter()
            .map(|d| {
                match *d {
                    SpendableOutputDescriptor::StaticOutput {
                        ref outpoint,
                        ref output,
                        ..
                    } => {
                        json!({
                            "txid": outpoint.txid.to_string(),
                            "outnum": outpoint.index,
                            "amount": output.value.to_sat(),
                            "keyindex": 1u32,
                            "is_p2sh": false,
                            "script_hex": "",
                            "is_in_coinbase": false,
                        })
                    }
                    SpendableOutputDescriptor::DelayedPaymentOutput(ref o) => {
                        json!({
                            "txid": o.outpoint.txid.to_string(),
                            "outnum": o.outpoint.index,
                            "amount": o.output.value.to_sat(),
                            "keyindex": 0u32,
                            "is_p2sh": false,
                            "script_hex": "",
                            "is_in_coinbase": false,
                        })
                    }
                    SpendableOutputDescriptor::StaticPaymentOutput(ref o) => {
                        json!({
                            "txid": o.outpoint.txid.to_string(),
                            "outnum": o.outpoint.index,
                            "amount": o.output.value.to_sat(),
                            "keyindex": 0u32,
                            "is_p2sh": false,
                            "script_hex": "",
                            "is_in_coinbase": false,
                        })
                    }
                }
                .to_string()
            })
            .collect::<Vec<_>>();
        let signed_psbt = self
            .backend
            .sign_spendable_outputs_psbt(descriptor_payload, psbt.to_string())
            .map_err(|_| ())?;
        Psbt::from_str(&signed_psbt).map_err(|_| ())
    }

    fn sign_rgb_psbt(
        &self,
        descriptors: Vec<String>,
        psbt: String,
    ) -> Result<String, RlnSignerError> {
        self.backend.sign_rgb_psbt(descriptors, psbt)
    }

    fn rgb_wallet_account(&self) -> RgbWalletAccountInfo {
        match self.bootstrap() {
            Ok(bootstrap) => RgbWalletAccountInfo {
                account_xpub_vanilla: bootstrap.identity.account_xpub_vanilla,
                account_xpub_colored: bootstrap.identity.account_xpub_colored,
                master_fingerprint: bootstrap.identity.master_fingerprint,
                vanilla_keychain: None,
            },
            Err(_) => RgbWalletAccountInfo {
                account_xpub_vanilla: String::new(),
                account_xpub_colored: String::new(),
                master_fingerprint: String::new(),
                vanilla_keychain: None,
            },
        }
    }
}
