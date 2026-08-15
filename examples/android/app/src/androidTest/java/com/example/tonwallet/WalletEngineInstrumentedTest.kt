package com.example.tonwallet

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.example.tonwallet.data.SecureWalletStore
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.ton.wallet.engine.CreateWalletRequest
import org.ton.wallet.engine.Network
import org.ton.wallet.engine.WalletLifecycle
import java.util.UUID

@RunWith(AndroidJUnit4::class)
class WalletEngineInstrumentedTest {
    @Test
    fun lifecycleStoresAndDeletesADeviceWallet() = runBlocking {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val store = SecureWalletStore(context)
        val lifecycle = WalletLifecycle(store)
        val created = lifecycle.createWallet(
            CreateWalletRequest(
                recordId = UUID.randomUUID().toString(),
                network = Network.TESTNET,
            ),
        )

        assertEquals(24, created.recoveryPhrase.phrase.split(" ").size)
        assertTrue(created.descriptor.address.startsWith("0Q"))

        val wallet = store.saveWallet(created.descriptor, "Instrumented wallet")
        val restoredStore = SecureWalletStore(context)
        assertEquals(wallet.address, restoredStore.selectedAddress())
        assertTrue(restoredStore.wallets().any { it.recordId == wallet.recordId })

        lifecycle.deleteWallet(created.descriptor)
        store.deleteMetadata(wallet.address)
    }
}
