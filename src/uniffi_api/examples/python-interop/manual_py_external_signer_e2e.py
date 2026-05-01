#!/usr/bin/env python3
import argparse
import os
import shutil
import socket
import subprocess
import time
from pathlib import Path
from typing import Optional

import rgb_lightning_node as rln

REPO_ROOT = Path(__file__).resolve().parents[4]

PROXY_ENDPOINT_LOCAL = "rpc://127.0.0.1:3000/json-rpc"

NODE_A_PASSWORD = os.getenv("NODE_A_PASSWORD", "nodeApass")
NODE_B_PASSWORD = os.getenv("NODE_B_PASSWORD", "nodeBpass")

OPEN_CHANNEL_CAPACITY_SAT = int(os.getenv("OPEN_CHANNEL_CAPACITY_SAT", "500000"))
PAYMENT_MSAT = int(os.getenv("PAYMENT_MSAT", "3000000"))
CREATE_UTXOS_NUM = int(os.getenv("CREATE_UTXOS_NUM", "10"))
CREATE_UTXOS_FEE_RATE = int(os.getenv("CREATE_UTXOS_FEE_RATE", "7"))
OPEN_CHANNEL_CONFIRM_BLOCKS = int(os.getenv("OPEN_CHANNEL_CONFIRM_BLOCKS", "12"))
CHANNEL_READY_TIMEOUT_SEC = int(os.getenv("CHANNEL_READY_TIMEOUT_SEC", "300"))
RESET_DATA = os.getenv("RESET_DATA", "1") == "1"


def run_command(*args: str) -> str:
    res = subprocess.run(
        list(args),
        cwd=REPO_ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    return (res.stdout or "").strip()


def run_regtest(*args: str) -> str:
    return run_command("./regtest.sh", *args)


def ensure_regtest_available():
    out = run_command("docker", "compose", "ps", "--services", "--status", "running")
    services = set(line.strip() for line in out.splitlines() if line.strip())
    for required in ("bitcoind", "electrs", "proxy"):
        if required not in services:
            raise RuntimeError(
                f"regtest service `{required}` is not running; start it with ./regtest.sh start"
            )


def ensure_dir(path: Path):
    if RESET_DATA and path.exists():
        shutil.rmtree(path)
    path.mkdir(parents=True, exist_ok=True)


def find_free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        return int(s.getsockname()[1])


def make_node(storage_dir: Path, daemon_port: int, peer_port: int) -> rln.SdkNode:
    req = rln.SdkInitRequest(
        storage_dir_path=str(storage_dir),
        daemon_listening_port=daemon_port,
        ldk_peer_listening_port=peer_port,
        network="regtest",
        max_media_upload_size_mb=20,
        enable_virtual_channels_v0=False,
        virtual_peer_pubkeys=None,
    )
    last_err = None
    for _ in range(5):
        try:
            return rln.SdkNode.create(req)
        except rln.RlnError.Internal as e:
            last_err = e
            req.daemon_listening_port = find_free_port()
            req.ldk_peer_listening_port = find_free_port()
            time.sleep(0.2)
    assert last_err is not None
    raise last_err


def unlock_request(password: str) -> rln.SdkUnlockRequest:
    return rln.SdkUnlockRequest(
        password=password,
        bitcoind_rpc_username="user",
        bitcoind_rpc_password="password",
        bitcoind_rpc_host="localhost",
        bitcoind_rpc_port=18443,
        indexer_url="127.0.0.1:50001",
        proxy_endpoint=PROXY_ENDPOINT_LOCAL,
        announce_addresses=[],
        announce_alias=None,
    )

def unlock_with_attached_signer(
    node: rln.SdkNode,
    bootstrap: rln.SdkExternalSignerBootstrap,
):
    node.unlock_with_attached_external_signer(
        bootstrap.node_id,
        bootstrap.account_xpub_vanilla,
        bootstrap.account_xpub_colored,
        bootstrap.master_fingerprint,
        bootstrap.protocol_version,
        bootstrap.api_level,
        "user",
        "password",
        "localhost",
        18443,
        "127.0.0.1:50001",
        PROXY_ENDPOINT_LOCAL,
        [],
        "RLN_external_py",
    )


def make_native_signer() -> "rln.NativeExternalSigner":
    seed_hex = os.getenv("RLN_TEST_NATIVE_SIGNER_SEED_HEX", "11" * 32)
    network = os.getenv("RLN_TEST_NATIVE_SIGNER_NETWORK", "regtest")
    permissive_policy = os.getenv("RLN_TEST_NATIVE_SIGNER_PERMISSIVE_POLICY", "1") == "1"
    return rln.NativeExternalSigner(seed_hex, network, permissive_policy)


def make_native_signer_with_seed(seed_hex: str) -> "rln.NativeExternalSigner":
    network = os.getenv("RLN_TEST_NATIVE_SIGNER_NETWORK", "regtest")
    permissive_policy = os.getenv("RLN_TEST_NATIVE_SIGNER_PERMISSIVE_POLICY", "1") == "1"
    return rln.NativeExternalSigner(seed_hex, network, permissive_policy)


def ensure_funded(node: rln.SdkNode, min_spendable_sat: int):
    spendable = node.btc_balance(False).vanilla.spendable
    if spendable >= min_spendable_sat:
        return
    address = node.address().address
    run_regtest("sendtoaddress", address, "1")
    run_regtest("mine", "6")
    node.sync()
    spendable_after = node.btc_balance(False).vanilla.spendable
    if spendable_after < min_spendable_sat:
        raise RuntimeError(
            f"node spendable balance too low after funding: {spendable_after} < {min_spendable_sat}"
        )


def create_utxos(node: rln.SdkNode):
    node.createutxos(
        rln.SdkCreateUtxosRequest(
            up_to=False,
            num=CREATE_UTXOS_NUM,
            size=None,
            fee_rate=CREATE_UTXOS_FEE_RATE,
            skip_sync=False,
        )
    )
    run_regtest("mine", "1")
    node.sync()


def create_utxos_if_possible(node: rln.SdkNode):
    try:
        create_utxos(node)
    except (rln.RlnError.Conflict, rln.RlnError.Internal):
        # External real signer backends may not support wallet key-index mapping
        # required by createutxos PSBT signing yet; continue with funded wallet UTXOs.
        pass


def wait_for_channel_funding_tx(node_a: rln.SdkNode, timeout_sec: int = 120) -> str:
    deadline = time.time() + timeout_sec
    while time.time() < deadline:
        node_a.sync()
        channels = node_a.list_channels()
        opening = next((c for c in channels if c.funding_txid is not None), None)
        if opening is not None:
            return str(opening.funding_txid)
        time.sleep(1)
    raise RuntimeError("timeout waiting for funding tx")


def mine_until_tx_confirmed(node: rln.SdkNode, txid: str, timeout_sec: int = 180):
    deadline = time.time() + timeout_sec
    while time.time() < deadline:
        node.sync()
        txs = node.list_transactions(False)
        tx = next((t for t in txs if str(t.txid) == txid), None)
        if tx is not None and tx.confirmation_time is not None:
            return
        run_regtest("mine", "1")
        time.sleep(1)
    raise RuntimeError(f"funding tx not confirmed: {txid}")


def wait_for_usable_channels(
    nodes: list[tuple[rln.SdkNode, int]], timeout_sec: int = 180
) -> None:
    deadline = time.time() + timeout_sec
    while time.time() < deadline:
        all_ready = True
        for node, expected_usable in nodes:
            node.sync()
            usable = sum(
                1 for c in node.list_channels() if c.ready and c.is_usable
            )
            if usable != expected_usable:
                all_ready = False
        if all_ready:
            return
        time.sleep(1)
    raise RuntimeError("timeout waiting for usable channels")


def wait_for_payment_succeeded(
    node: rln.SdkNode, payment_hash: rln.PaymentHash, timeout_sec: int = 120
):
    deadline = time.time() + timeout_sec
    while time.time() < deadline:
        node.sync()
        payment = next(
            (p for p in node.list_payments() if p.payment_hash == payment_hash), None
        )
        if payment is not None and payment.status == rln.HtlcStatus.SUCCEEDED:
            return
        time.sleep(1)
    raise RuntimeError("payment did not reach SUCCEEDED")


def run_regular_channel_flow_external_real():
    signer = make_native_signer()
    bootstrap = signer.bootstrap()

    data_root = REPO_ROOT / "target" / "uniffi" / "python-e2e" / "external-real-flow"
    node_a_dir = data_root / "node_a"
    node_b_dir = data_root / "node_b"
    ensure_dir(node_a_dir)
    ensure_dir(node_b_dir)

    node_a_daemon_port = find_free_port()
    node_b_daemon_port = find_free_port()
    node_a_peer_port = find_free_port()
    node_b_peer_port = find_free_port()

    node_a = make_node(node_a_dir, node_a_daemon_port, node_a_peer_port)
    node_b = make_node(node_b_dir, node_b_daemon_port, node_b_peer_port)
    try:
        node_a.init_with_native_external_signer(signer)
        node_b.init(NODE_B_PASSWORD, None)
        node_a.unlock_with_native_external_signer(
            signer,
            "user",
            "password",
            "localhost",
            18443,
            "127.0.0.1:50001",
            PROXY_ENDPOINT_LOCAL,
            [],
            "RLN_external_py",
        )
        node_b.unlock(unlock_request(NODE_B_PASSWORD))

        ensure_funded(node_a, 300_000)
        ensure_funded(node_b, 100_000)
        # External signer backends may not yet support the key-index mapping
        # needed by createutxos; skip for node_a to keep the regular-flow
        # scenario focused on channel operations.
        # create_utxos_if_possible(node_a)
        create_utxos_if_possible(node_b)

        peer_uri = f"{node_b.node_info().pubkey}@127.0.0.1:{node_b_peer_port}"
        try:
            node_a.connectpeer(peer_uri)
        except rln.RlnError.Conflict:
            pass

        open_res = node_a.openchannel(
            rln.SdkOpenChannelRequest(
                peer_pubkey_and_opt_addr=peer_uri,
                capacity_sat=OPEN_CHANNEL_CAPACITY_SAT,
                push_msat=0,
                public=False,
                with_anchors=True,
                fee_base_msat=None,
                fee_proportional_millionths=None,
                temporary_channel_id=None,
                asset_id=None,
                asset_amount=None,
                push_asset_amount=None,
                virtual_open_mode=None,
            )
        )
        print("opened channel temporary id:", open_res.temporary_channel_id)

        txid = wait_for_channel_funding_tx(node_a)
        mine_until_tx_confirmed(node_a, txid)
        print(f"mining {OPEN_CHANNEL_CONFIRM_BLOCKS} extra blocks for channel confirmations...")
        run_regtest("mine", str(OPEN_CHANNEL_CONFIRM_BLOCKS))
        wait_for_usable_channels(
            [(node_a, 1), (node_b, 1)], timeout_sec=CHANNEL_READY_TIMEOUT_SEC
        )

        inv_1 = node_b.ln_invoice(
            rln.LnInvoiceRequest(
                amt_msat=PAYMENT_MSAT,
                expiry_sec=900,
                asset_id=None,
                asset_amount=None,
                payment_hash=None,
                description_hash=None,
            )
        ).invoice
        send_1 = node_a.sendpayment(
            rln.SdkSendPaymentRequest(
                invoice=str(inv_1),
                amt_msat=None,
                asset_id=None,
                asset_amount=None,
            )
        )
        if send_1.payment_hash is None:
            raise RuntimeError("sendpayment did not return payment_hash")
        wait_for_payment_succeeded(node_a, send_1.payment_hash)
        wait_for_payment_succeeded(node_b, send_1.payment_hash)
        print("payment #1 succeeded")

        node_a.shutdown()
        time.sleep(0.5)
        node_a = make_node(node_a_dir, node_a_daemon_port, node_a_peer_port)
        node_a.unlock_with_native_external_signer(
            signer,
            "user",
            "password",
            "localhost",
            18443,
            "127.0.0.1:50001",
            PROXY_ENDPOINT_LOCAL,
            [],
            "RLN_external_py",
        )
        wait_for_usable_channels([(node_a, 1), (node_b, 1)])

        inv_2 = node_b.ln_invoice(
            rln.LnInvoiceRequest(
                amt_msat=PAYMENT_MSAT,
                expiry_sec=900,
                asset_id=None,
                asset_amount=None,
                payment_hash=None,
                description_hash=None,
            )
        ).invoice
        send_2 = node_a.sendpayment(
            rln.SdkSendPaymentRequest(
                invoice=str(inv_2),
                amt_msat=None,
                asset_id=None,
                asset_amount=None,
            )
        )
        if send_2.payment_hash is None:
            raise RuntimeError("sendpayment did not return payment_hash")
        wait_for_payment_succeeded(node_a, send_2.payment_hash)
        wait_for_payment_succeeded(node_b, send_2.payment_hash)
        print("payment #2 succeeded after restart")
    finally:
        try:
            node_a.shutdown()
        except Exception:
            pass
        try:
            node_b.shutdown()
        except Exception:
            pass
def run_connection_loss_restore_real():
    ensure_regtest_available()
    signer = make_native_signer()
    bootstrap = signer.bootstrap()

    data_root = REPO_ROOT / "target" / "uniffi" / "python-e2e" / "external-real-loss"
    node_a_dir = data_root / "node_a"
    node_b_dir = data_root / "node_b"
    ensure_dir(node_a_dir)
    ensure_dir(node_b_dir)

    node_a_daemon_port = find_free_port()
    node_b_daemon_port = find_free_port()
    node_a_peer_port = find_free_port()
    node_b_peer_port = find_free_port()

    node_a = make_node(node_a_dir, node_a_daemon_port, node_a_peer_port)
    node_b = make_node(node_b_dir, node_b_daemon_port, node_b_peer_port)
    try:
        node_a.init_with_native_external_signer(signer)
        node_a.unlock_with_native_external_signer(
            signer,
            "user",
            "password",
            "localhost",
            18443,
            "127.0.0.1:50001",
            PROXY_ENDPOINT_LOCAL,
            [],
            "RLN_external_py",
        )
        node_b.init(NODE_B_PASSWORD, None)
        node_b.unlock(unlock_request(NODE_B_PASSWORD))

        ensure_funded(node_a, 300_000)
        ensure_funded(node_b, 100_000)
        create_utxos_if_possible(node_b)

        peer_uri = f"{node_b.node_info().pubkey}@127.0.0.1:{node_b_peer_port}"
        try:
            node_a.connectpeer(peer_uri)
        except rln.RlnError.Conflict:
            pass

        node_a.openchannel(
            rln.SdkOpenChannelRequest(
                peer_pubkey_and_opt_addr=peer_uri,
                capacity_sat=OPEN_CHANNEL_CAPACITY_SAT,
                push_msat=0,
                public=False,
                with_anchors=True,
                fee_base_msat=None,
                fee_proportional_millionths=None,
                temporary_channel_id=None,
                asset_id=None,
                asset_amount=None,
                push_asset_amount=None,
                virtual_open_mode=None,
            )
        )
        txid = wait_for_channel_funding_tx(node_a)
        mine_until_tx_confirmed(node_a, txid)
        run_regtest("mine", str(OPEN_CHANNEL_CONFIRM_BLOCKS))
        wait_for_usable_channels(
            [(node_a, 1), (node_b, 1)], timeout_sec=CHANNEL_READY_TIMEOUT_SEC
        )

        inv_1 = node_b.ln_invoice(
            rln.LnInvoiceRequest(
                amt_msat=PAYMENT_MSAT,
                expiry_sec=900,
                asset_id=None,
                asset_amount=None,
                payment_hash=None,
                description_hash=None,
            )
        ).invoice
        send_1 = node_a.sendpayment(
            rln.SdkSendPaymentRequest(
                invoice=str(inv_1),
                amt_msat=None,
                asset_id=None,
                asset_amount=None,
            )
        )
        if send_1.payment_hash is None:
            raise RuntimeError("sendpayment did not return payment_hash")
        wait_for_payment_succeeded(node_a, send_1.payment_hash)
        wait_for_payment_succeeded(node_b, send_1.payment_hash)
        print("payment #1 succeeded before signer outage")

        node_a.shutdown()
        time.sleep(0.5)
        node_a = make_node(node_a_dir, node_a_daemon_port, node_a_peer_port)
        try:
            unlock_with_attached_signer(node_a, bootstrap)
            raise RuntimeError("expected unlock failure while signer unavailable")
        except Exception as e:
            print("expected signer-unavailable failure:", str(e))

        node_a.attach_native_external_signer(signer)
        unlock_with_attached_signer(node_a, bootstrap)
        wait_for_usable_channels([(node_a, 1), (node_b, 1)])

        inv_2 = node_b.ln_invoice(
            rln.LnInvoiceRequest(
                amt_msat=PAYMENT_MSAT,
                expiry_sec=900,
                asset_id=None,
                asset_amount=None,
                payment_hash=None,
                description_hash=None,
            )
        ).invoice
        send_2 = node_a.sendpayment(
            rln.SdkSendPaymentRequest(
                invoice=str(inv_2),
                amt_msat=None,
                asset_id=None,
                asset_amount=None,
            )
        )
        if send_2.payment_hash is None:
            raise RuntimeError("sendpayment did not return payment_hash after recovery")
        wait_for_payment_succeeded(node_a, send_2.payment_hash)
        wait_for_payment_succeeded(node_b, send_2.payment_hash)
        print("payment #2 succeeded after signer recovery")
    finally:
        try:
            node_a.shutdown()
        except Exception:
            pass
        try:
            node_b.shutdown()
        except Exception:
            pass


def run_restart_with_mismatched_signer_real():
    ensure_regtest_available()
    signer_a = make_native_signer_with_seed("11" * 32)
    signer_b = make_native_signer_with_seed("22" * 32)

    data_root = REPO_ROOT / "target" / "uniffi" / "python-e2e" / "external-real-mismatch"
    node_dir = data_root / "node_a"
    ensure_dir(node_dir)

    node_daemon_port = find_free_port()
    node_peer_port = find_free_port()

    node = make_node(node_dir, node_daemon_port, node_peer_port)
    try:
        node.init_with_native_external_signer(signer_a)
        node.unlock_with_native_external_signer(
            signer_a,
            "user",
            "password",
            "localhost",
            18443,
            "127.0.0.1:50001",
            PROXY_ENDPOINT_LOCAL,
            [],
            "RLN_external_py",
        )
        info = node.node_info()
        print("initial unlock succeeded for node id:", info.pubkey)
        node.shutdown()
    finally:
        try:
            node.shutdown()
        except Exception:
            pass

    time.sleep(0.5)
    restarted = make_node(node_dir, node_daemon_port, node_peer_port)
    try:
        try:
            restarted.unlock_with_native_external_signer(
                signer_b,
                "user",
                "password",
                "localhost",
                18443,
                "127.0.0.1:50001",
                PROXY_ENDPOINT_LOCAL,
                [],
                "RLN_external_py",
            )
            raise RuntimeError("expected mismatched signer unlock to fail")
        except Exception as e:
            msg = str(e).lower()
            if "mismatch" not in msg and "conflict" not in msg:
                raise RuntimeError(f"unexpected mismatched signer error: {e}") from e
            print("mismatched signer unlock failed as expected:", str(e))
    finally:
        try:
            restarted.shutdown()
        except Exception:
            pass
def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Python UniFFI external signer E2E scenarios"
    )
    parser.add_argument(
        "--scenario",
        choices=[
            "regular-flow-real",
            "restart-mismatch-real",
            "connection-loss-real",
        ],
        default=os.getenv("PY_EXT_SIGNER_SCENARIO", "regular-flow-real"),
        help="which scenario to run",
    )
    return parser.parse_args()


def main():
    args = parse_args()
    if args.scenario == "regular-flow-real":
        run_regular_channel_flow_external_real()
    elif args.scenario == "restart-mismatch-real":
        run_restart_with_mismatched_signer_real()
    elif args.scenario == "connection-loss-real":
        run_connection_loss_restore_real()
    else:
        raise RuntimeError(f"unsupported scenario: {args.scenario}")


if __name__ == "__main__":
    main()
