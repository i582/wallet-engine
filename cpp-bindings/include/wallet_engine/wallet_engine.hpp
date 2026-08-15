#pragma once

// Idiomatic C++ ownership layer over the stable Wallet Engine C ABI.

#include <cstdint>
#include <functional>
#include <future>
#include <memory>
#include <optional>
#include <stdexcept>
#include <string>
#include <variant>
#include <vector>

namespace wallet_engine {

enum class Network : std::uint32_t {
    mainnet,
    testnet,
};

enum class ProtectedSecretErrorKind : std::uint32_t {
    not_found,
    authentication_failed,
    cancelled,
    unavailable,
    policy_violation,
    other,
};

struct ProtectedSecretStoreError {
    ProtectedSecretErrorKind kind;
    std::string diagnostic;
};

struct ProtectedSecretStoreRequest {
    std::string secret_ref;
    std::vector<std::uint8_t> bytes;
    bool require_user_presence;
};

using ProtectedSecretStoreCompletion =
    std::function<void(std::optional<ProtectedSecretStoreError>)>;

class ProtectedSecretStore {
public:
    virtual ~ProtectedSecretStore() = default;

    // Implementations may finish synchronously or asynchronously, but must
    // invoke completion exactly once. The request owns all of its data.
    virtual void store(
        ProtectedSecretStoreRequest request,
        ProtectedSecretStoreCompletion completion
    ) = 0;
};

struct CreateWalletRequest {
    std::string record_id;
    Network network;
};

struct WalletDescriptor {
    std::string record_id;
    std::string address;
    Network network;
    std::string secret_ref;
};

struct CreatedWallet {
    WalletDescriptor descriptor;
    std::vector<std::string> recovery_words;
};

enum class LifecycleErrorCode : std::uint32_t {
    invalid_record_id,
    invalid_recovery_phrase,
    address_derivation_failed,
    secret_wallet_mismatch,
    protected_secret_host,
};

struct LifecycleError {
    LifecycleErrorCode code;
    std::optional<ProtectedSecretErrorKind> protected_secret_error_kind;
    std::string diagnostic;
};

using CreateWalletResult = std::variant<CreatedWallet, LifecycleError>;

class AbiError final : public std::runtime_error {
public:
    AbiError(std::uint32_t status, std::string message);

    [[nodiscard]] std::uint32_t status() const noexcept;

private:
    std::uint32_t status_;
};

class Lifecycle final {
public:
    explicit Lifecycle(std::shared_ptr<ProtectedSecretStore> protected_store);
    ~Lifecycle();

    Lifecycle(const Lifecycle&) = delete;
    Lifecycle& operator=(const Lifecycle&) = delete;

    Lifecycle(Lifecycle&& other) noexcept;
    Lifecycle& operator=(Lifecycle&& other) noexcept;

    // The request is copied into the operation. A valid Lifecycle may be
    // destroyed immediately after this function successfully returns.
    [[nodiscard]] std::future<CreateWalletResult> create_wallet(
        CreateWalletRequest request
    ) const;

private:
    struct Impl;
    std::unique_ptr<Impl> impl_;
};

} // namespace wallet_engine
