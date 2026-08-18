package com.example.tonwallet

import android.app.Application
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import com.example.tonwallet.data.AccountSnapshot
import com.example.tonwallet.data.SecureWalletStore
import com.example.tonwallet.data.StoredWallet
import com.example.tonwallet.data.TransactionCursor
import com.example.tonwallet.data.WalletRepository
import com.example.tonwallet.data.WalletSnapshot
import com.example.tonwallet.data.WalletTransaction
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

data class WalletUiState(
    val wallets: List<StoredWallet> = emptyList(),
    val selectedAddress: String? = null,
    val account: AccountSnapshot? = null,
    val transactions: List<WalletTransaction> = emptyList(),
    val usdPerTon: Double? = null,
    val nextCursor: TransactionCursor? = null,
    val canLoadMore: Boolean = false,
    val isRefreshing: Boolean = false,
    val isLoadingMore: Boolean = false,
    val isSending: Boolean = false,
    val canForceRetry: Boolean = false,
    val sendError: String? = null,
    val isLiveConnected: Boolean = false,
    val error: String? = null,
    val notice: String? = null,
    val newRecoveryPhrase: String? = null,
) {
    val activeWallet: StoredWallet?
        get() = wallets.firstOrNull { it.address == selectedAddress } ?: wallets.firstOrNull()
}

class WalletViewModel(application: Application) : AndroidViewModel(application) {
    private val repository = WalletRepository(SecureWalletStore(application))
    private val mutableState = MutableStateFlow(restoreState())
    val state: StateFlow<WalletUiState> = mutableState.asStateFlow()
    private var autoRefreshEnabled = false
    private var autoRefreshJob: Job? = null

    init {
        if (mutableState.value.activeWallet != null) refresh()
    }

    fun createWallet(name: String) {
        viewModelScope.launch {
            runCatching { repository.createWallet(name) }
                .onSuccess { (_, mnemonic) ->
                    mutableState.value = restoreState().copy(newRecoveryPhrase = mnemonic)
                    refresh()
                    restartAutoRefresh()
                }
                .onFailure(::showError)
        }
    }

    fun importWallet(name: String, mnemonic: String) {
        viewModelScope.launch {
            runCatching { repository.importWallet(name, mnemonic) }
                .onSuccess {
                    mutableState.value = restoreState().copy(notice = "Wallet imported")
                    refresh()
                    restartAutoRefresh()
                }
                .onFailure(::showError)
        }
    }

    fun selectWallet(address: String) {
        repository.select(address)
        mutableState.value = restoreState().copy(isRefreshing = true)
        refresh()
        restartAutoRefresh()
    }

    fun renameWallet(name: String) {
        val wallet = mutableState.value.activeWallet ?: return
        val normalized = name.trim().take(32)
        if (normalized.isBlank()) return
        repository.rename(wallet.address, normalized)
        mutableState.value = mutableState.value.copy(wallets = repository.wallets())
    }

    fun deleteWallet() {
        val wallet = mutableState.value.activeWallet ?: return
        viewModelScope.launch {
            runCatching { repository.delete(wallet) }
                .onSuccess {
                    mutableState.value = restoreState().copy(notice = "Wallet removed from this device")
                    if (mutableState.value.activeWallet != null) refresh()
                    restartAutoRefresh()
                }
                .onFailure(::showError)
        }
    }

    fun startAutoRefresh() {
        autoRefreshEnabled = true
        restartAutoRefresh()
    }

    fun stopAutoRefresh() {
        autoRefreshEnabled = false
        autoRefreshJob?.cancel()
        autoRefreshJob = null
    }

    fun refresh() {
        val wallet = mutableState.value.activeWallet ?: return
        if (mutableState.value.isRefreshing) return
        mutableState.value = mutableState.value.copy(isRefreshing = true, error = null)
        viewModelScope.launch {
            runCatching { repository.refresh(wallet) }
                .onSuccess { snapshot ->
                    if (mutableState.value.activeWallet?.recordId != wallet.recordId) return@onSuccess
                    publish(snapshot, isRefreshing = false)
                }
                .onFailure { error ->
                    if (mutableState.value.activeWallet?.recordId == wallet.recordId) {
                        mutableState.value = mutableState.value.copy(
                            isRefreshing = false,
                            error = friendlyError(error.message),
                        )
                    }
                }
        }
    }

    fun loadMore() {
        val current = mutableState.value
        val wallet = current.activeWallet ?: return
        if (!current.canLoadMore || current.isLoadingMore) return
        mutableState.value = current.copy(isLoadingMore = true)
        viewModelScope.launch {
            runCatching { repository.loadMore(wallet) }
                .onSuccess { snapshot ->
                    if (mutableState.value.activeWallet?.recordId == wallet.recordId) {
                        publish(snapshot, isLoadingMore = false)
                    }
                }
                .onFailure { error ->
                    mutableState.value = mutableState.value.copy(
                        isLoadingMore = false,
                        error = friendlyError(error.message),
                    )
                }
        }
    }

    fun send(destination: String, amount: String, force: Boolean) {
        val wallet = mutableState.value.activeWallet ?: return
        mutableState.value = mutableState.value.copy(isSending = true, sendError = null, error = null)
        viewModelScope.launch {
            runCatching { repository.send(wallet, destination, amount, force) }
                .onSuccess {
                    mutableState.value = mutableState.value.copy(
                        isSending = false,
                        notice = "Transfer submitted to testnet",
                    )
                    refresh()
                }
                .onFailure { error ->
                    runCatching { repository.snapshot(wallet) }
                        .getOrNull()
                        ?.let { publish(it) }
                    mutableState.value = mutableState.value.copy(
                        isSending = false,
                        sendError = friendlyError(error.message),
                    )
                }
        }
    }

    fun clearSendError() {
        mutableState.value = mutableState.value.copy(sendError = null)
    }

    fun clearRecoveryPhrase() {
        mutableState.value = mutableState.value.copy(newRecoveryPhrase = null)
    }

    fun consumeMessages() {
        mutableState.value = mutableState.value.copy(error = null, notice = null)
    }

    private fun publish(
        snapshot: WalletSnapshot,
        isRefreshing: Boolean = mutableState.value.isRefreshing,
        isLoadingMore: Boolean = mutableState.value.isLoadingMore,
    ) {
        mutableState.value = mutableState.value.copy(
            account = snapshot.account ?: mutableState.value.account,
            transactions = snapshot.transactions,
            nextCursor = snapshot.nextCursor,
            canLoadMore = snapshot.canLoadMore,
            canForceRetry = snapshot.canForceRetry,
            isRefreshing = isRefreshing,
            isLoadingMore = isLoadingMore,
            error = snapshot.accountError ?: snapshot.activityError,
        )
    }

    private fun restoreState(): WalletUiState {
        val wallets = repository.wallets()
        val selected = repository.selectedAddress().takeIf { address ->
            wallets.any { it.address == address }
        } ?: wallets.firstOrNull()?.address
        return WalletUiState(wallets = wallets, selectedAddress = selected)
    }

    private fun restartAutoRefresh() {
        autoRefreshJob?.cancel()
        autoRefreshJob = null
        if (!autoRefreshEnabled || mutableState.value.activeWallet == null) return
        autoRefreshJob = viewModelScope.launch {
            while (isActive) {
                delay(AUTO_REFRESH_INTERVAL_MILLIS)
                refresh()
            }
        }
    }

    private fun showError(error: Throwable) {
        mutableState.value = mutableState.value.copy(error = friendlyError(error.message))
    }

    private fun friendlyError(message: String?): String = when {
        message.isNullOrBlank() -> "Something went wrong"
        "resolve" in message.lowercase() -> "No internet connection"
        "429" in message -> "Too many requests. Please try again in a moment."
        "not enough" in message.lowercase() || "insufficient" in message.lowercase() ->
            "Not enough balance to cover the amount and network fees."
        else -> message
    }

    private companion object {
        const val AUTO_REFRESH_INTERVAL_MILLIS = 30_000L
    }
}
