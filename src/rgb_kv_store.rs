/// RGB-specific namespace and serialization helpers layered on top of the
/// generic `KVStoreSync` backend.
///
/// This is intentionally separate from `kv_store.rs`: `kv_store.rs` provides
/// the storage engine adapter, while this module centralizes RGB key layout,
/// namespace constants, and serde of RGB payloads.
use bitcoin::hex::DisplayHex;
use bitcoin::io;
use lightning::rgb_utils::{RgbInfo, RgbPaymentInfo, TransferInfo};
use lightning::types::payment::PaymentHash;
use lightning::util::persist::KVStoreSync;
use rgb_lib::ContractId;

pub(crate) const RGB_PRIMARY_NS: &str = "rgb";
pub(crate) const RGB_CHANNEL_INFO_NS: &str = "channel_info";
pub(crate) const RGB_CHANNEL_INFO_PENDING_NS: &str = "channel_info_pending";
pub(crate) const RGB_PAYMENT_INFO_INBOUND_NS: &str = "payment_info_inbound";
pub(crate) const RGB_PAYMENT_INFO_OUTBOUND_NS: &str = "payment_info_outbound";
pub(crate) const RGB_TRANSFER_INFO_NS: &str = "transfer_info";
pub(crate) const RGB_CONSIGNMENT_NS: &str = "consignment";
pub(crate) const RGB_WALLET_CONFIG_NS: &str = "wallet_config";

pub(crate) trait RgbKvStoreExt {
    fn write_config(&self, key: &str, value: &str);
    fn read_rgb_channel_info(&self, channel_id: &str, pending: bool) -> Result<RgbInfo, io::Error>;
    fn write_rgb_channel_info(&self, channel_id: &str, rgb_info: &RgbInfo, pending: bool);
    fn remove_rgb_channel_info(&self, channel_id: &str, pending: bool) -> Result<(), io::Error>;
    fn read_rgb_payment_info(
        &self,
        payment_hash: &PaymentHash,
        inbound: bool,
    ) -> Result<RgbPaymentInfo, io::Error>;
    fn write_rgb_payment_info(
        &self,
        payment_hash: &PaymentHash,
        payment_info: &RgbPaymentInfo,
        pending: bool,
    ) -> Result<(), io::Error>;
    fn read_rgb_transfer_info(&self, txid: &str) -> Result<TransferInfo, io::Error>;
    fn read_rgb_consignment(&self, txid: &str) -> Result<Vec<u8>, io::Error>;
    fn remove_rgb_consignment(&self, txid: &str) -> Result<(), io::Error>;
}

fn channel_namespace(pending: bool) -> &'static str {
    if pending {
        RGB_CHANNEL_INFO_PENDING_NS
    } else {
        RGB_CHANNEL_INFO_NS
    }
}

fn payment_key(payment_hash: &PaymentHash, pending: bool) -> String {
    let hash = payment_hash.0.as_hex().to_string();
    if pending {
        format!("{hash}_pending")
    } else {
        hash
    }
}

impl<T: KVStoreSync + ?Sized> RgbKvStoreExt for T {
    fn write_config(&self, key: &str, value: &str) {
        let _ = self.write(
            RGB_PRIMARY_NS,
            RGB_WALLET_CONFIG_NS,
            key,
            value.as_bytes().to_vec(),
        );
    }

    fn read_rgb_channel_info(&self, channel_id: &str, pending: bool) -> Result<RgbInfo, io::Error> {
        let namespace = channel_namespace(pending);
        let bytes = self.read(RGB_PRIMARY_NS, namespace, channel_id)?;
        bincode::deserialize(&bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    fn write_rgb_channel_info(&self, channel_id: &str, rgb_info: &RgbInfo, pending: bool) {
        let namespace = channel_namespace(pending);
        let bytes = bincode::serialize(rgb_info).expect("valid RGB channel info");
        let _ = self.write(RGB_PRIMARY_NS, namespace, channel_id, bytes);
    }

    fn remove_rgb_channel_info(&self, channel_id: &str, pending: bool) -> Result<(), io::Error> {
        let namespace = channel_namespace(pending);
        self.remove(RGB_PRIMARY_NS, namespace, channel_id, false)
    }

    fn read_rgb_payment_info(
        &self,
        payment_hash: &PaymentHash,
        inbound: bool,
    ) -> Result<RgbPaymentInfo, io::Error> {
        let ns = if inbound {
            RGB_PAYMENT_INFO_INBOUND_NS
        } else {
            RGB_PAYMENT_INFO_OUTBOUND_NS
        };
        let key = payment_key(payment_hash, false);
        let bytes = self.read(RGB_PRIMARY_NS, ns, &key)?;
        bincode::deserialize(&bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    fn write_rgb_payment_info(
        &self,
        payment_hash: &PaymentHash,
        payment_info: &RgbPaymentInfo,
        pending: bool,
    ) -> Result<(), io::Error> {
        let ns = if payment_info.inbound {
            RGB_PAYMENT_INFO_INBOUND_NS
        } else {
            RGB_PAYMENT_INFO_OUTBOUND_NS
        };
        let key = payment_key(payment_hash, pending);
        let bytes = bincode::serialize(payment_info)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        self.write(RGB_PRIMARY_NS, ns, &key, bytes)
    }

    fn read_rgb_transfer_info(&self, txid: &str) -> Result<TransferInfo, io::Error> {
        let bytes = self.read(RGB_PRIMARY_NS, RGB_TRANSFER_INFO_NS, txid)?;
        bincode::deserialize(&bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    fn read_rgb_consignment(&self, txid: &str) -> Result<Vec<u8>, io::Error> {
        self.read(RGB_PRIMARY_NS, RGB_CONSIGNMENT_NS, txid)
    }

    fn remove_rgb_consignment(&self, txid: &str) -> Result<(), io::Error> {
        self.remove(RGB_PRIMARY_NS, RGB_CONSIGNMENT_NS, txid, false)
    }
}

pub(crate) fn write_rgb_payment_info_file(
    payment_hash: &PaymentHash,
    contract_id: ContractId,
    amount_rgb: u64,
    swap_payment: bool,
    inbound: bool,
    kv_store: &dyn KVStoreSync,
) {
    let payment_info = RgbPaymentInfo {
        contract_id,
        amount: amount_rgb,
        local_rgb_amount: 0,
        remote_rgb_amount: 0,
        swap_payment,
        inbound,
    };
    let _ = kv_store.write_rgb_payment_info(payment_hash, &payment_info, false);
    let _ = kv_store.write_rgb_payment_info(payment_hash, &payment_info, true);
}

pub(crate) fn is_channel_rgb(
    channel_id: &lightning::ln::types::ChannelId,
    kv_store: &dyn KVStoreSync,
) -> bool {
    kv_store
        .read_rgb_channel_info(&channel_id.to_string(), false)
        .is_ok()
}

pub(crate) fn get_rgb_channel_info_pending(
    channel_id: &lightning::ln::types::ChannelId,
    kv_store: &dyn KVStoreSync,
) -> RgbInfo {
    kv_store
        .read_rgb_channel_info(&channel_id.to_string(), true)
        .expect("rgb pending channel info exists")
}

pub(crate) fn update_rgb_channel_amount(
    channel_id: &str,
    rgb_offered_htlc: u64,
    rgb_received_htlc: u64,
    pending: bool,
    kv_store: &dyn KVStoreSync,
) {
    let mut rgb_info = kv_store
        .read_rgb_channel_info(channel_id, pending)
        .expect("rgb channel info exists");
    if rgb_offered_htlc > rgb_received_htlc {
        let spent = rgb_offered_htlc - rgb_received_htlc;
        rgb_info.local_rgb_amount -= spent;
        rgb_info.remote_rgb_amount += spent;
    } else {
        let received = rgb_received_htlc - rgb_offered_htlc;
        rgb_info.local_rgb_amount += received;
        rgb_info.remote_rgb_amount -= received;
    }
    kv_store.write_rgb_channel_info(channel_id, &rgb_info, pending);
}
