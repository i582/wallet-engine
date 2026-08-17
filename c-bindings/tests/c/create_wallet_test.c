#include "wallet_engine.h"

#include <stdatomic.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#define TEXT_CAPACITY 256

typedef struct TestContext {
    atomic_size_t retains;
    atomic_size_t releases;
    atomic_size_t stores;
    atomic_size_t completions;
    atomic_bool done;
    WalletEngineCompletionId pending_completion_id;
    bool valid;
    size_t recovery_word_count;
    WalletEngineNetwork network;
    char record_id[TEXT_CAPACITY];
    char address[TEXT_CAPACITY];
    char stored_secret_ref[TEXT_CAPACITY];
    char result_secret_ref[TEXT_CAPACITY];
} TestContext;

static bool copy_string(
    char destination[TEXT_CAPACITY],
    WalletEngineStringView source
) {
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
    atomic_fetch_add_explicit(&test->retains, 1, memory_order_relaxed);
}

static void release_context(void *context) {
    TestContext *test = context;
    atomic_fetch_add_explicit(&test->releases, 1, memory_order_release);
}

static void store_protected_secret(
    void *context,
    WalletEngineCompletionId completion_id,
    const WalletEngineProtectedSecretStoreView *request
) {
    TestContext *test = context;
    atomic_fetch_add_explicit(&test->stores, 1, memory_order_relaxed);

    if (request == NULL || request->bytes.data == NULL || request->bytes.len == 0 ||
        !request->require_user_presence ||
        !copy_string(test->stored_secret_ref, request->secret_ref.value)) {
        test->valid = false;
    }

    test->pending_completion_id = completion_id;
}

static void create_wallet_complete(
    void *context,
    WalletEngineAbiStatus abi_status,
    const WalletEngineCreatedWalletView *wallet,
    const WalletEngineWalletLifecycleErrorView *error
) {
    TestContext *test = context;
    atomic_fetch_add_explicit(&test->completions, 1, memory_order_relaxed);
    if (abi_status != WALLET_ENGINE_ABI_STATUS_OK || wallet == NULL || error != NULL) {
        test->valid = false;
        atomic_store_explicit(&test->done, true, memory_order_release);
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
    atomic_store_explicit(&test->done, true, memory_order_release);
}

#define CHECK(condition)                                                       \
    do {                                                                       \
        if (!(condition)) {                                                    \
            fprintf(                                                           \
                stderr,                                                        \
                "%s:%d: check failed: %s\n",                                 \
                __FILE__,                                                      \
                __LINE__,                                                      \
                #condition                                                     \
            );                                                                 \
            return 1;                                                          \
        }                                                                      \
    } while (0)

int main(void) {
    TestContext context = {
        .retains = 0,
        .releases = 0,
        .stores = 0,
        .completions = 0,
        .done = false,
        .pending_completion_id = 0,
        .valid = true,
    };
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
    CHECK(atomic_load_explicit(&context.retains, memory_order_acquire) == 1);

    static const char record_id[] = "c-wallet-1";
    const WalletEngineCreateWalletRequest request = {
        .record_id = {record_id, sizeof(record_id) - 1},
        .network = WALLET_ENGINE_NETWORK_TESTNET,
    };
    WalletEngineCreateWalletOperation *operation = NULL;
    CHECK(
        wallet_engine_lifecycle_create_wallet_start(lifecycle, &request, &operation) ==
        WALLET_ENGINE_ABI_STATUS_OK
    );
    CHECK(operation != NULL);
    CHECK(atomic_load_explicit(&context.stores, memory_order_acquire) == 0);
    CHECK(atomic_load_explicit(&context.completions, memory_order_acquire) == 0);

    wallet_engine_lifecycle_free(lifecycle);

    WalletEngineOperationPollState poll_state = WALLET_ENGINE_OPERATION_POLL_STATE_READY;
    CHECK(
        wallet_engine_create_wallet_operation_poll(
            operation,
            &context,
            create_wallet_complete,
            &poll_state
        ) == WALLET_ENGINE_ABI_STATUS_OK
    );
    CHECK(poll_state == WALLET_ENGINE_OPERATION_POLL_STATE_PENDING);
    CHECK(context.pending_completion_id != 0);
    CHECK(atomic_load_explicit(&context.stores, memory_order_acquire) == 1);
    CHECK(atomic_load_explicit(&context.completions, memory_order_acquire) == 0);

    CHECK(
        wallet_engine_store_protected_secret_complete(
            context.pending_completion_id,
            NULL
        ) == WALLET_ENGINE_ABI_STATUS_OK
    );
    CHECK(atomic_load_explicit(&context.completions, memory_order_acquire) == 0);

    CHECK(
        wallet_engine_create_wallet_operation_poll(
            operation,
            &context,
            create_wallet_complete,
            &poll_state
        ) == WALLET_ENGINE_ABI_STATUS_OK
    );
    CHECK(poll_state == WALLET_ENGINE_OPERATION_POLL_STATE_READY);
    wallet_engine_create_wallet_operation_free(operation);

    CHECK(atomic_load_explicit(&context.done, memory_order_acquire));
    CHECK(atomic_load_explicit(&context.releases, memory_order_acquire) == 1);
    CHECK(context.valid);
    CHECK(atomic_load_explicit(&context.stores, memory_order_acquire) == 1);
    CHECK(atomic_load_explicit(&context.completions, memory_order_acquire) == 1);
    CHECK(context.recovery_word_count == 24);
    CHECK(context.network == WALLET_ENGINE_NETWORK_TESTNET);
    CHECK(strcmp(context.record_id, record_id) == 0);
    CHECK(context.address[0] != '\0');
    CHECK(strcmp(context.stored_secret_ref, "wallet:c-wallet-1:mnemonic") == 0);
    CHECK(strcmp(context.result_secret_ref, context.stored_secret_ref) == 0);

    puts("Wallet Engine C create-wallet test passed");
    return 0;
}
