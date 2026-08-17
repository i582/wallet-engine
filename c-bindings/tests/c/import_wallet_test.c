#include "wallet_engine.h"

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#define TEXT_CAPACITY 256
#define RECOVERY_WORD_COUNT 24

typedef struct TestContext {
    size_t retains;
    size_t releases;
    size_t stores;
    size_t results;
    WalletEngineProtectedSecretStoreCompletion *pending_completion;
    bool valid;
    bool done;
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
    WalletEngineProtectedSecretStoreCompletion *completion,
    const WalletEngineProtectedSecretStoreView *request
) {
    TestContext *test = context;
    ++test->stores;

    if (request == NULL || request->bytes.data == NULL || request->bytes.len == 0 ||
        !request->require_user_presence ||
        !copy_string(test->stored_secret_ref, request->secret_ref.value)) {
        test->valid = false;
    }

    test->pending_completion = completion;
}

static void import_wallet_complete(
    void *context,
    WalletEngineAbiStatus abi_status,
    const WalletEngineWalletDescriptorView *descriptor,
    const WalletEngineWalletLifecycleErrorView *error
) {
    TestContext *test = context;
    ++test->results;
    if (abi_status != WALLET_ENGINE_ABI_STATUS_OK || descriptor == NULL ||
        error != NULL) {
        test->valid = false;
        test->done = true;
        return;
    }

    if (!copy_string(test->record_id, descriptor->record_id) ||
        !copy_string(test->address, descriptor->address) ||
        !copy_string(test->result_secret_ref, descriptor->secret_ref.value) ||
        descriptor->public_key.data == NULL || descriptor->public_key.len != 32) {
        test->valid = false;
    }
    test->network = descriptor->network;
    test->done = true;
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
        .pending_completion = NULL,
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
    CHECK(context.retains == 1);

    char record_id[] = "c-import-1";
    char recovery_words[RECOVERY_WORD_COUNT][16] = {
        "section", "garden", "tomato", "dinner", "season", "dice",
        "renew", "length", "useful", "spin", "trade", "intact",
        "use", "universe", "what", "post", "spike", "keen",
        "mandate", "behind", "concert", "egg", "doll", "rug",
    };
    WalletEngineStringView recovery_word_views[RECOVERY_WORD_COUNT];
    for (size_t index = 0; index < RECOVERY_WORD_COUNT; ++index) {
        recovery_word_views[index].data = recovery_words[index];
        recovery_word_views[index].len = strlen(recovery_words[index]);
    }
    const WalletEngineImportWalletRequest request = {
        .record_id = {record_id, sizeof(record_id) - 1},
        .network = WALLET_ENGINE_NETWORK_TESTNET,
        .recovery_words = {recovery_word_views, RECOVERY_WORD_COUNT},
    };
    WalletEngineImportWalletOperation *operation = NULL;
    CHECK(
        wallet_engine_lifecycle_import_wallet_start(lifecycle, &request, &operation) ==
        WALLET_ENGINE_ABI_STATUS_OK
    );
    CHECK(operation != NULL);
    CHECK(context.stores == 0);
    CHECK(context.results == 0);

    memset(record_id, 'x', sizeof(record_id) - 1);
    memset(recovery_words, 0, sizeof(recovery_words));
    wallet_engine_lifecycle_free(lifecycle);

    WalletEngineOperationPollState poll_state = WALLET_ENGINE_OPERATION_POLL_STATE_READY;
    CHECK(
        wallet_engine_import_wallet_operation_poll(
            operation,
            &context,
            import_wallet_complete,
            &poll_state
        ) == WALLET_ENGINE_ABI_STATUS_OK
    );
    CHECK(poll_state == WALLET_ENGINE_OPERATION_POLL_STATE_PENDING);
    CHECK(context.pending_completion != NULL);
    CHECK(context.stores == 1);
    CHECK(context.results == 0);

    CHECK(
        wallet_engine_protected_secret_store_completion_complete(
            context.pending_completion,
            NULL
        ) == WALLET_ENGINE_ABI_STATUS_OK
    );
    wallet_engine_protected_secret_store_completion_free(context.pending_completion);
    context.pending_completion = NULL;
    CHECK(context.results == 0);

    CHECK(
        wallet_engine_import_wallet_operation_poll(
            operation,
            &context,
            import_wallet_complete,
            &poll_state
        ) == WALLET_ENGINE_ABI_STATUS_OK
    );
    CHECK(poll_state == WALLET_ENGINE_OPERATION_POLL_STATE_READY);
    wallet_engine_import_wallet_operation_free(operation);

    CHECK(context.done);
    CHECK(context.valid);
    CHECK(context.releases == 1);
    CHECK(context.stores == 1);
    CHECK(context.results == 1);
    CHECK(context.network == WALLET_ENGINE_NETWORK_TESTNET);
    CHECK(strcmp(context.record_id, "c-import-1") == 0);
    CHECK(
        strcmp(
            context.address,
            "0QA_6fh0aRAkD7n1MNfAUx8TvyCUw2iTQfzVM-0isMze2anN"
        ) == 0
    );
    CHECK(strcmp(context.stored_secret_ref, "wallet:c-import-1:mnemonic") == 0);
    CHECK(strcmp(context.result_secret_ref, context.stored_secret_ref) == 0);

    puts("Wallet Engine C import-wallet test passed");
    return 0;
}
