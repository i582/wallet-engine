#include "wallet_engine.h"

#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

_Static_assert(sizeof(WalletEngineAbiStatus) == sizeof(uint32_t), "ABI status must be 32-bit");
_Static_assert(sizeof(WalletEngineNetwork) == sizeof(uint32_t), "network must be 32-bit");
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

int main(void) {
    if (!test_abi_version() || !test_status_values() || !test_network_values() ||
        !test_borrowed_views()) {
        return 1;
    }

    puts("Wallet Engine C ABI tests passed");
    return 0;
}
