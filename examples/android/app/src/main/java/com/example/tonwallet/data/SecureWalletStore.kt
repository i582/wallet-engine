package com.example.tonwallet.data

import android.content.Context
import android.content.pm.PackageManager
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.security.keystore.StrongBoxUnavailableException
import android.util.Base64
import org.json.JSONArray
import org.json.JSONObject
import org.ton.wallet.engine.JournalCompareExchange
import org.ton.wallet.engine.JournalCompareExchangeResult
import org.ton.wallet.engine.JournalHostErrorKind
import org.ton.wallet.engine.JournalHostException
import org.ton.wallet.engine.JournalKey
import org.ton.wallet.engine.JournalRecord
import org.ton.wallet.engine.Network
import org.ton.wallet.engine.ProtectedSecretHostErrorKind
import org.ton.wallet.engine.ProtectedSecretHostException
import org.ton.wallet.engine.ProtectedSecretRead
import org.ton.wallet.engine.ProtectedSecretRef
import org.ton.wallet.engine.ProtectedSecretStore
import org.ton.wallet.engine.WalletDescriptor
import org.ton.wallet.engine.WalletPlatformHost
import java.security.KeyStore
import java.security.MessageDigest
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

class SecureWalletStore(private val context: Context) : WalletPlatformHost {
    private val preferences = context.getSharedPreferences("wallet_engine_example", Context.MODE_PRIVATE)
    private val keyStore = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
    private val journalLock = Any()

    fun wallets(): List<StoredWallet> {
        val array = JSONArray(preferences.getString(KEY_WALLETS, "[]"))
        return buildList {
            for (index in 0 until array.length()) {
                val item = array.getJSONObject(index)
                add(
                    StoredWallet(
                        recordId = item.getString("record_id"),
                        address = item.getString("address"),
                        name = item.getString("name"),
                        network = Network.valueOf(item.getString("network")),
                        secretRef = item.getString("secret_ref"),
                    ),
                )
            }
        }
    }

    fun selectedAddress(): String? = preferences.getString(KEY_SELECTED, null)

    fun saveWallet(descriptor: WalletDescriptor, name: String): StoredWallet {
        val wallet = StoredWallet(
            recordId = descriptor.recordId,
            address = descriptor.address,
            name = name.trim().ifBlank { "My Wallet" }.take(32),
            network = descriptor.network,
            secretRef = descriptor.secretRef.value,
        )
        val updated = wallets().filterNot { it.recordId == wallet.recordId } + wallet
        check(
            preferences.edit()
                .putString(KEY_WALLETS, encodeWallets(updated))
                .putString(KEY_SELECTED, wallet.address)
                .commit(),
        ) { "Could not persist wallet metadata" }
        return wallet
    }

    fun select(address: String) {
        require(wallets().any { it.address == address }) { "Unknown wallet" }
        check(preferences.edit().putString(KEY_SELECTED, address).commit()) {
            "Could not persist selected wallet"
        }
    }

    fun rename(address: String, name: String) {
        val updated = wallets().map { wallet ->
            if (wallet.address == address) wallet.copy(name = name) else wallet
        }
        check(preferences.edit().putString(KEY_WALLETS, encodeWallets(updated)).commit()) {
            "Could not persist wallet name"
        }
    }

    fun deleteMetadata(address: String) {
        val updated = wallets().filterNot { it.address == address }
        val nextSelected = selectedAddress().takeIf { selected ->
            updated.any { it.address == selected }
        } ?: updated.firstOrNull()?.address
        val edit = preferences.edit().putString(KEY_WALLETS, encodeWallets(updated))
        if (nextSelected == null) edit.remove(KEY_SELECTED) else edit.putString(KEY_SELECTED, nextSelected)
        check(edit.commit()) { "Could not delete wallet metadata" }
    }

    override suspend fun readProtectedSecret(request: ProtectedSecretRead): ByteArray = secretOperation {
        val payload = preferences.getString(secretKey(request.secretRef), null)
            ?: throw ProtectedSecretHostException.Failed(
                ProtectedSecretHostErrorKind.NOT_FOUND,
                "Protected secret does not exist",
            )
        decrypt(payload)
    }

    override suspend fun storeProtectedSecret(request: ProtectedSecretStore) = secretOperation {
        val encrypted = encrypt(request.bytes)
        check(preferences.edit().putString(secretKey(request.secretRef), encrypted).commit()) {
            "Could not persist protected secret"
        }
    }

    override suspend fun deleteProtectedSecret(secretRef: ProtectedSecretRef) = secretOperation {
        check(preferences.edit().remove(secretKey(secretRef)).commit()) {
            "Could not delete protected secret"
        }
    }

    override suspend fun loadJournal(key: JournalKey): JournalRecord? = journalOperation {
        synchronized(journalLock) { readJournal(key) }
    }

    override suspend fun compareExchangeJournal(
        mutation: JournalCompareExchange,
    ): JournalCompareExchangeResult = journalOperation {
        synchronized(journalLock) {
            val current = readJournal(mutation.key)
            if (current?.version != mutation.expectedVersion) {
                return@synchronized JournalCompareExchangeResult(false, current)
            }
            val encoded = JSONObject()
                .put("version", mutation.replacement.version.toString())
                .put("payload", Base64.encodeToString(mutation.replacement.payload, Base64.NO_WRAP))
                .toString()
            check(preferences.edit().putString(journalKey(mutation.key), encoded).commit()) {
                "Could not persist send journal"
            }
            JournalCompareExchangeResult(true, mutation.replacement)
        }
    }

    private fun readJournal(key: JournalKey): JournalRecord? {
        val encoded = preferences.getString(journalKey(key), null) ?: return null
        val value = JSONObject(encoded)
        return JournalRecord(
            version = value.getString("version").toULong(),
            payload = Base64.decode(value.getString("payload"), Base64.NO_WRAP),
        )
    }

    private fun encodeWallets(wallets: List<StoredWallet>) = JSONArray().apply {
        wallets.forEach { wallet ->
            put(
                JSONObject()
                    .put("record_id", wallet.recordId)
                    .put("address", wallet.address)
                    .put("name", wallet.name)
                    .put("network", wallet.network.name)
                    .put("secret_ref", wallet.secretRef),
            )
        }
    }.toString()

    private fun encrypt(value: ByteArray): String {
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.ENCRYPT_MODE, secretKey())
        val ciphertext = cipher.doFinal(value)
        return JSONObject()
            .put("version", ENVELOPE_VERSION)
            .put("iv", Base64.encodeToString(cipher.iv, Base64.NO_WRAP))
            .put("ciphertext", Base64.encodeToString(ciphertext, Base64.NO_WRAP))
            .toString()
    }

    private fun decrypt(payload: String): ByteArray {
        val envelope = JSONObject(payload)
        require(envelope.getInt("version") == ENVELOPE_VERSION) { "Unsupported secret format" }
        val iv = Base64.decode(envelope.getString("iv"), Base64.NO_WRAP)
        val ciphertext = Base64.decode(envelope.getString("ciphertext"), Base64.NO_WRAP)
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.DECRYPT_MODE, secretKey(), GCMParameterSpec(GCM_TAG_BITS, iv))
        return cipher.doFinal(ciphertext)
    }

    private fun secretKey(): SecretKey {
        (keyStore.getKey(KEY_ALIAS, null) as? SecretKey)?.let { return it }
        val strongBox = context.packageManager.hasSystemFeature(PackageManager.FEATURE_STRONGBOX_KEYSTORE)
        if (strongBox) {
            try {
                return generateKey(true)
            } catch (_: StrongBoxUnavailableException) {
                // Some devices report StrongBox before a key slot is available.
            }
        }
        return generateKey(false)
    }

    private fun generateKey(strongBox: Boolean): SecretKey {
        val spec = KeyGenParameterSpec.Builder(
            KEY_ALIAS,
            KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
        )
            .setKeySize(AES_KEY_BITS)
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            .setRandomizedEncryptionRequired(true)
            .setIsStrongBoxBacked(strongBox)
            .build()
        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore").run {
            init(spec)
            generateKey()
        }
    }

    private fun secretKey(reference: ProtectedSecretRef): String =
        "secret_${digest(reference.value)}"

    private fun journalKey(key: JournalKey): String =
        "journal_${digest("${key.recordId}:${key.slot}")}"

    private fun digest(value: String): String = Base64.encodeToString(
        MessageDigest.getInstance("SHA-256").digest(value.toByteArray()),
        Base64.NO_WRAP or Base64.URL_SAFE,
    )

    private inline fun <T> secretOperation(block: () -> T): T = try {
        block()
    } catch (error: ProtectedSecretHostException) {
        throw error
    } catch (error: Throwable) {
        throw ProtectedSecretHostException.Failed(
            ProtectedSecretHostErrorKind.UNAVAILABLE,
            error.message?.take(256) ?: "Protected storage failed",
        )
    }

    private inline fun <T> journalOperation(block: () -> T): T = try {
        block()
    } catch (error: JournalHostException) {
        throw error
    } catch (error: Throwable) {
        throw JournalHostException.Failed(
            JournalHostErrorKind.UNAVAILABLE,
            error.message?.take(256) ?: "Journal storage failed",
        )
    }

    private companion object {
        const val KEY_ALIAS = "wallet_engine_example_secret_v1"
        const val KEY_WALLETS = "wallets"
        const val KEY_SELECTED = "selected_address"
        const val TRANSFORMATION = "AES/GCM/NoPadding"
        const val AES_KEY_BITS = 256
        const val GCM_TAG_BITS = 128
        const val ENVELOPE_VERSION = 1
    }
}
