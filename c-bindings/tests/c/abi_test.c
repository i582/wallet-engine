#include "wallet_engine.h"

#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

_Static_assert(sizeof(WalletEngineAbiStatus) == sizeof(uint32_t), "ABI status must be 32-bit");
_Static_assert(sizeof(WalletEngineNetwork) == sizeof(uint32_t), "network must be 32-bit");
_Static_assert(
    sizeof(WalletEngineProtectedSecretHostErrorKind) == sizeof(uint32_t),
    "protected-secret host error kind must be 32-bit"
);
_Static_assert(
    sizeof(WalletEngineWalletLifecycleErrorCode) == sizeof(uint32_t),
    "wallet lifecycle error code must be 32-bit"
);
_Static_assert(
    offsetof(WalletEngineStringView, data) == 0,
    "string-view data must be the first field"
);
_Static_assert(
    offsetof(WalletEngineBytesView, data) == 0,
    "bytes-view data must be the first field"
);

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
            return 0;                                                          \
        }                                                                      \
    } while (0)

static int test_abi_version(void) {
    CHECK(wallet_engine_abi_version() == WALLET_ENGINE_ABI_VERSION);
    return 1;
}

static int test_status_values(void) {
    CHECK(WALLET_ENGINE_ABI_STATUS_OK == 0);
    CHECK(WALLET_ENGINE_ABI_STATUS_INVALID_ARGUMENT == 1);
    CHECK(WALLET_ENGINE_ABI_STATUS_INVALID_UTF8 == 2);
    CHECK(WALLET_ENGINE_ABI_STATUS_PANIC == 3);
    CHECK(WALLET_ENGINE_ABI_STATUS_OPERATION_BUSY == 4);
    CHECK(WALLET_ENGINE_OPERATION_POLL_STATE_PENDING == 0);
    CHECK(WALLET_ENGINE_OPERATION_POLL_STATE_READY == 1);
    return 1;
}

static int test_network_values(void) {
    const WalletEngineNetwork mainnet = WALLET_ENGINE_NETWORK_MAINNET;
    const WalletEngineNetwork testnet = WALLET_ENGINE_NETWORK_TESTNET;

    CHECK(mainnet == 0);
    CHECK(testnet == 1);
    CHECK(mainnet != testnet);
    return 1;
}

static int test_protected_secret_host_error_values(void) {
    CHECK(WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_NOT_FOUND == 0);
    CHECK(WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_AUTHENTICATION_FAILED == 1);
    CHECK(WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_CANCELLED == 2);
    CHECK(WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_UNAVAILABLE == 3);
    CHECK(WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_POLICY_VIOLATION == 4);
    CHECK(WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_OTHER == 5);
    return 1;
}

static int test_wallet_lifecycle_error_values(void) {
    CHECK(WALLET_ENGINE_WALLET_LIFECYCLE_ERROR_CODE_INVALID_RECORD_ID == 0);
    CHECK(WALLET_ENGINE_WALLET_LIFECYCLE_ERROR_CODE_INVALID_RECOVERY_PHRASE == 1);
    CHECK(WALLET_ENGINE_WALLET_LIFECYCLE_ERROR_CODE_ADDRESS_DERIVATION_FAILED == 2);
    CHECK(WALLET_ENGINE_WALLET_LIFECYCLE_ERROR_CODE_SECRET_WALLET_MISMATCH == 3);
    CHECK(WALLET_ENGINE_WALLET_LIFECYCLE_ERROR_CODE_PROTECTED_SECRET_HOST == 4);
    return 1;
}

static int test_borrowed_views(void) {
    static const char text[] = "wallet-engine";
    static const uint8_t bytes[] = {0x57, 0x45};

    const WalletEngineStringView string_view = {text, sizeof(text) - 1};
    const WalletEngineBytesView bytes_view = {bytes, sizeof(bytes)};
    const WalletEngineStringView empty_string = {NULL, 0};
    const WalletEngineBytesView empty_bytes = {NULL, 0};

    CHECK(string_view.data == text);
    CHECK(string_view.len == strlen(text));
    CHECK(bytes_view.data == bytes);
    CHECK(bytes_view.len == 2);
    CHECK(empty_string.data == NULL);
    CHECK(empty_string.len == 0);
    CHECK(empty_bytes.data == NULL);
    CHECK(empty_bytes.len == 0);
    return 1;
}

static int test_create_wallet_types(void) {
    static const char record_id[] = "wallet-1";
    static const char address[] = "UQExampleAddress";
    static const char secret_ref[] = "wallet:wallet-1:mnemonic";
    static const uint8_t secret_bytes[] = {0x73, 0x65, 0x63, 0x72, 0x65, 0x74};
    static const char phrase[] = "first second";

    const WalletEngineStringView record_id_view = {record_id, sizeof(record_id) - 1};
    const WalletEngineProtectedSecretRefView secret_ref_view = {
        .value = {secret_ref, sizeof(secret_ref) - 1},
    };
    const WalletEngineCreateWalletRequest request = {
        .record_id = record_id_view,
        .network = WALLET_ENGINE_NETWORK_TESTNET,
    };
    const WalletEngineProtectedSecretStoreView store = {
        .secret_ref = secret_ref_view,
        .bytes = {secret_bytes, sizeof(secret_bytes)},
        .require_user_presence = true,
    };
    const WalletEngineWalletDescriptorView descriptor = {
        .record_id = record_id_view,
        .address = {address, sizeof(address) - 1},
        .network = WALLET_ENGINE_NETWORK_TESTNET,
        .secret_ref = secret_ref_view,
    };
    const WalletEngineCreatedWalletView created = {
        .descriptor = descriptor,
        .recovery_phrase = {
            .phrase = {phrase, sizeof(phrase) - 1},
        },
    };
    const WalletEngineWalletLifecycleErrorView error = {
        .code = WALLET_ENGINE_WALLET_LIFECYCLE_ERROR_CODE_PROTECTED_SECRET_HOST,
        .has_protected_secret_host_error_kind = true,
        .protected_secret_host_error_kind =
            WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_UNAVAILABLE,
        .diagnostic = {NULL, 0},
    };

    CHECK(request.record_id.data == record_id);
    CHECK(request.network == WALLET_ENGINE_NETWORK_TESTNET);
    CHECK(store.secret_ref.value.data == secret_ref);
    CHECK(store.bytes.len == sizeof(secret_bytes));
    CHECK(store.require_user_presence);
    CHECK(created.descriptor.address.data == address);
    CHECK(created.recovery_phrase.phrase.data == phrase);
    CHECK(created.recovery_phrase.phrase.len == sizeof(phrase) - 1);
    CHECK(error.has_protected_secret_host_error_kind);
    CHECK(
        error.protected_secret_host_error_kind ==
        WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_UNAVAILABLE
    );
    return 1;
}

typedef struct HostContext {
    size_t retains;
    size_t releases;
} HostContext;

static void retain_context(void *context) {
    HostContext *host = context;
    host->retains += 1;
}

static void release_context(void *context) {
    HostContext *host = context;
    host->releases += 1;
}

static void store_protected_secret(
    void *context,
    WalletEngineCompletionId completion_id,
    const WalletEngineProtectedSecretStoreView *request
) {
    (void)context;
    (void)completion_id;
    (void)request;
}

static int test_platform_host_contract(void) {
    HostContext context = {0};
    const WalletEnginePlatformHostCallbacks callbacks = {
        .struct_size = sizeof(WalletEnginePlatformHostCallbacks),
        .context = &context,
        .retain = retain_context,
        .release = release_context,
        .store_protected_secret = store_protected_secret,
    };

    CHECK(callbacks.struct_size == sizeof(callbacks));
    CHECK(callbacks.context == &context);
    CHECK(callbacks.retain != NULL);
    CHECK(callbacks.release != NULL);
    CHECK(callbacks.store_protected_secret != NULL);
    CHECK(
        wallet_engine_store_protected_secret_complete(0, NULL) ==
        WALLET_ENGINE_ABI_STATUS_INVALID_ARGUMENT
    );
    CHECK(
        wallet_engine_store_protected_secret_complete(UINT64_MAX, NULL) ==
        WALLET_ENGINE_ABI_STATUS_INVALID_ARGUMENT
    );
    return 1;
}

static int test_lifecycle_handle(void) {
    HostContext context = {0};
    const WalletEnginePlatformHostCallbacks callbacks = {
        .struct_size = sizeof(WalletEnginePlatformHostCallbacks),
        .context = &context,
        .retain = retain_context,
        .release = release_context,
        .store_protected_secret = store_protected_secret,
    };
    WalletEngineLifecycle *lifecycle = (WalletEngineLifecycle *)(uintptr_t)1;

    CHECK(
        wallet_engine_lifecycle_new(NULL, &lifecycle) ==
        WALLET_ENGINE_ABI_STATUS_INVALID_ARGUMENT
    );
    CHECK(lifecycle == NULL);
    CHECK(context.retains == 0);
    CHECK(
        wallet_engine_lifecycle_new(&callbacks, NULL) ==
        WALLET_ENGINE_ABI_STATUS_INVALID_ARGUMENT
    );
    CHECK(context.retains == 0);

    CHECK(
        wallet_engine_lifecycle_new(&callbacks, &lifecycle) ==
        WALLET_ENGINE_ABI_STATUS_OK
    );
    CHECK(lifecycle != NULL);
    CHECK(context.retains == 1);
    CHECK(context.releases == 0);

    wallet_engine_lifecycle_free(lifecycle);
    CHECK(context.releases == 1);
    wallet_engine_lifecycle_free(NULL);
    CHECK(context.releases == 1);
    return 1;
}

int main(void) {
    if (!test_abi_version() || !test_status_values() || !test_network_values() ||
        !test_protected_secret_host_error_values() ||
        !test_wallet_lifecycle_error_values() || !test_borrowed_views() ||
        !test_create_wallet_types() || !test_platform_host_contract() ||
        !test_lifecycle_handle()) {
        return 1;
    }

    puts("Wallet Engine C ABI tests passed");
    return 0;
}
