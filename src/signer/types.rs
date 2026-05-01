pub(crate) type WalletInputMetadata = signer_contract::WalletInputMetadata;
#[allow(dead_code)]
pub(crate) type SignerIdentity = signer_contract::SignerIdentity;
pub(crate) type BootstrapData = signer_contract::BootstrapData;
pub(crate) type ExternalSignerRequest = signer_contract::SignerRequest;
pub(crate) type ExternalSignerResponse = signer_contract::SignerResponse;
pub(crate) type ExternalNodeRequest = signer_contract::NodeRequest;
pub(crate) type ExternalNodeResponse = signer_contract::NodeResponse;
pub(crate) type ExternalChannelHtlc = signer_contract::ChannelHtlc;
pub(crate) type ExternalChannelOp = signer_contract::ChannelOp;
pub(crate) type ExternalChannelRequest = signer_contract::ChannelRequest;
pub(crate) type ExternalChannelResponse = signer_contract::ChannelResponse;
pub(crate) type ChannelPublicKeys = signer_contract::ChannelPublicKeys;

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

pub(crate) const SUPPORTED_SIGNER_API_LEVEL: u32 = 1;
