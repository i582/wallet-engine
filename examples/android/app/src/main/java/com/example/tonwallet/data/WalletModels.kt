package com.example.tonwallet.data

import org.ton.wallet.engine.Network
import org.ton.wallet.engine.ProtectedSecretRef
import org.ton.wallet.engine.WalletDescriptor
import java.math.BigInteger

data class StoredWallet(
    val recordId: String,
    val address: String,
    val publicKey: ByteArray,
    val name: String,
    val network: Network,
    val secretRef: String,
) {
    val testnet: Boolean get() = network == Network.TESTNET

    fun descriptor(): WalletDescriptor = WalletDescriptor(
        recordId = recordId,
        address = address,
        publicKey = publicKey,
        network = network,
        secretRef = ProtectedSecretRef(secretRef),
    )
}

data class AccountSnapshot(
    val balanceNanograms: String,
    val status: String,
    val syncUtime: Long,
) {
    val balanceGrams: String get() = formatNanograms(balanceNanograms)
}

data class WalletTransaction(
    val id: String,
    val transactionHash: String,
    val logicalTime: String,
    val timestamp: Long,
    val direction: String,
    val amountNanograms: String,
    val counterparty: String?,
) {
    val amountGrams: String get() = formatNanograms(amountNanograms)
    val isReceived: Boolean get() = direction == "received"
}

data class TransactionCursor(val logicalTime: String, val hash: String)

data class WalletSnapshot(
    val account: AccountSnapshot?,
    val transactions: List<WalletTransaction>,
    val nextCursor: TransactionCursor?,
    val canLoadMore: Boolean,
    val accountError: String?,
    val activityError: String?,
)

private val nanogramsPerGram = BigInteger("1000000000")

fun formatNanograms(value: String): String {
    val nanograms = value.toBigIntegerOrNull() ?: return value
    val (whole, remainder) = nanograms.divideAndRemainder(nanogramsPerGram)
    val fraction = remainder.abs().toString().padStart(9, '0').trimEnd('0')
    return if (fraction.isEmpty()) whole.toString() else "$whole.$fraction"
}
