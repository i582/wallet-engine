#include "wallet_engine_internal.hpp"

// This file contains the reusable C-to-C++ host ownership bridge.

#include <atomic>
#include <cstdint>
#include <exception>
#include <memory>
#include <optional>
#include <stdexcept>
#include <string>
#include <string_view>
#include <utility>
#include <vector>

namespace wallet_engine::detail {
namespace {

[[nodiscard]] WalletEngineStringView string_view(std::string_view value) noexcept {
    return WalletEngineStringView{
        value.empty() ? nullptr : value.data(),
        value.size(),
    };
}

[[nodiscard]] std::string copy_string(WalletEngineStringView value) {
    if (value.data == nullptr) {
        if (value.len == 0) {
            return {};
        }
        throw std::invalid_argument("string view has null data");
    }
    return std::string(value.data, value.len);
}

[[nodiscard]] std::vector<std::uint8_t> copy_bytes(WalletEngineBytesView value) {
    if (value.data == nullptr) {
        if (value.len == 0) {
            return {};
        }
        throw std::invalid_argument("byte view has null data");
    }
    return std::vector<std::uint8_t>(value.data, value.data + value.len);
}

[[nodiscard]] WalletEngineProtectedSecretHostErrorKind error_kind_to_abi(
    ProtectedSecretErrorKind kind
) noexcept {
    switch (kind) {
    case ProtectedSecretErrorKind::not_found:
        return WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_NOT_FOUND;
    case ProtectedSecretErrorKind::authentication_failed:
        return WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_AUTHENTICATION_FAILED;
    case ProtectedSecretErrorKind::cancelled:
        return WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_CANCELLED;
    case ProtectedSecretErrorKind::unavailable:
        return WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_UNAVAILABLE;
    case ProtectedSecretErrorKind::policy_violation:
        return WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_POLICY_VIOLATION;
    case ProtectedSecretErrorKind::other:
        return WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_OTHER;
    }
    return WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_OTHER;
}

void complete_store_success(WalletEngineCompletionId completion_id) noexcept {
    static_cast<void>(
        wallet_engine_store_protected_secret_complete(completion_id, nullptr)
    );
}

void complete_store_failure(
    WalletEngineCompletionId completion_id,
    ProtectedSecretErrorKind kind,
    std::string_view diagnostic
) noexcept {
    const WalletEngineProtectedSecretHostErrorView error{
        error_kind_to_abi(kind),
        string_view(diagnostic),
    };
    static_cast<void>(
        wallet_engine_store_protected_secret_complete(completion_id, &error)
    );
}

class StoreCompletion final {
public:
    explicit StoreCompletion(WalletEngineCompletionId completion_id) noexcept
        : completion_id_(completion_id) {}

    void complete(std::optional<ProtectedSecretStoreError> error) noexcept {
        if (completed_.exchange(true, std::memory_order_acq_rel)) {
            return;
        }

        if (!error.has_value()) {
            complete_store_success(completion_id_);
            return;
        }

        complete_store_failure(
            completion_id_,
            error->kind,
            error->diagnostic
        );
    }

    void complete_failure(
        ProtectedSecretErrorKind kind,
        std::string_view diagnostic
    ) noexcept {
        if (completed_.exchange(true, std::memory_order_acq_rel)) {
            return;
        }
        complete_store_failure(completion_id_, kind, diagnostic);
    }

private:
    WalletEngineCompletionId completion_id_;
    std::atomic_bool completed_{false};
};

} // namespace

HostState::HostState(std::shared_ptr<ProtectedSecretStore> protected_store)
    : protected_store_(std::move(protected_store)) {
    if (!protected_store_) {
        throw std::invalid_argument("protected store must not be null");
    }
}

WalletEnginePlatformHostCallbacks HostState::callbacks() noexcept {
    return WalletEnginePlatformHostCallbacks{
        sizeof(WalletEnginePlatformHostCallbacks),
        this,
        &HostState::retain_callback,
        &HostState::release_callback,
        &HostState::store_protected_secret_callback,
    };
}

void HostState::retain() noexcept {
    references_.fetch_add(1, std::memory_order_relaxed);
}

void HostState::release() noexcept {
    if (references_.fetch_sub(1, std::memory_order_acq_rel) == 1) {
        delete this;
    }
}

void HostState::store_protected_secret(
    WalletEngineCompletionId completion_id,
    const WalletEngineProtectedSecretStoreView* request
) noexcept {
    if (request == nullptr) {
        complete_store_failure(
            completion_id,
            ProtectedSecretErrorKind::other,
            "protected-secret request is null"
        );
        return;
    }

    std::shared_ptr<StoreCompletion> completion;
    try {
        ProtectedSecretStoreRequest owned_request{
            copy_string(request->secret_ref.value),
            copy_bytes(request->bytes),
            request->require_user_presence,
        };
        completion = std::make_shared<StoreCompletion>(completion_id);
        auto protected_store = protected_store_;

        protected_store->store(
            std::move(owned_request),
            [completion, protected_store](
                std::optional<ProtectedSecretStoreError> error
            ) noexcept {
                static_cast<void>(protected_store);
                completion->complete(std::move(error));
            }
        );
    } catch (const std::exception& error) {
        if (completion) {
            completion->complete_failure(
                ProtectedSecretErrorKind::other,
                error.what()
            );
        } else {
            complete_store_failure(
                completion_id,
                ProtectedSecretErrorKind::other,
                error.what()
            );
        }
    } catch (...) {
        constexpr std::string_view diagnostic =
            "protected store threw an unknown exception";
        if (completion) {
            completion->complete_failure(
                ProtectedSecretErrorKind::other,
                diagnostic
            );
        } else {
            complete_store_failure(
                completion_id,
                ProtectedSecretErrorKind::other,
                diagnostic
            );
        }
    }
}

void HostState::retain_callback(void* context) noexcept {
    static_cast<HostState*>(context)->retain();
}

void HostState::release_callback(void* context) noexcept {
    static_cast<HostState*>(context)->release();
}

void HostState::store_protected_secret_callback(
    void* context,
    WalletEngineCompletionId completion_id,
    const WalletEngineProtectedSecretStoreView* request
) noexcept {
    static_cast<HostState*>(context)->store_protected_secret(
        completion_id,
        request
    );
}

void HostStateReleaser::operator()(HostState* state) const noexcept {
    if (state != nullptr) {
        state->release();
    }
}

HostStateOwner make_host_state(
    std::shared_ptr<ProtectedSecretStore> protected_store
) {
    return HostStateOwner(new HostState(std::move(protected_store)));
}

} // namespace wallet_engine::detail
