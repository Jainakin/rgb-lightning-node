pub(crate) type WalletInputMetadata = signer_external::contract::WalletInputMetadata;
pub(crate) type SpendableOutputUtxo = signer_external::contract::SpendableOutputUtxo;
pub(crate) type DebugDerivedAddress = signer_external::contract::DebugDerivedAddress;
#[allow(dead_code)]
pub(crate) type SignerIdentity = signer_external::contract::SignerIdentity;
pub(crate) type BootstrapData = signer_external::contract::BootstrapData;
pub(crate) type ExternalSignerRequest = signer_external::contract::SignerRequest;
pub(crate) type ExternalSignerResponse = signer_external::contract::SignerResponse;
pub(crate) type ExternalNodeRequest = signer_external::contract::NodeRequest;
pub(crate) type ExternalNodeResponse = signer_external::contract::NodeResponse;
pub(crate) type ExternalChannelHtlc = signer_external::contract::ChannelHtlc;
pub(crate) type ExternalChannelOp = signer_external::contract::ChannelOp;
pub(crate) type ExternalChannelRequest = signer_external::contract::ChannelRequest;
pub(crate) type ExternalChannelResponse = signer_external::contract::ChannelResponse;
pub(crate) type ChannelPublicKeys = signer_external::contract::ChannelPublicKeys;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RgbWalletAccountInfo {
    pub(crate) account_xpub_vanilla: String,
    pub(crate) account_xpub_colored: String,
    pub(crate) master_fingerprint: String,
    pub(crate) vanilla_keychain: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum RlnSignerError {
    #[allow(dead_code)]
    #[error("external signer transport error: {0}")]
    Transport(String),
    #[error("external signer protocol error: {0}")]
    Protocol(String),
    #[error("unsupported operation in external signer mode: {0}")]
    Unsupported(String),
}

/// Bootstrap `api_level` the node accepts from an attached external signer. **Must stay in sync**
/// with hosts and `signer_external::contract::BootstrapData::api_level` (currently **`1`** only).
pub(crate) const SUPPORTED_SIGNER_API_LEVEL: u32 = 1;

/// Lowercase hex encoding for test fixtures and mock signers (not used in release-only paths).
#[cfg(test)]
pub(crate) fn hex_encode_lower(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(&mut s, "{b:02x}");
    }
    s
}

/// Validates LDK auxiliary key hex fields from bootstrap (64 hex chars = 32 bytes each).
pub(crate) fn validate_bootstrap_ldk_auxiliary_keys(
    bootstrap: &BootstrapData,
) -> Result<(), RlnSignerError> {
    use bitcoin::hex::FromHex;
    for (label, h) in [
        (
            "ldk_inbound_payment_key_hex",
            bootstrap.ldk_inbound_payment_key_hex.as_str(),
        ),
        (
            "ldk_peer_storage_key_hex",
            bootstrap.ldk_peer_storage_key_hex.as_str(),
        ),
        (
            "ldk_receive_auth_key_hex",
            bootstrap.ldk_receive_auth_key_hex.as_str(),
        ),
    ] {
        if h.len() != 64 {
            return Err(RlnSignerError::Protocol(format!(
                "{label} must be 64 hex characters (32 bytes), got length {}",
                h.len()
            )));
        }
        let _ = Vec::<u8>::from_hex(h)
            .map_err(|e| RlnSignerError::Protocol(format!("{label} is not valid hex: {e}")))?;
    }
    let ap = bootstrap.async_payments_root_seed_hex.as_str();
    if !ap.is_empty() {
        if ap.len() != 64 {
            return Err(RlnSignerError::Protocol(format!(
                "async_payments_root_seed_hex must be empty or 64 hex characters (32 bytes), got length {}",
                ap.len()
            )));
        }
        let _ = Vec::<u8>::from_hex(ap).map_err(|e| {
            RlnSignerError::Protocol(format!(
                "async_payments_root_seed_hex is not valid hex: {e}"
            ))
        })?;
    }
    Ok(())
}

/// Legacy deterministic 32-byte seed when [`BootstrapData::async_payments_root_seed_hex`] is empty.
pub(crate) fn derive_async_payments_compat_seed_from_bootstrap(
    bootstrap: &BootstrapData,
) -> [u8; 32] {
    use bitcoin::hashes::{sha256, Hash as BitcoinHash};
    let mut seed_material = Vec::new();
    seed_material.extend_from_slice(bootstrap.identity.node_id.as_bytes());
    seed_material.extend_from_slice(bootstrap.identity.account_xpub_vanilla.as_bytes());
    seed_material.extend_from_slice(bootstrap.identity.account_xpub_colored.as_bytes());
    seed_material.extend_from_slice(bootstrap.identity.master_fingerprint.as_bytes());
    seed_material.extend_from_slice(bootstrap.protocol_version.as_bytes());
    <sha256::Hash as BitcoinHash>::hash(&seed_material).to_byte_array()
}

/// 32-byte seed for [`crate::async_order::AsyncPaymentsPreimageRoot::build_from_seed`]: host value
/// when [`BootstrapData::async_payments_root_seed_hex`] is set, otherwise [`derive_async_payments_compat_seed_from_bootstrap`].
pub(crate) fn async_payments_root_seed_bytes(
    bootstrap: &BootstrapData,
) -> Result<[u8; 32], RlnSignerError> {
    use bitcoin::hex::FromHex;
    let h = bootstrap.async_payments_root_seed_hex.as_str();
    if h.is_empty() {
        return Ok(derive_async_payments_compat_seed_from_bootstrap(bootstrap));
    }
    let v = Vec::<u8>::from_hex(h).map_err(|e| {
        RlnSignerError::Protocol(format!("async_payments_root_seed_hex invalid hex: {e}"))
    })?;
    v.try_into().map_err(|_| {
        RlnSignerError::Protocol(
            "async_payments_root_seed_hex must decode to exactly 32 bytes".to_string(),
        )
    })
}

#[cfg(test)]
mod async_payments_seed_tests {
    use super::*;

    fn fake_bootstrap(async_hex: String) -> BootstrapData {
        BootstrapData {
            identity: SignerIdentity {
                node_id: "02".repeat(33),
                account_xpub_vanilla: "xv".to_string(),
                account_xpub_colored: "xc".to_string(),
                master_fingerprint: "deadbeef".to_string(),
            },
            protocol_version: "1".to_string(),
            api_level: 1,
            ldk_inbound_payment_key_hex: "ab".repeat(32),
            ldk_peer_storage_key_hex: "cd".repeat(32),
            ldk_receive_auth_key_hex: "ef".repeat(32),
            async_payments_root_seed_hex: async_hex,
        }
    }

    #[test]
    fn empty_async_seed_uses_compat_derivation() {
        let b = fake_bootstrap(String::new());
        validate_bootstrap_ldk_auxiliary_keys(&b).expect("valid");
        let seed = async_payments_root_seed_bytes(&b).expect("seed");
        assert_eq!(seed, derive_async_payments_compat_seed_from_bootstrap(&b));
    }

    #[test]
    fn nonempty_async_seed_is_host_bytes() {
        let b = fake_bootstrap("07".repeat(32));
        validate_bootstrap_ldk_auxiliary_keys(&b).expect("valid");
        let seed = async_payments_root_seed_bytes(&b).expect("seed");
        assert_eq!(seed, [7u8; 32]);
    }
}
