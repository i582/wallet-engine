#pragma once

#include "wallet_engine.h"
#include "wallet_engine/wallet_engine.hpp"

#include <atomic>
#include <cstddef>
#include <memory>

namespace wallet_engine::detail {

class HostState final {
public:
    explicit HostState(std::shared_ptr<ProtectedSecretStore> protected_store);

    HostState(const HostState&) = delete;
    HostState& operator=(const HostState&) = delete;

    [[nodiscard]] WalletEnginePlatformHostCallbacks callbacks() noexcept;
    void release() noexcept;

private:
    ~HostState() = default;

    void retain() noexcept;
    void store_protected_secret(
        WalletEngineCompletionId completion_id,
        const WalletEngineProtectedSecretStoreView* request
    ) noexcept;

    static void retain_callback(void* context) noexcept;
    static void release_callback(void* context) noexcept;
    static void store_protected_secret_callback(
        void* context,
        WalletEngineCompletionId completion_id,
        const WalletEngineProtectedSecretStoreView* request
    ) noexcept;

    std::atomic_size_t references_{1};
    std::shared_ptr<ProtectedSecretStore> protected_store_;
};

struct HostStateReleaser {
    void operator()(HostState* state) const noexcept;
};

using HostStateOwner = std::unique_ptr<HostState, HostStateReleaser>;

[[nodiscard]] HostStateOwner make_host_state(
    std::shared_ptr<ProtectedSecretStore> protected_store
);

} // namespace wallet_engine::detail
