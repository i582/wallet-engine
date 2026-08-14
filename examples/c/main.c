#include "wallet_engine.h"

#include <stdint.h>
#include <stdio.h>

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

    printf("Wallet Engine C ABI version: %u\n", (unsigned)linked_version);
    return 0;
}
