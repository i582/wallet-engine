package com.example.tonwallet.data

import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import org.ton.wallet.engine.ActivityDirection
import org.ton.wallet.engine.CreateWalletRequest
import org.ton.wallet.engine.ImportWalletRequest
import org.ton.wallet.engine.Network
import org.ton.wallet.engine.ProviderConfig
import org.ton.wallet.engine.ResourcePhase
import org.ton.wallet.engine.SendAmount
import org.ton.wallet.engine.SendPhase
import org.ton.wallet.engine.SendRequest
import org.ton.wallet.engine.WalletClient
import org.ton.wallet.engine.WalletClientConfig
import org.ton.wallet.engine.WalletLifecycle
import org.ton.wallet.engine.WalletSnapshot as EngineWalletSnapshot
import java.math.BigDecimal
import java.math.RoundingMode
import java.util.UUID

class WalletRepository(private val store: SecureWalletStore) {
    private data class Session(
        val wallet: StoredWallet,
        val httpHost: AndroidWalletHttpHost,
        val client: WalletClient,
    )

    private val lifecycle = WalletLifecycle(store)
    private val sessionMutex = Mutex()
    private val activationMutex = Mutex()
    private var session: Session? = null

    fun wallets(): List<StoredWallet> = store.wallets()
    fun selectedAddress(): String? = store.selectedAddress()
    fun select(address: String) = store.select(address)
    fun rename(address: String, name: String) = store.rename(address, name)

    suspend fun createWallet(name: String): Pair<StoredWallet, String> {
        val created = lifecycle.createWallet(
            CreateWalletRequest(
                recordId = UUID.randomUUID().toString().lowercase(),
                network = Network.TESTNET,
            ),
        )
        return try {
            val wallet = store.saveWallet(created.descriptor, name)
            wallet to created.recoveryPhrase.phrase
        } catch (error: Throwable) {
            runCatching { lifecycle.deleteWallet(created.descriptor) }
            throw error
        }
    }

    suspend fun importWallet(name: String, mnemonic: String): StoredWallet {
        val descriptor = lifecycle.importWallet(
            ImportWalletRequest(
                recordId = UUID.randomUUID().toString().lowercase(),
                network = Network.TESTNET,
                recoveryWords = mnemonic.trim().lowercase().split(Regex("\\s+")).filter(String::isNotBlank),
            ),
        )
        return try {
            store.saveWallet(descriptor, name)
        } catch (error: Throwable) {
            runCatching { lifecycle.deleteWallet(descriptor) }
            throw error
        }
    }

    suspend fun delete(wallet: StoredWallet) {
        val active = sessionMutex.withLock {
            session?.takeIf { it.wallet.recordId == wallet.recordId }?.also { session = null }
        }
        active?.client?.shutdown()
        lifecycle.deleteWallet(wallet.descriptor())
        store.deleteMetadata(wallet.address)
    }

    suspend fun refresh(wallet: StoredWallet): WalletSnapshot {
        val update = client(wallet).refresh()
        return update.snapshot.toUiSnapshot()
    }

    suspend fun loadMore(wallet: StoredWallet): WalletSnapshot {
        val update = client(wallet).loadMoreActivity()
        return update.snapshot.toUiSnapshot()
    }

    suspend fun send(wallet: StoredWallet, destination: String, amount: String) {
        val amountNanograms = gramToNanograms(amount)
        val result = client(wallet).send(
            SendRequest(
                operationId = UUID.randomUUID().toString().lowercase(),
                destination = destination.trim(),
                amount = SendAmount.Exact(nanograms = amountNanograms),
                comment = null,
            ),
        )
        when (result.phase) {
            SendPhase.SUBMITTED -> Unit
            SendPhase.SUBMISSION_UNKNOWN -> {
                val diagnostic = client(wallet).snapshot().send.errorMessage
                error(
                    buildString {
                        append("The transfer may have been submitted. Do not send it again until you verify the wallet.")
                        if (!diagnostic.isNullOrBlank()) append("\n\n$diagnostic")
                    },
                )
            }
            else -> error(client(wallet).snapshot().send.errorMessage ?: "Transfer was not submitted")
        }
    }

    suspend fun shutdown() {
        val current = sessionMutex.withLock { session.also { session = null } }
        current?.client?.shutdown()
    }

    private suspend fun client(wallet: StoredWallet): WalletClient = activationMutex.withLock {
        val existing = sessionMutex.withLock {
            session?.takeIf { it.wallet.recordId == wallet.recordId }
        }
        if (existing != null) return@withLock existing.client

        val previous = sessionMutex.withLock {
            session.also { current ->
                if (current != null) session = null
            }
        }
        previous?.client?.shutdown()

        val httpHost = AndroidWalletHttpHost()
        val candidate = Session(
            wallet = wallet,
            httpHost = httpHost,
            client = WalletClient(
                config = WalletClientConfig(
                    recordId = wallet.recordId,
                    address = wallet.address,
                    publicKey = wallet.publicKey,
                    localSecretRef = wallet.descriptor().secretRef,
                    network = wallet.network,
                    sendValiditySeconds = SEND_VALIDITY_SECONDS,
                    resolutionMarginSeconds = 60u,
                    providers = ProviderConfig(
                        toncenterBaseUrl = if (wallet.testnet) TESTNET_BASE_URL else MAINNET_BASE_URL,
                        requestTimeoutMs = PROVIDER_REQUEST_TIMEOUT_MILLIS.toULong(),
                    ),
                ),
                httpHost = httpHost,
                platformHost = store,
            ),
        )
        sessionMutex.withLock { session = candidate }
        candidate.client
    }

    private fun EngineWalletSnapshot.toUiSnapshot(): WalletSnapshot = WalletSnapshot(
        account = account?.let {
            AccountSnapshot(
                balanceNanograms = it.balanceNanograms,
                status = it.status.name.lowercase(),
                syncUtime = it.syncUtime?.toLong(),
            )
        },
        transactions = activity.map {
            WalletTransaction(
                id = it.id,
                transactionHash = it.transactionHash,
                logicalTime = it.logicalTime,
                timestamp = it.timestamp.toLong(),
                direction = when (it.direction) {
                    ActivityDirection.SENT -> "sent"
                    ActivityDirection.RECEIVED -> "received"
                },
                amountNanograms = it.amountNanograms,
                counterparty = it.counterparty,
            )
        },
        nextCursor = activityCursor?.let { TransactionCursor(it.logicalTime, it.hash) },
        canLoadMore = activityHasMore,
        accountError = accountResource.takeIf { it.phase == ResourcePhase.FAILED }
            ?.error?.developerMessage,
        activityError = activityResource.takeIf { it.phase == ResourcePhase.FAILED }
            ?.error?.developerMessage,
    )

    private fun gramToNanograms(value: String): String {
        val grams = value.trim().toBigDecimalOrNull() ?: error("Enter a valid amount")
        require(grams > BigDecimal.ZERO) { "Amount must be greater than zero" }
        return try {
            grams.movePointRight(9).setScale(0, RoundingMode.UNNECESSARY).toBigIntegerExact().toString()
        } catch (_: ArithmeticException) {
            error("GRAM supports at most 9 decimal places")
        }
    }

    private companion object {
        const val TESTNET_BASE_URL = "https://testnet.toncenter.com"
        const val MAINNET_BASE_URL = "https://toncenter.com"
        const val PROVIDER_REQUEST_TIMEOUT_MILLIS = 15_000
        val SEND_VALIDITY_SECONDS: UInt = 300u
    }
}
