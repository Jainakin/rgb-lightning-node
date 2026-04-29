#include <cstdarg>
#include <cstddef>
#include <cstdint>
#include <cstdlib>
#include <ostream>
#include <new>


enum class CResultValue {
  Ok,
  Err,
};

struct COpaqueStruct {
  const void *ptr;
  uint64_t ty;
};

struct CResultString {
  CResultValue result;
  char *inner;
};

struct CResult {
  CResultValue result;
  COpaqueStruct inner;
};


extern "C" {

void free_sdk_node(COpaqueStruct obj);

CResultString rln_address(const COpaqueStruct *node);

CResultString rln_asset_balance(const COpaqueStruct *node, const char *asset_id);

CResultString rln_asset_metadata(const COpaqueStruct *node, const char *asset_id);

CResultString rln_btc_balance(const COpaqueStruct *node, bool skip_sync);

CResultString rln_cancel_hodl_invoice(const COpaqueStruct *node, const char *request_json);

CResultString rln_check_indexer_url(const COpaqueStruct *node, const char *indexer_url);

CResultString rln_check_proxy_endpoint(const COpaqueStruct *node, const char *proxy_endpoint);

CResultString rln_claim_hodl_invoice(const COpaqueStruct *node, const char *request_json);

CResultString rln_close_channel(const COpaqueStruct *node, const char *request_json);

CResultString rln_connect_peer(const COpaqueStruct *node, const char *peer_pubkey_and_addr);

CResultString rln_create_utxos(const COpaqueStruct *node, const char *request_json);

CResultString rln_decode_ln_invoice(const COpaqueStruct *node, const char *invoice);

CResultString rln_decode_rgb_invoice(const COpaqueStruct *node, const char *invoice);

CResultString rln_disconnect_peer(const COpaqueStruct *node, const char *request_json);

CResultString rln_estimate_fee(const COpaqueStruct *node, uint16_t blocks);

CResultString rln_fail_transfers(const COpaqueStruct *node, const char *request_json);

/// Free a string previously returned in `CResultString.inner`.
void rln_free_string(char *s);

CResultString rln_get_asset_media(const COpaqueStruct *node, const char *digest);

CResultString rln_get_channel_id(const COpaqueStruct *node, const char *temporary_channel_id_hex);

CResultString rln_get_payment(const COpaqueStruct *node,
                              const char *payment_hash_hex,
                              const char *payment_type);

CResultString rln_get_swap(const COpaqueStruct *node, const char *payment_hash, bool taker_flag);

CResultString rln_inflate(const COpaqueStruct *node, const char *request_json);

CResultString rln_invoice_status(const COpaqueStruct *node, const char *invoice);

CResultString rln_issue_asset_cfa(const COpaqueStruct *node, const char *request_json);

CResultString rln_issue_asset_ifa(const COpaqueStruct *node, const char *request_json);

CResultString rln_issue_asset_nia(const COpaqueStruct *node, const char *request_json);

CResultString rln_issue_asset_uda(const COpaqueStruct *node, const char *request_json);

CResultString rln_keysend(const COpaqueStruct *node, const char *request_json);

CResultString rln_list_assets(const COpaqueStruct *node, const char *filter_asset_schemas_json);

CResultString rln_list_channels(const COpaqueStruct *node);

CResultString rln_list_payments(const COpaqueStruct *node);

CResultString rln_list_peers(const COpaqueStruct *node);

CResultString rln_list_swaps(const COpaqueStruct *node);

CResultString rln_list_transactions(const COpaqueStruct *node, bool skip_sync);

CResultString rln_list_transfers(const COpaqueStruct *node, const char *asset_id);

CResultString rln_list_unspents(const COpaqueStruct *node, bool skip_sync);

CResultString rln_ln_invoice(const COpaqueStruct *node, const char *request_json);

CResultString rln_maker_execute(const COpaqueStruct *node, const char *request_json);

CResultString rln_maker_init(const COpaqueStruct *node, const char *request_json);

CResultString rln_network_info(const COpaqueStruct *node);

CResultString rln_node_info(const COpaqueStruct *node);

CResultString rln_open_channel(const COpaqueStruct *node, const char *request_json);

CResultString rln_post_asset_media(const COpaqueStruct *node, const char *request_json);

CResultString rln_refresh_transfers(const COpaqueStruct *node, const char *request_json);

CResultString rln_rgb_invoice(const COpaqueStruct *node, const char *request_json);

CResultString rln_sdk_initialize(const char *request_json);

CResultString rln_sdk_node_init(const COpaqueStruct *node,
                                const char *password,
                                const char *mnemonic_opt);

CResult rln_sdk_node_new(const char *request_json);

CResultString rln_sdk_node_shutdown(const COpaqueStruct *node);

CResultString rln_sdk_node_unlock(const COpaqueStruct *node, const char *request_json);

CResultString rln_sdk_shutdown();

CResultString rln_send_btc(const COpaqueStruct *node, const char *request_json);

CResultString rln_send_onion_message(const COpaqueStruct *node, const char *request_json);

CResultString rln_send_payment(const COpaqueStruct *node, const char *request_json);

CResultString rln_send_rgb(const COpaqueStruct *node, const char *request_json);

CResultString rln_sign_message(const COpaqueStruct *node, const char *message);

CResultString rln_sync(const COpaqueStruct *node);

CResultString rln_taker(const COpaqueStruct *node, const char *request_json);

CResultString rln_uniffi_healthcheck();

CResultString rln_uniffi_is_initialized();

}  // extern "C"
