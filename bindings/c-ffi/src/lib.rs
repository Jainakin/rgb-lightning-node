//! C / C++ bindings for `rgb-lightning-node`.
//!
//! This crate is a thin extern-"C" shim on top of the existing UniFFI-exposed
//! API in [`rgb_lightning_node::uniffi_api`]. All complex types cross the
//! boundary as JSON strings; the same async-bridge (`block_on_sdk`) is reused,
//! so callers see a synchronous API.
//!
//! See [`README.md`](../README.md) and [`example.c`](../example.c) for usage.

mod api;
mod json_types;
mod utils;

use rgb_lightning_node::SdkNode;

use std::{
    any::TypeId,
    collections::hash_map::DefaultHasher,
    ffi::{c_char, c_void, CStr, CString},
    hash::{Hash, Hasher},
    ptr::null_mut,
};

// Re-exported so utils.rs can refer to it via `super::RlnError` without an
// explicit import in every helper.
use rgb_lightning_node::RlnError;

#[repr(C)]
pub struct COpaqueStruct {
    ptr: *const c_void,
    ty: u64,
}

#[repr(C)]
pub enum CResultValue {
    Ok,
    Err,
}

#[repr(C)]
pub struct CResult {
    result: CResultValue,
    inner: COpaqueStruct,
}

#[repr(C)]
pub struct CResultString {
    result: CResultValue,
    inner: *mut c_char,
}

// ---------------------------------------------------------------------------
// Drop / free
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn free_sdk_node(obj: COpaqueStruct) {
    unsafe {
        let _ = Box::from_raw(obj.ptr as *mut SdkNode);
    }
}

/// Free a string previously returned in `CResultString.inner`.
#[unsafe(no_mangle)]
pub extern "C" fn rln_free_string(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    unsafe {
        let _ = CString::from_raw(s);
    }
}

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn rln_sdk_node_new(request_json: *const c_char) -> CResult {
    api::sdk_node_new(request_json).into()
}

#[unsafe(no_mangle)]
pub extern "C" fn rln_sdk_node_init(
    node: &COpaqueStruct,
    password: *const c_char,
    mnemonic_opt: *const c_char,
) -> CResultString {
    api::sdk_node_init(node, password, mnemonic_opt).into()
}

#[unsafe(no_mangle)]
pub extern "C" fn rln_sdk_node_unlock(
    node: &COpaqueStruct,
    request_json: *const c_char,
) -> CResultString {
    api::sdk_node_unlock(node, request_json).into()
}

#[unsafe(no_mangle)]
pub extern "C" fn rln_sdk_node_shutdown(node: &COpaqueStruct) -> CResultString {
    api::sdk_node_shutdown(node).into()
}

// ---------------------------------------------------------------------------
// Channels / peers
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn rln_connect_peer(
    node: &COpaqueStruct,
    peer_pubkey_and_addr: *const c_char,
) -> CResultString {
    api::connect_peer(node, peer_pubkey_and_addr).into()
}

#[unsafe(no_mangle)]
pub extern "C" fn rln_disconnect_peer(
    node: &COpaqueStruct,
    request_json: *const c_char,
) -> CResultString {
    api::disconnect_peer(node, request_json).into()
}

#[unsafe(no_mangle)]
pub extern "C" fn rln_open_channel(
    node: &COpaqueStruct,
    request_json: *const c_char,
) -> CResultString {
    api::open_channel(node, request_json).into()
}

#[unsafe(no_mangle)]
pub extern "C" fn rln_close_channel(
    node: &COpaqueStruct,
    request_json: *const c_char,
) -> CResultString {
    api::close_channel(node, request_json).into()
}

#[unsafe(no_mangle)]
pub extern "C" fn rln_list_channels(node: &COpaqueStruct) -> CResultString {
    api::list_channels(node).into()
}

#[unsafe(no_mangle)]
pub extern "C" fn rln_list_peers(node: &COpaqueStruct) -> CResultString {
    api::list_peers(node).into()
}

#[unsafe(no_mangle)]
pub extern "C" fn rln_get_channel_id(
    node: &COpaqueStruct,
    temporary_channel_id_hex: *const c_char,
) -> CResultString {
    api::get_channel_id(node, temporary_channel_id_hex).into()
}

// ---------------------------------------------------------------------------
// Payments / invoices
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn rln_send_payment(
    node: &COpaqueStruct,
    request_json: *const c_char,
) -> CResultString {
    api::send_payment(node, request_json).into()
}

#[unsafe(no_mangle)]
pub extern "C" fn rln_keysend(
    node: &COpaqueStruct,
    request_json: *const c_char,
) -> CResultString {
    api::keysend(node, request_json).into()
}

#[unsafe(no_mangle)]
pub extern "C" fn rln_ln_invoice(
    node: &COpaqueStruct,
    request_json: *const c_char,
) -> CResultString {
    api::ln_invoice(node, request_json).into()
}

#[unsafe(no_mangle)]
pub extern "C" fn rln_cancel_hodl_invoice(
    node: &COpaqueStruct,
    request_json: *const c_char,
) -> CResultString {
    api::cancel_hodl_invoice(node, request_json).into()
}

#[unsafe(no_mangle)]
pub extern "C" fn rln_claim_hodl_invoice(
    node: &COpaqueStruct,
    request_json: *const c_char,
) -> CResultString {
    api::claim_hodl_invoice(node, request_json).into()
}

#[unsafe(no_mangle)]
pub extern "C" fn rln_invoice_status(
    node: &COpaqueStruct,
    invoice: *const c_char,
) -> CResultString {
    api::invoice_status(node, invoice).into()
}

#[unsafe(no_mangle)]
pub extern "C" fn rln_decode_ln_invoice(
    node: &COpaqueStruct,
    invoice: *const c_char,
) -> CResultString {
    api::decode_ln_invoice(node, invoice).into()
}

#[unsafe(no_mangle)]
pub extern "C" fn rln_decode_rgb_invoice(
    node: &COpaqueStruct,
    invoice: *const c_char,
) -> CResultString {
    api::decode_rgb_invoice(node, invoice).into()
}

#[unsafe(no_mangle)]
pub extern "C" fn rln_get_payment(
    node: &COpaqueStruct,
    payment_hash_hex: *const c_char,
    payment_type: *const c_char,
) -> CResultString {
    api::get_payment(node, payment_hash_hex, payment_type).into()
}

#[unsafe(no_mangle)]
pub extern "C" fn rln_list_payments(node: &COpaqueStruct) -> CResultString {
    api::list_payments(node).into()
}

// ---------------------------------------------------------------------------
// RGB
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn rln_rgb_invoice(
    node: &COpaqueStruct,
    request_json: *const c_char,
) -> CResultString {
    api::rgb_invoice(node, request_json).into()
}

#[unsafe(no_mangle)]
pub extern "C" fn rln_send_rgb(
    node: &COpaqueStruct,
    request_json: *const c_char,
) -> CResultString {
    api::send_rgb(node, request_json).into()
}

#[unsafe(no_mangle)]
pub extern "C" fn rln_refresh_transfers(
    node: &COpaqueStruct,
    request_json: *const c_char,
) -> CResultString {
    api::refresh_transfers(node, request_json).into()
}

#[unsafe(no_mangle)]
pub extern "C" fn rln_fail_transfers(
    node: &COpaqueStruct,
    request_json: *const c_char,
) -> CResultString {
    api::fail_transfers(node, request_json).into()
}

#[unsafe(no_mangle)]
pub extern "C" fn rln_inflate(
    node: &COpaqueStruct,
    request_json: *const c_char,
) -> CResultString {
    api::inflate(node, request_json).into()
}

#[unsafe(no_mangle)]
pub extern "C" fn rln_list_transfers(
    node: &COpaqueStruct,
    asset_id: *const c_char,
) -> CResultString {
    api::list_transfers(node, asset_id).into()
}

#[unsafe(no_mangle)]
pub extern "C" fn rln_list_unspents(node: &COpaqueStruct, skip_sync: bool) -> CResultString {
    api::list_unspents(node, skip_sync).into()
}

#[unsafe(no_mangle)]
pub extern "C" fn rln_post_asset_media(
    node: &COpaqueStruct,
    request_json: *const c_char,
) -> CResultString {
    api::post_asset_media(node, request_json).into()
}

#[unsafe(no_mangle)]
pub extern "C" fn rln_get_asset_media(
    node: &COpaqueStruct,
    digest: *const c_char,
) -> CResultString {
    api::get_asset_media(node, digest).into()
}

// ---------------------------------------------------------------------------
// Asset issuance / metadata
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn rln_issue_asset_nia(
    node: &COpaqueStruct,
    request_json: *const c_char,
) -> CResultString {
    api::issue_asset_nia(node, request_json).into()
}

#[unsafe(no_mangle)]
pub extern "C" fn rln_issue_asset_cfa(
    node: &COpaqueStruct,
    request_json: *const c_char,
) -> CResultString {
    api::issue_asset_cfa(node, request_json).into()
}

#[unsafe(no_mangle)]
pub extern "C" fn rln_issue_asset_ifa(
    node: &COpaqueStruct,
    request_json: *const c_char,
) -> CResultString {
    api::issue_asset_ifa(node, request_json).into()
}

#[unsafe(no_mangle)]
pub extern "C" fn rln_issue_asset_uda(
    node: &COpaqueStruct,
    request_json: *const c_char,
) -> CResultString {
    api::issue_asset_uda(node, request_json).into()
}

#[unsafe(no_mangle)]
pub extern "C" fn rln_list_assets(
    node: &COpaqueStruct,
    filter_asset_schemas_json: *const c_char,
) -> CResultString {
    api::list_assets(node, filter_asset_schemas_json).into()
}

#[unsafe(no_mangle)]
pub extern "C" fn rln_asset_balance(
    node: &COpaqueStruct,
    asset_id: *const c_char,
) -> CResultString {
    api::asset_balance(node, asset_id).into()
}

#[unsafe(no_mangle)]
pub extern "C" fn rln_asset_metadata(
    node: &COpaqueStruct,
    asset_id: *const c_char,
) -> CResultString {
    api::asset_metadata(node, asset_id).into()
}

// ---------------------------------------------------------------------------
// Node info / network / btc / address / sign / fee / indexer / utxos / sync
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn rln_node_info(node: &COpaqueStruct) -> CResultString {
    api::node_info(node).into()
}

#[unsafe(no_mangle)]
pub extern "C" fn rln_network_info(node: &COpaqueStruct) -> CResultString {
    api::network_info(node).into()
}

#[unsafe(no_mangle)]
pub extern "C" fn rln_address(node: &COpaqueStruct) -> CResultString {
    api::address(node).into()
}

#[unsafe(no_mangle)]
pub extern "C" fn rln_btc_balance(node: &COpaqueStruct, skip_sync: bool) -> CResultString {
    api::btc_balance(node, skip_sync).into()
}

#[unsafe(no_mangle)]
pub extern "C" fn rln_sign_message(
    node: &COpaqueStruct,
    message: *const c_char,
) -> CResultString {
    api::sign_message(node, message).into()
}

#[unsafe(no_mangle)]
pub extern "C" fn rln_estimate_fee(node: &COpaqueStruct, blocks: u16) -> CResultString {
    api::estimate_fee(node, blocks).into()
}

#[unsafe(no_mangle)]
pub extern "C" fn rln_check_indexer_url(
    node: &COpaqueStruct,
    indexer_url: *const c_char,
) -> CResultString {
    api::check_indexer_url(node, indexer_url).into()
}

#[unsafe(no_mangle)]
pub extern "C" fn rln_check_proxy_endpoint(
    node: &COpaqueStruct,
    proxy_endpoint: *const c_char,
) -> CResultString {
    api::check_proxy_endpoint(node, proxy_endpoint).into()
}

#[unsafe(no_mangle)]
pub extern "C" fn rln_send_btc(
    node: &COpaqueStruct,
    request_json: *const c_char,
) -> CResultString {
    api::send_btc(node, request_json).into()
}

#[unsafe(no_mangle)]
pub extern "C" fn rln_create_utxos(
    node: &COpaqueStruct,
    request_json: *const c_char,
) -> CResultString {
    api::create_utxos(node, request_json).into()
}

#[unsafe(no_mangle)]
pub extern "C" fn rln_list_transactions(
    node: &COpaqueStruct,
    skip_sync: bool,
) -> CResultString {
    api::list_transactions(node, skip_sync).into()
}

#[unsafe(no_mangle)]
pub extern "C" fn rln_sync(node: &COpaqueStruct) -> CResultString {
    api::sync(node).into()
}

// ---------------------------------------------------------------------------
// Swaps / onion
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn rln_maker_init(
    node: &COpaqueStruct,
    request_json: *const c_char,
) -> CResultString {
    api::maker_init(node, request_json).into()
}

#[unsafe(no_mangle)]
pub extern "C" fn rln_maker_execute(
    node: &COpaqueStruct,
    request_json: *const c_char,
) -> CResultString {
    api::maker_execute(node, request_json).into()
}

#[unsafe(no_mangle)]
pub extern "C" fn rln_taker(
    node: &COpaqueStruct,
    request_json: *const c_char,
) -> CResultString {
    api::taker(node, request_json).into()
}

#[unsafe(no_mangle)]
pub extern "C" fn rln_send_onion_message(
    node: &COpaqueStruct,
    request_json: *const c_char,
) -> CResultString {
    api::send_onion_message(node, request_json).into()
}

#[unsafe(no_mangle)]
pub extern "C" fn rln_get_swap(
    node: &COpaqueStruct,
    payment_hash: *const c_char,
    taker_flag: bool,
) -> CResultString {
    api::get_swap(node, payment_hash, taker_flag).into()
}

#[unsafe(no_mangle)]
pub extern "C" fn rln_list_swaps(node: &COpaqueStruct) -> CResultString {
    api::list_swaps(node).into()
}

// ---------------------------------------------------------------------------
// Module-level (no handle): healthcheck / global init / global shutdown
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn rln_uniffi_healthcheck() -> CResultString {
    api::uniffi_healthcheck().into()
}

#[unsafe(no_mangle)]
pub extern "C" fn rln_uniffi_is_initialized() -> CResultString {
    api::uniffi_is_initialized().into()
}

#[unsafe(no_mangle)]
pub extern "C" fn rln_sdk_initialize(request_json: *const c_char) -> CResultString {
    api::sdk_global_initialize(request_json).into()
}

#[unsafe(no_mangle)]
pub extern "C" fn rln_sdk_shutdown() -> CResultString {
    api::sdk_global_shutdown().into()
}
