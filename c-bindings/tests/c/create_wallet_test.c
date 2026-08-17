#include "wallet_engine.h"

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#define TEXT_CAPACITY 256

typedef struct TestContext {
    size_t retains;
    size_t releases;
    size_t stores;
    size_t results;
    bool valid;
    size_t recovery_word_count;
    WalletEngineNetwork network;
    char record_id[TEXT_CAPACITY];
    char address[TEXT_CAPACITY];
    char stored_secret_ref[TEXT_CAPACITY];
    char result_secret_ref[TEXT_CAPACITY];
} TestContext;

static bool copy_string(char destination[TEXT_CAPACITY], WalletEngineStringView source) {
    if (source.len >= TEXT_CAPACITY || (source.data == NULL && source.len != 0)) {
        return false;
    }
    if (source.len != 0) {
        memcpy(destination, source.data, source.len);
    }
    destination[source.len] = '\0';
    return true;
}

static size_t count_phrase_words(WalletEngineStringView phrase) {
    size_t count = 0;
    bool in_word = false;
    for (size_t index = 0; index < phrase.len; ++index) {
        if (phrase.data[index] == ' ') {
            in_word = false;
        } else if (!in_word) {
            ++count;
            in_word = true;
        }
    }
    return count;
}

static void retain_context(void *context) {
    TestContext *test = context;
    ++test->retains;
}

static void release_context(void *context) {
    TestContext *test = context;
    ++test->releases;
}

static void store_protected_secret(
    void *context,
    const WalletEngineProtectedSecretStoreView *request,
    void *result_context,
    WalletEngineProtectedSecretStoreResultFn result
) {
    TestContext *test = context;
    ++test->stores;
    if (request == NULL || request->bytes.data == NULL || request->bytes.len == 0 ||
        !request->require_user_presence ||
        !copy_string(test->stored_secret_ref, request->secret_ref.value)) {
        test->valid = false;
    }
    if (result == NULL || result(result_context, NULL) != WALLET_ENGINE_ABI_STATUS_OK) {
        test->valid = false;
    }
}

static void create_wallet_result(
    void *context,
    WalletEngineAbiStatus abi_status,
    const WalletEngineCreatedWalletView *wallet,
    const WalletEngineWalletLifecycleErrorView *error
) {
    TestContext *test = context;
    ++test->results;
    if (abi_status != WALLET_ENGINE_ABI_STATUS_OK || wallet == NULL || error != NULL) {
        test->valid = false;
        return;
    }

    const WalletEngineStringView phrase = wallet->recovery_phrase.phrase;
    if (!copy_string(test->record_id, wallet->descriptor.record_id) ||
        !copy_string(test->address, wallet->descriptor.address) ||
        !copy_string(test->result_secret_ref, wallet->descriptor.secret_ref.value) ||
        phrase.data == NULL || phrase.len == 0) {
        test->valid = false;
    }
    test->network = wallet->descriptor.network;
    test->recovery_word_count = count_phrase_words(phrase);
}

#define CHECK(condition)                                                       \
    do {                                                                       \
        if (!(condition)) {                                                    \
            fprintf(stderr, "%s:%d: check failed: %s\n", __FILE__, __LINE__, #condition); \
            return 1;                                                          \
        }                                                                      \
    } while (0)

int main(void) {
    TestContext context = {.valid = true};
    const WalletEnginePlatformHostCallbacks callbacks = {
        .struct_size = sizeof(WalletEnginePlatformHostCallbacks),
        .context = &context,
        .retain = retain_context,
        .release = release_context,
        .store_protected_secret = store_protected_secret,
    };
    WalletEngineLifecycle *lifecycle = NULL;
    CHECK(
        wallet_engine_lifecycle_new(&callbacks, &lifecycle) ==
        WALLET_ENGINE_ABI_STATUS_OK
    );
    CHECK(lifecycle != NULL);

    static const char record_id[] = "c-wallet-1";
    const WalletEngineCreateWalletRequest request = {
        .record_id = {record_id, sizeof(record_id) - 1},
        .network = WALLET_ENGINE_NETWORK_TESTNET,
    };
    CHECK(
        wallet_engine_lifecycle_create_wallet(
            lifecycle,
            &request,
            &context,
            create_wallet_result
        ) == WALLET_ENGINE_ABI_STATUS_OK
    );

    CHECK(context.valid);
    CHECK(context.retains == 1);
    CHECK(context.releases == 0);
    CHECK(context.stores == 1);
    CHECK(context.results == 1);
    CHECK(context.recovery_word_count == 24);
    CHECK(context.network == WALLET_ENGINE_NETWORK_TESTNET);
    CHECK(strcmp(context.record_id, record_id) == 0);
    CHECK(context.address[0] != '\0');
    CHECK(strcmp(context.stored_secret_ref, "wallet:c-wallet-1:mnemonic") == 0);
    CHECK(strcmp(context.result_secret_ref, context.stored_secret_ref) == 0);

    wallet_engine_lifecycle_free(lifecycle);
    CHECK(context.releases == 1);
    puts("Wallet Engine synchronous C create-wallet test passed");
    return 0;
}
