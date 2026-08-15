#include "wallet_engine_internal.hpp"

// Focused tests for callback ownership and borrowed-view copying.

#include <cstddef>
#include <cstdint>
#include <iostream>
#include <memory>
#include <optional>
#include <stdexcept>
#include <string>
#include <utility>

namespace {

std::size_t completion_calls = 0;
WalletEngineCompletionId last_completion_id = 0;
std::optional<WalletEngineProtectedSecretHostErrorKind> last_error_kind;

void reset_completion_calls() {
    completion_calls = 0;
    last_completion_id = 0;
    last_error_kind.reset();
}

class RecordingStore final : public wallet_engine::ProtectedSecretStore {
public:
    void store(
        wallet_engine::ProtectedSecretStoreRequest request,
        wallet_engine::ProtectedSecretStoreCompletion completion
    ) override {
        request_ = std::move(request);
        completion_ = std::move(completion);
    }

    void complete_twice() {
        completion_(std::nullopt);
        completion_(std::nullopt);
        completion_ = {};
    }

    wallet_engine::ProtectedSecretStoreRequest request_;

private:
    wallet_engine::ProtectedSecretStoreCompletion completion_;
};

class CompleteThenThrowStore final
    : public wallet_engine::ProtectedSecretStore {
public:
    void store(
        wallet_engine::ProtectedSecretStoreRequest,
        wallet_engine::ProtectedSecretStoreCompletion completion
    ) override {
        completion(std::nullopt);
        throw std::runtime_error("failure after completion");
    }
};

[[nodiscard]] bool check(bool condition, const char* message) {
    if (!condition) {
        std::cerr << "check failed: " << message << '\n';
    }
    return condition;
}

[[nodiscard]] bool test_owned_request_and_reference_counting() {
    reset_completion_calls();

    auto store = std::make_shared<RecordingStore>();
    std::weak_ptr<RecordingStore> weak_store = store;
    auto state = wallet_engine::detail::make_host_state(store);
    const auto callbacks = state->callbacks();

    callbacks.retain(callbacks.context);
    state.reset();

    char secret_ref[] = "wallet:test";
    std::uint8_t secret_bytes[] = {1, 2, 3, 4};
    const WalletEngineProtectedSecretStoreView request{
        {{secret_ref, sizeof(secret_ref) - 1}},
        {secret_bytes, sizeof(secret_bytes)},
        true,
    };

    callbacks.store_protected_secret(callbacks.context, 41, &request);
    secret_ref[0] = 'X';
    secret_bytes[0] = 99;

    bool valid = true;
    valid &= check(
        store->request_.secret_ref == "wallet:test",
        "secret reference must be copied"
    );
    valid &= check(
        store->request_.bytes.size() == 4 && store->request_.bytes[0] == 1,
        "secret bytes must be copied"
    );
    valid &= check(
        store->request_.require_user_presence,
        "user-presence flag must be forwarded"
    );

    store->complete_twice();
    valid &= check(completion_calls == 1, "completion must be forwarded once");
    valid &= check(last_completion_id == 41, "completion ID must be forwarded");
    valid &= check(!last_error_kind.has_value(), "success must not carry an error");

    store.reset();
    valid &= check(!weak_store.expired(), "retained host state must own the store");
    callbacks.release(callbacks.context);
    valid &= check(weak_store.expired(), "release must destroy the last host owner");
    return valid;
}

[[nodiscard]] bool test_throw_after_completion_is_not_completed_twice() {
    reset_completion_calls();

    auto state = wallet_engine::detail::make_host_state(
        std::make_shared<CompleteThenThrowStore>()
    );
    const auto callbacks = state->callbacks();
    const WalletEngineProtectedSecretStoreView request{
        {{nullptr, 0}},
        {nullptr, 0},
        false,
    };

    callbacks.store_protected_secret(callbacks.context, 42, &request);
    return check(
        completion_calls == 1 && last_completion_id == 42,
        "an exception after completion must not complete twice"
    );
}

[[nodiscard]] bool test_invalid_request_becomes_host_error() {
    reset_completion_calls();

    auto state = wallet_engine::detail::make_host_state(
        std::make_shared<RecordingStore>()
    );
    const auto callbacks = state->callbacks();
    callbacks.store_protected_secret(callbacks.context, 43, nullptr);

    return check(
        completion_calls == 1 && last_completion_id == 43 &&
            last_error_kind ==
                WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_OTHER,
        "an invalid request must complete with a classified host error"
    );
}

} // namespace

extern "C" WalletEngineAbiStatus
wallet_engine_store_protected_secret_complete(
    WalletEngineCompletionId completion_id,
    const WalletEngineProtectedSecretHostErrorView* error
) {
    ++completion_calls;
    last_completion_id = completion_id;
    last_error_kind = error == nullptr
        ? std::nullopt
        : std::optional<WalletEngineProtectedSecretHostErrorKind>(error->kind);
    return WALLET_ENGINE_ABI_STATUS_OK;
}

int main() {
    const bool valid = test_owned_request_and_reference_counting() &&
        test_throw_after_completion_is_not_completed_twice() &&
        test_invalid_request_becomes_host_error();
    return valid ? 0 : 1;
}
