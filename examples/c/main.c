#include "wallet_engine.h"

#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

_Static_assert(sizeof(WalletEngineAbiStatus) == sizeof(uint32_t), "ABI status must be 32-bit");
_Static_assert(sizeof(WalletEngineNetwork) == sizeof(uint32_t), "network must be 32-bit");

static int verify_basic_types(void) {
    static const char text[] = "wallet-engine";
    static const uint8_t bytes[] = {0x57, 0x45};

    const WalletEngineStringView string_view = {text, sizeof(text) - 1};
    const WalletEngineBytesView bytes_view = {bytes, sizeof(bytes)};

    if (string_view.len != strlen(text) || bytes_view.len != 2) {
        return 0;
    }

    return WALLET_ENGINE_ABI_STATUS_OK == 0 &&
           WALLET_ENGINE_ABI_STATUS_INVALID_ARGUMENT == 1 &&
           WALLET_ENGINE_ABI_STATUS_INVALID_UTF8 == 2 &&
           WALLET_ENGINE_ABI_STATUS_PANIC == 3 &&
           WALLET_ENGINE_NETWORK_MAINNET == 0 &&
           WALLET_ENGINE_NETWORK_TESTNET == 1;
}

int main(void) {
    const uint32_t linked_version = wallet_engine_abi_version();
    if (linked_version != WALLET_ENGINE_ABI_VERSION) {
        fprintf(
            stderr,
            "C ABI version mismatch: header=%u, library=%u\n",
            (unsigned)WALLET_ENGINE_ABI_VERSION,
            (unsigned)linked_version
        );
        return 1;
    }

    if (!verify_basic_types()) {
        fputs("C ABI basic type check failed\n", stderr);
        return 1;
    }

    printf("Wallet Engine C ABI version: %u\n", (unsigned)linked_version);
    return 0;
}
