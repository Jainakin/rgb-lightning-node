package org.rgblightningnode

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Test
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.runner.RunWith
import org.utexo.rgblightningnode.AssetRecipients
import org.utexo.rgblightningnode.AssignmentKind
import org.utexo.rgblightningnode.ContractId
import org.utexo.rgblightningnode.HtlcStatus
import org.utexo.rgblightningnode.InvoiceStatus
import org.utexo.rgblightningnode.LnInvoiceRequest
import org.utexo.rgblightningnode.Payment
import org.utexo.rgblightningnode.PaymentHash
import org.utexo.rgblightningnode.PaymentType
import org.utexo.rgblightningnode.RgbRecipient
import org.utexo.rgblightningnode.RlnException
import org.utexo.rgblightningnode.SdkCloseChannelRequest
import org.utexo.rgblightningnode.SdkCreateUtxosRequest
import org.utexo.rgblightningnode.SdkInitRequest
import org.utexo.rgblightningnode.SdkIssueAssetNiaRequest
import org.utexo.rgblightningnode.SdkNode
import org.utexo.rgblightningnode.SdkOpenChannelRequest
import org.utexo.rgblightningnode.SdkRefreshTransfersRequest
import org.utexo.rgblightningnode.SdkRgbInvoiceRequest
import org.utexo.rgblightningnode.SdkSendPaymentRequest
import org.utexo.rgblightningnode.SdkUnlockRequest
import org.utexo.rgblightningnode.SendRgbRequest
import org.utexo.rgblightningnode.TransactionType
import org.utexo.rgblightningnode.Txid
import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import java.security.MessageDigest
import java.util.Base64

@RunWith(AndroidJUnit4::class)
class PaymentTest {

    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val storageBase = context.filesDir.absolutePath

    private val bitcoindHost = "10.0.2.2"
    private val bitcoindPort = 18443
    private val bitcoindUser = "user"
    private val bitcoindPass = "password"
    private val proxyEndpoint = "rpc://10.0.2.2:3000/json-rpc"

    private val nodeADaemonPort: UShort = 3711u
    private val nodeBDaemonPort: UShort = 3712u
    private val nodeCDaemonPort: UShort = 3713u
    private val nodeAPeerPort: UShort = 13111u
    private val nodeBPeerPort: UShort = 13112u
    private val nodeCPeerPort: UShort = 13113u

    // Keep Android smoke aligned with the stable host Kotlin payment scenario.
    private val channelCapacitySat: ULong = 500_000u
    private val channelPushMsat: ULong = 0u
    private val paymentMsat: ULong = 3_000_000u
    private val utxosNum: UByte = 10u
    private val utxosSizeSat: UInt = 100_000u
    private val utxosFeeRate: ULong = 1u
    private val assetSupply: ULong = 1000u
    private val channelAssetAmount: ULong = 200u
    private val paymentAssetAmount: ULong = 50u
    /** CI emulators are slow; keep generous margins vs host-side Kotlin E2E. */
    private val channelReadyTimeoutSec: Long = 180L
    private val channelFundingTxTimeoutSec: Long = 180L
    private val paymentStatusTimeoutSec: Long = 120L
    private val lnBalanceWaitTimeoutSec: Long = 120L
    private val stableChannelBalanceTimeoutSec: Long = 120L

    private fun assertRegtestNetwork(label: String, network: String) {
        assertTrue(
            "$label: expected regtest-class network, got '$network'",
            network.contains("egtest", ignoreCase = true),
        )
    }

    // ── Bitcoin RPC ──────────────────────────────────────────────────────────

    private fun bitcoindRpc(method: String, vararg params: Any): JSONObject {
        val url = URL("http://$bitcoindHost:$bitcoindPort/")
        val conn = url.openConnection() as HttpURLConnection
        conn.requestMethod = "POST"
        conn.doOutput = true
        conn.connectTimeout = 15_000
        conn.readTimeout = 60_000
        conn.setRequestProperty("Content-Type", "application/json")
        val creds = Base64.getEncoder().encodeToString("$bitcoindUser:$bitcoindPass".toByteArray())
        conn.setRequestProperty("Authorization", "Basic $creds")

        val body = JSONObject().apply {
            put("jsonrpc", "1.0")
            put("id", "android-e2e")
            put("method", method)
            put("params", JSONArray().apply { params.forEach { put(it) } })
        }.toString()
        conn.outputStream.use { it.write(body.toByteArray()) }

        val response = conn.inputStream.bufferedReader().readText()
        return JSONObject(response)
    }

    private fun mine(blocks: Int) {
        val addrResp = bitcoindRpc("getnewaddress")
        val addr = addrResp.getString("result")
        bitcoindRpc("generatetoaddress", blocks, addr)
        log("mined $blocks block(s)")
    }

    private fun sendToAddress(address: String, amountBtc: String) {
        bitcoindRpc("sendtoaddress", address, amountBtc.toDouble())
        log("sent $amountBtc BTC to $address")
    }

    // ── Node helpers ─────────────────────────────────────────────────────────

    private fun makeNode(name: String, daemonPort: UShort, peerPort: UShort): SdkNode {
        return SdkNode.create(
            SdkInitRequest(
                storageDirPath = "$storageBase/$name",
                daemonListeningPort = daemonPort,
                ldkPeerListeningPort = peerPort,
                network = "regtest",
                maxMediaUploadSizeMb = 20u,
                enableVirtualChannelsV0 = false,
                virtualPeerPubkeys = null,
                lspBaseUrl = null,
                lspBearerToken = null,
            )
        )
    }

    private fun unlockRequest(password: String) = SdkUnlockRequest(
        password = password,
        bitcoindRpcUsername = bitcoindUser,
        bitcoindRpcPassword = bitcoindPass,
        bitcoindRpcHost = bitcoindHost,
        bitcoindRpcPort = bitcoindPort.toUShort(),
        indexerUrl = "$bitcoindHost:50001",
        proxyEndpoint = proxyEndpoint,
        announceAddresses = listOf(),
        announceAlias = null,
    )

    private fun initNode(node: SdkNode, password: String, name: String) {
        node.init(password, null)
        log("$name: initialized")
    }

    private fun unlockNode(node: SdkNode, password: String, name: String) {
        node.unlock(unlockRequest(password))
        log("$name: unlocked")
    }

    private fun ensureFunded(node: SdkNode, name: String, minSat: ULong, amountBtc: String) {
        val spendable = node.btcBalance(false).vanilla.spendable
        log("$name spendable: $spendable sat")
        if (spendable >= minSat) return
        val address = node.address().address
        sendToAddress(address, amountBtc)
        mine(6)
        node.sync()
        val after = node.btcBalance(false).vanilla.spendable
        log("$name spendable after fund: $after sat")
        assertTrue("$name still underfunded: $after < $minSat", after >= minSat)
    }

    private fun createUtxos(node: SdkNode, name: String) {
        node.createutxos(
            SdkCreateUtxosRequest(
                upTo = false,
                num = utxosNum,
                size = utxosSizeSat,
                feeRate = utxosFeeRate,
                skipSync = false,
            )
        )
        log("$name: createutxos done")
        mine(1)
        node.sync()
    }

    private fun waitForPeer(node: SdkNode, peerPubkey: Any, timeoutSec: Long) {
        val expected = peerPubkey.toString()
        val deadline = System.currentTimeMillis() + timeoutSec * 1_000L
        while (System.currentTimeMillis() < deadline) {
            if (node.listPeers().any { it.pubkey.toString() == expected }) {
                return
            }
            log("waiting for peer connection: $expected")
            Thread.sleep(1_000L)
        }
        error("peer did not appear in listPeers() after ${timeoutSec}s: peer=$expected")
    }

    private fun assetBalanceSpendable(node: SdkNode, assetId: ContractId): ULong =
        node.assetBalance(assetId).spendable

    private fun assetBalanceOffchainOutbound(node: SdkNode, assetId: ContractId): ULong =
        node.assetBalance(assetId).offchainOutbound

    private fun waitForLnBalance(node: SdkNode, assetId: ContractId, expected: ULong, timeoutSec: Long) {
        val deadline = System.currentTimeMillis() + timeoutSec * 1_000L
        var lastBalance = 0uL
        while (System.currentTimeMillis() < deadline) {
            node.sync()
            val balance = assetBalanceOffchainOutbound(node, assetId)
            lastBalance = balance
            if (balance == expected) {
                return
            }
            node.refreshtransfers(SdkRefreshTransfersRequest(skipSync = false))
            Thread.sleep(1_000L)
        }
        error("offchain_outbound balance did not become expected=$expected actual=$lastBalance after ${timeoutSec}s")
    }

    private fun waitForBalance(node: SdkNode, assetId: ContractId, expected: ULong, timeoutSec: Long) {
        val deadline = System.currentTimeMillis() + timeoutSec * 1_000L
        var lastBalance = 0uL
        while (System.currentTimeMillis() < deadline) {
            node.sync()
            val balance = assetBalanceSpendable(node, assetId)
            lastBalance = balance
            if (balance == expected) {
                return
            }
            node.refreshtransfers(SdkRefreshTransfersRequest(skipSync = false))
            Thread.sleep(1_000L)
        }
        error("spendable balance did not become expected=$expected actual=$lastBalance after ${timeoutSec}s")
    }

    private fun waitForChannelFundingTx(nodeA: SdkNode, nodeB: SdkNode, assetId: ContractId, timeoutSec: Long): Txid {
        val deadline = System.currentTimeMillis() + timeoutSec * 1_000L
        while (System.currentTimeMillis() < deadline) {
            nodeA.sync(); nodeB.sync()
            val opening = nodeA.listChannels().firstOrNull { it.assetId == assetId && it.fundingTxid != null }
            if (opening != null) {
                log("channel funding tx found: ${opening.fundingTxid}")
                return requireNotNull(opening.fundingTxid)
            }
            log("waiting for channel funding tx...")
            Thread.sleep(1_000L)
        }
        error("no channel funding tx after ${timeoutSec}s")
    }

    private fun mineUntilTxConfirmed(node: SdkNode, txid: Txid, timeoutSec: Long = 180L) {
        val deadline = System.currentTimeMillis() + timeoutSec * 1_000L
        while (System.currentTimeMillis() < deadline) {
            node.sync()
            val tx = node.listTransactions(false).firstOrNull { it.txid == txid }
            if (tx != null && tx.confirmationTime != null) {
                log("funding tx confirmed in block: $txid")
                return
            }
            log("waiting for funding tx to be included in a block...")
            mine(1)
            Thread.sleep(1_000L)
        }
        error("funding tx was not confirmed before timeout: txid=$txid")
    }

    private fun waitForUsableChannel(nodeA: SdkNode, nodeB: SdkNode, assetId: ContractId, timeoutSec: Long) {
        val deadline = System.currentTimeMillis() + timeoutSec * 1_000L
        var polls = 0
        while (System.currentTimeMillis() < deadline) {
            polls++
            nodeA.sync(); nodeB.sync()
            val usable = nodeA.listChannels().any { it.isUsable && it.assetId == assetId }
            if (usable) { log("channel is usable"); return }
            if (polls % 5 == 0) { log("mining 1 block..."); mine(1) }
            log("waiting for usable channel... (poll $polls)")
            Thread.sleep(2_000L)
        }
        error("channel not usable after ${timeoutSec}s")
    }

    private fun waitForStableChannelBalances(
        nodeA: SdkNode,
        nodeB: SdkNode,
        channelId: String,
        expectedNodeABalance: ULong,
        expectedNodeBBalance: ULong,
        timeoutSec: Long,
    ) {
        val deadline = System.currentTimeMillis() + timeoutSec * 1_000L
        var lastNodeABalance: ULong? = null
        var lastNodeBBalance: ULong? = null
        while (System.currentTimeMillis() < deadline) {
            nodeA.sync()
            nodeB.sync()
            val channelA = nodeA.listChannels().firstOrNull { it.channelId == channelId }
            val channelB = nodeB.listChannels().firstOrNull { it.channelId == channelId }
            lastNodeABalance = channelA?.localBalanceSat
            lastNodeBBalance = channelB?.localBalanceSat
            if (lastNodeABalance == expectedNodeABalance && lastNodeBBalance == expectedNodeBBalance) {
                return
            }
            Thread.sleep(1_000L)
        }
        error(
            "channel balances did not stabilize after ${timeoutSec}s: " +
                "expectedA=$expectedNodeABalance actualA=$lastNodeABalance " +
                "expectedB=$expectedNodeBBalance actualB=$lastNodeBBalance"
        )
    }

    private fun waitPaymentFinal(node: SdkNode, invoice: String, timeoutSec: Long = 120L): InvoiceStatus {
        val deadline = System.currentTimeMillis() + timeoutSec * 1_000L
        var last = InvoiceStatus.PENDING
        while (System.currentTimeMillis() < deadline) {
            node.sync()
            val status = node.invoiceStatus(invoice)
            last = status
            if (status == InvoiceStatus.SUCCEEDED || status == InvoiceStatus.FAILED || status == InvoiceStatus.EXPIRED) {
                return status
            }
            Thread.sleep(1_000L)
        }
        error("invoice did not finalize after ${timeoutSec}s, last=$last")
    }

    private fun waitForPaymentStatus(
        node: SdkNode,
        paymentHash: PaymentHash,
        paymentType: PaymentType,
        timeoutSec: Long,
    ): Payment {
        val deadline = System.currentTimeMillis() + timeoutSec * 1_000L
        var last = "not found"
        while (System.currentTimeMillis() < deadline) {
            val payment = node.listPayments().firstOrNull {
                it.paymentHash == paymentHash && it.paymentType == paymentType
            }
            if (payment != null) {
                last = payment.status.name
                if (payment.status == HtlcStatus.SUCCEEDED) {
                    return payment
                }
            }
            Thread.sleep(1_000L)
        }
        error("payment did not succeed after ${timeoutSec}s, paymentType=$paymentType, last=$last")
    }

    private fun waitForPaymentPresentInList(
        node: SdkNode,
        paymentHash: PaymentHash,
        paymentType: PaymentType,
        timeoutSec: Long,
    ): Payment {
        val deadline = System.currentTimeMillis() + timeoutSec * 1_000L
        var lastCount = 0
        while (System.currentTimeMillis() < deadline) {
            val payments = node.listPayments()
            lastCount = payments.size
            val payment = payments.firstOrNull {
                it.paymentHash == paymentHash && it.paymentType == paymentType
            }
            if (payment != null) {
                return payment
            }
            Thread.sleep(1_000L)
        }
        error(
            "payment not found in listPayments: paymentHash=$paymentHash paymentType=$paymentType " +
                "list_size=$lastCount after ${timeoutSec}s"
        )
    }

    private fun sendPaymentWithLnBalance(
        sender: SdkNode,
        receiver: SdkNode,
        invoice: String,
        assetId: ContractId,
        assetAmount: ULong,
        initialSenderBalance: ULong,
        initialReceiverBalance: ULong,
    ) {
        sender.sendpayment(
            SdkSendPaymentRequest(
                invoice = invoice,
                amtMsat = null,
                assetId = null,
                assetAmount = null,
            )
        )
        waitForLnBalance(sender, assetId, initialSenderBalance - assetAmount, lnBalanceWaitTimeoutSec)
        waitForLnBalance(receiver, assetId, initialReceiverBalance + assetAmount, lnBalanceWaitTimeoutSec)
    }

    private fun closeChannel(node: SdkNode, channelId: String, peerPubkey: String, force: Boolean = false) {
        node.closechannel(
            SdkCloseChannelRequest(
                channelId = channelId,
                peerPubkey = peerPubkey,
                force = force,
            )
        )

        val deadline = System.currentTimeMillis() + 30_000L
        var lastChannels = "no channels"
        while (System.currentTimeMillis() < deadline) {
            val channels = node.listChannels()
            lastChannels = channels.joinToString { it.channelId }.ifEmpty { "no channels" }
            if (channels.none { it.channelId == channelId }) {
                mine(if (force) 144 else 6)
                return
            }
            Thread.sleep(1_000L)
        }
        error("channel did not close in time: channelId=$channelId remainingChannels=$lastChannels")
    }

    private fun refreshTransfers(node: SdkNode) {
        node.refreshtransfers(SdkRefreshTransfersRequest(skipSync = false))
    }

    private fun rgbInvoice(node: SdkNode): String {
        return node.rgbinvoice(
            SdkRgbInvoiceRequest(
                assetId = null,
                assignmentKind = null,
                assignmentAmount = null,
                durationSeconds = null,
                minConfirmations = 1u,
                witness = false,
            )
        ).recipientId
    }

    private fun sendRgb(node: SdkNode, assetId: ContractId, recipientId: String, amount: ULong) {
        node.sendRgb(
            SendRgbRequest(
                donation = true,
                feeRate = utxosFeeRate,
                minConfirmations = 1u,
                skipSync = false,
                recipientGroups = listOf(
                    AssetRecipients(
                        assetId = assetId,
                        recipients = listOf(
                            RgbRecipient(
                                recipientId = recipientId,
                                witnessData = null,
                                assignmentKind = AssignmentKind.FUNGIBLE,
                                assignmentAmount = amount,
                                transportEndpoints = listOf(proxyEndpoint),
                            )
                        ),
                    )
                ),
            )
        )
    }

    private fun hexToBytes(value: String): ByteArray {
        require(value.length % 2 == 0) { "hex string must have even length" }
        return ByteArray(value.length / 2) { index ->
            value.substring(index * 2, index * 2 + 2).toInt(16).toByte()
        }
    }

    private fun sha256Hex(bytes: ByteArray): String =
        MessageDigest.getInstance("SHA-256")
            .digest(bytes)
            .joinToString("") { "%02x".format(it.toInt() and 0xff) }

    private fun checkPreimageMatchesHash(payment: Payment, expectedPaymentHash: PaymentHash) {
        val paymentPreimage = requireNotNull(payment.preimage) { "payment preimage is null" }
        val paymentPreimageHash = sha256Hex(hexToBytes(paymentPreimage))
        assertEquals(expectedPaymentHash, paymentPreimageHash)
    }

    private fun log(msg: String) {
        android.util.Log.i("PaymentTest", msg)
    }

    private fun safeShutdown(node: SdkNode?) {
        try {
            node?.shutdown()
        } catch (_: Exception) {
        }
    }

    // ── Test ─────────────────────────────────────────────────────────────────

    @Test
    fun payment() {
        File("$storageBase/payment/node_a").deleteRecursively()
        File("$storageBase/payment/node_b").deleteRecursively()

        val nodeA = makeNode("payment/node_a", nodeADaemonPort, nodeAPeerPort)
        val nodeB = makeNode("payment/node_b", nodeBDaemonPort, nodeBPeerPort)
        try {
            initNode(nodeA, "nodeApass", "node A")
            initNode(nodeB, "nodeBpass", "node B")
            unlockNode(nodeA, "nodeApass", "node A")
            unlockNode(nodeB, "nodeBpass", "node B")

            ensureFunded(nodeA, "node A", channelCapacitySat + 200_000u, "0.02")
            ensureFunded(nodeB, "node B", 200_000u, "0.02")
            createUtxos(nodeA, "node A")
            createUtxos(nodeB, "node B")

            val assetId = nodeA.issueassetnia(
                SdkIssueAssetNiaRequest(
                    amounts = listOf(assetSupply),
                    ticker = "USDT",
                    name = "Tether",
                    precision = 0u,
                )
            ).assetId
            log("issued asset: $assetId")

            val infoA = nodeA.nodeInfo(); val infoB = nodeB.nodeInfo()
            log("node A pubkey: ${infoA.pubkey}")
            log("node B pubkey: ${infoB.pubkey}")

            val peerUri = "${infoB.pubkey}@127.0.0.1:${nodeBPeerPort.toInt()}"
            try {
                nodeA.connectpeer(peerUri)
                log("connectpeer: ok")
            } catch (_: RlnException.Conflict) {
                log("connectpeer: already connected")
            }
            waitForPeer(nodeA, infoB.pubkey, 20L)

            nodeA.openchannel(
                SdkOpenChannelRequest(
                    peerPubkeyAndOptAddr = peerUri,
                    capacitySat = channelCapacitySat,
                    pushMsat = channelPushMsat,
                    `public` = false,
                    withAnchors = true,
                    feeBaseMsat = null,
                    feeProportionalMillionths = null,
                    temporaryChannelId = null,
                    assetId = assetId,
                    assetAmount = channelAssetAmount,
                    pushAssetAmount = null,
                    virtualOpenMode = null,
                )
            )
            log("openchannel sent")

            val fundingTxid = waitForChannelFundingTx(nodeA, nodeB, assetId, channelFundingTxTimeoutSec)
            log("Mining blocks one by one until funding tx is confirmed..."); mineUntilTxConfirmed(nodeA, fundingTxid)
            mine(6)
            waitForUsableChannel(nodeA, nodeB, assetId, channelReadyTimeoutSec)
            assertEquals(0uL, assetBalanceSpendable(nodeB, assetId))

            val invoice1 = nodeB.lnInvoice(
                LnInvoiceRequest(
                    amtMsat = paymentMsat,
                    expirySec = 900u,
                    assetId = assetId,
                    assetAmount = paymentAssetAmount,
                    descriptionHash = null,
                    paymentHash = null,
                )
            ).invoice
            sendPaymentWithLnBalance(
                nodeA,
                nodeB,
                invoice1,
                assetId,
                paymentAssetAmount,
                channelAssetAmount,
                0u,
            )

            val decoded1 = nodeA.decodeLnInvoice(invoice1)
            assertEquals(assetId, decoded1.assetId)
            assertEquals(paymentAssetAmount, decoded1.assetAmount)
            assertEquals(paymentMsat, decoded1.amtMsat)
            assertEquals(900uL, decoded1.expirySec)
            assertEquals(infoB.pubkey, decoded1.payeePubkey)
            assertRegtestNetwork("decode invoice1", decoded1.network)
            assertEquals(InvoiceStatus.SUCCEEDED, nodeB.invoiceStatus(invoice1))

            val payment1Sender = waitForPaymentStatus(
                nodeA,
                decoded1.paymentHash,
                PaymentType.OUTBOUND,
                paymentStatusTimeoutSec,
            )
            assertEquals(HtlcStatus.SUCCEEDED, payment1Sender.status)
            assertEquals(assetId, payment1Sender.assetId)
            assertEquals(paymentAssetAmount, payment1Sender.assetAmount)
            checkPreimageMatchesHash(payment1Sender, decoded1.paymentHash)

            val payment1Receiver = waitForPaymentStatus(
                nodeB,
                decoded1.paymentHash,
                PaymentType.INBOUND_AUTO_CLAIM,
                paymentStatusTimeoutSec,
            )
            assertEquals(HtlcStatus.SUCCEEDED, payment1Receiver.status)
            assertEquals(assetId, payment1Receiver.assetId)
            assertEquals(paymentAssetAmount, payment1Receiver.assetAmount)
            checkPreimageMatchesHash(payment1Receiver, decoded1.paymentHash)

            val listedPayment1Sender = waitForPaymentPresentInList(
                nodeA,
                decoded1.paymentHash,
                PaymentType.OUTBOUND,
                paymentStatusTimeoutSec,
            )
            assertEquals(decoded1.paymentHash, listedPayment1Sender.paymentHash)
            checkPreimageMatchesHash(listedPayment1Sender, decoded1.paymentHash)

            val listedPayment1Receiver = waitForPaymentPresentInList(
                nodeB,
                decoded1.paymentHash,
                PaymentType.INBOUND_AUTO_CLAIM,
                paymentStatusTimeoutSec,
            )
            assertEquals(decoded1.paymentHash, listedPayment1Receiver.paymentHash)
            checkPreimageMatchesHash(listedPayment1Receiver, decoded1.paymentHash)
            log("SUCCESS: Android payment smoke flow completed")
        } finally {
            safeShutdown(nodeA)
            safeShutdown(nodeB)
            Thread.sleep(1_000L)
        }
    }
}
