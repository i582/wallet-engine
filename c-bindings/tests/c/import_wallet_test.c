#include "wallet_engine.h"

#include <stdbool.h>
#include <stddef.h>
#include <stdio.h>
#include <string.h>

#define TEXT_CAPACITY 256
#define RECOVERY_WORD_COUNT 24

typedef struct TestContext {
    size_t retains;
    size_t releases;
    size_t stores;
    size_t results;
    bool valid;
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

static void retain_context(void *context) {
    ++((TestContext *)context)->retains;
}

static void release_context(void *context) {
    ++((TestContext *)context)->releases;
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

static void import_wallet_result(
    void *context,
    WalletEngineAbiStatus abi_status,
    const WalletEngineWalletDescriptorView *descriptor,
    const WalletEngineWalletLifecycleErrorView *error
) {
    TestContext *test = context;
    ++test->results;
    if (abi_status != WALLET_ENGINE_ABI_STATUS_OK || descriptor == NULL || error != NULL ||
        !copy_string(test->record_id, descriptor->record_id) ||
        !copy_string(test->address, descriptor->address) ||
        !copy_string(test->result_secret_ref, descriptor->secret_ref.value) ||
        descriptor->public_key.data == NULL || descriptor->public_key.len != 32) {
        test->valid = false;
        return;
    }
    test->network = descriptor->network;
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

    static const char record_id[] = "c-import-1";
    static const char *recovery_words[RECOVERY_WORD_COUNT] = {
        "section", "garden", "tomato", "dinner", "season", "dice",
        "renew", "length", "useful", "spin", "trade", "intact",
        "use", "universe", "what", "post", "spike", "keen",
        "mandate", "behind", "concert", "egg", "doll", "rug",
    };
    WalletEngineStringView word_views[RECOVERY_WORD_COUNT];
    for (size_t index = 0; index < RECOVERY_WORD_COUNT; ++index) {
        word_views[index].data = recovery_words[index];
        word_views[index].len = strlen(recovery_words[index]);
    }
    const WalletEngineImportWalletRequest request = {
        .record_id = {record_id, sizeof(record_id) - 1},
        .network = WALLET_ENGINE_NETWORK_TESTNET,
        .recovery_words = {word_views, RECOVERY_WORD_COUNT},
    };
    CHECK(
        wallet_engine_lifecycle_import_wallet(
            lifecycle,
            &request,
            &context,
            import_wallet_result
        ) == WALLET_ENGINE_ABI_STATUS_OK
    );

    CHECK(context.valid);
    CHECK(context.stores == 1);
    CHECK(context.results == 1);
    CHECK(context.network == WALLET_ENGINE_NETWORK_TESTNET);
    CHECK(strcmp(context.record_id, record_id) == 0);
    CHECK(
        strcmp(context.address, "0QA_6fh0aRAkD7n1MNfAUx8TvyCUw2iTQfzVM-0isMze2anN") == 0
    );
    CHECK(strcmp(context.stored_secret_ref, "wallet:c-import-1:mnemonic") == 0);
    CHECK(strcmp(context.result_secret_ref, context.stored_secret_ref) == 0);

    wallet_engine_lifecycle_free(lifecycle);
    CHECK(context.retains == 1);
    CHECK(context.releases == 1);
    puts("Wallet Engine synchronous C import-wallet test passed");
    return 0;
}
