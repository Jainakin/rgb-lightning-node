pub(crate) type WalletInputMetadata = signer_external::contract::WalletInputMetadata;
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

pub(crate) const SUPPORTED_SIGNER_API_LEVEL: u32 = 1;
