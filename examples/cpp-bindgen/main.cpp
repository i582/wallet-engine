#include "wallet_engine.hpp"

#include <cstdint>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <map>
#include <memory>
#include <mutex>
#include <optional>
#include <string>
#include <utility>
#include <vector>

namespace {

using namespace wallet_engine;

constexpr const char *wallet_metadata_file = "wallet_engine_wallets.tsv";
constexpr const char *wallet_secrets_file = "wallet_engine_secrets.tsv";

[[noreturn]] void throw_secret_error(
    ProtectedSecretHostErrorKind kind,
    const std::string &diagnostic
) {
    protected_secret_host_error::Failed error(diagnostic);
    error.kind = kind;
    error.diagnostic = diagnostic;
    throw error;
}

class FilePlatformHost final : public WalletPlatformHost {
public:
    std::vector<uint8_t> read_protected_secret(
        const ProtectedSecretRead &request
    ) override {
        std::lock_guard<std::mutex> lock(mutex_);
        const auto entry = secrets_.find(request.secret_ref.value);
        if (entry == secrets_.end()) {
            throw_secret_error(
                ProtectedSecretHostErrorKind::kNotFound,
                "secret is not available in this process"
            );
        }
        return entry->second;
    }

    void store_protected_secret(const ProtectedSecretStore &request) override {
        if (request.bytes.empty()) {
            throw_secret_error(
                ProtectedSecretHostErrorKind::kOther,
                "invalid file-storage request"
            );
        }

        std::ofstream file(
            wallet_secrets_file,
            std::ios::binary | std::ios::app
        );
        if (!file) {
            throw_secret_error(
                ProtectedSecretHostErrorKind::kUnavailable,
                "failed to open the secrets file"
            );
        }

        std::error_code permission_error;
        std::filesystem::permissions(
            wallet_secrets_file,
            std::filesystem::perms::owner_read |
                std::filesystem::perms::owner_write,
            std::filesystem::perm_options::replace,
            permission_error
        );
        if (permission_error) {
            throw_secret_error(
                ProtectedSecretHostErrorKind::kPolicyViolation,
                "failed to restrict permissions on the secrets file"
            );
        }

        file << request.secret_ref.value << '\t'
             << (request.require_user_presence ? "true" : "false") << '\t';
        file.write(
            reinterpret_cast<const char *>(request.bytes.data()),
            static_cast<std::streamsize>(request.bytes.size())
        );
        file.put('\n');
        file.close();
        if (!file) {
            throw_secret_error(
                ProtectedSecretHostErrorKind::kOther,
                "failed to append the mnemonic to the secrets file"
            );
        }

        std::lock_guard<std::mutex> lock(mutex_);
        secrets_[request.secret_ref.value] = request.bytes;
    }

    void delete_protected_secret(const ProtectedSecretRef &secret_ref) override {
        std::lock_guard<std::mutex> lock(mutex_);
        if (secrets_.erase(secret_ref.value) == 0) {
            throw_secret_error(
                ProtectedSecretHostErrorKind::kNotFound,
                "secret is not available in this process"
            );
        }
    }

    std::optional<JournalRecord> load_journal(const JournalKey &key) override {
        std::lock_guard<std::mutex> lock(mutex_);
        const auto entry = journal_.find({key.record_id, key.slot});
        if (entry == journal_.end()) {
            return std::nullopt;
        }
        return entry->second;
    }

    JournalCompareExchangeResult compare_exchange_journal(
        const JournalCompareExchange &mutation
    ) override {
        std::lock_guard<std::mutex> lock(mutex_);
        const auto key = std::make_pair(
            mutation.key.record_id,
            mutation.key.slot
        );
        const auto entry = journal_.find(key);
        const std::optional<JournalRecord> current =
            entry == journal_.end() ? std::nullopt :
                                      std::optional<JournalRecord>(entry->second);
        const bool version_matches = mutation.expected_version.has_value() ?
            current.has_value() &&
                current->version == mutation.expected_version.value() :
            !current.has_value();

        if (!version_matches) {
            return {false, current};
        }

        journal_[key] = mutation.replacement;
        return {true, mutation.replacement};
    }

private:
    std::mutex mutex_;
    std::map<std::string, std::vector<uint8_t>> secrets_;
    std::map<std::pair<std::string, std::string>, JournalRecord> journal_;
};

bool append_wallet_metadata(const CreatedWallet &wallet) {
    std::ofstream file(wallet_metadata_file, std::ios::app);
    if (!file) {
        return false;
    }

    file << wallet.descriptor.record_id << '\t'
         << (wallet.descriptor.network == Network::kMainnet ? "mainnet" :
                                                               "testnet")
         << '\t' << wallet.descriptor.address << '\t'
         << wallet.descriptor.secret_ref.value << '\n';
    return static_cast<bool>(file);
}

bool print_created_wallet(const CreatedWallet &wallet) {
    if (!append_wallet_metadata(wallet)) {
        std::cerr << "Failed to append wallet metadata\n";
        return false;
    }

    std::cout << "\nRecord ID: " << wallet.descriptor.record_id
              << "\nAddress: " << wallet.descriptor.address
              << "\nNetwork: "
              << (wallet.descriptor.network == Network::kMainnet ? "mainnet" :
                                                                    "testnet")
              << "\nSecret reference: "
              << wallet.descriptor.secret_ref.value
              << "\nRecovery phrase (display once and keep private):\n"
              << wallet.recovery_phrase.phrase << '\n';
    return !wallet.recovery_phrase.phrase.empty();
}

void print_lifecycle_error(const WalletLifecycleError &error) {
    std::cerr << "Wallet creation failed";
    if (dynamic_cast<const wallet_lifecycle_error::InvalidRecordId *>(&error)) {
        std::cerr << ": invalid record ID";
    } else if (
        dynamic_cast<const wallet_lifecycle_error::AddressDerivationFailed *>(
            &error
        )
    ) {
        std::cerr << ": address derivation failed";
    } else if (const auto *host_error =
                   dynamic_cast<const wallet_lifecycle_error::ProtectedSecretHost *>(
                       &error
                   )) {
        std::cerr << ": protected-storage error: "
                  << host_error->diagnostic;
    } else {
        std::cerr << ": lifecycle error";
    }
    std::cerr << '\n';
}

std::string read_line() {
    std::string value;
    std::getline(std::cin, value);
    return value;
}

Network prompt_network() {
    std::cout << "Network [1 = testnet, 2 = mainnet]: ";
    return read_line() == "2" ? Network::kMainnet : Network::kTestnet;
}

void create_wallet(WalletLifecycle &lifecycle) {
    std::cout << "Record ID: ";
    const auto record_id = read_line();
    if (record_id.empty()) {
        std::cerr << "Record ID is required\n";
        return;
    }

    try {
        const auto wallet = lifecycle.create_wallet({
            record_id,
            prompt_network(),
        });
        if (print_created_wallet(wallet)) {
            std::cout << "Saved metadata to " << wallet_metadata_file << '\n'
                      << "Saved plaintext mnemonic to " << wallet_secrets_file
                      << '\n';
        }
    } catch (const WalletLifecycleError &error) {
        print_lifecycle_error(error);
    } catch (const std::exception &error) {
        std::cerr << "Wallet creation failed at the FFI boundary: "
                  << error.what() << '\n';
    }
}

void list_wallets() {
    std::ifstream file(wallet_metadata_file);
    if (!file) {
        std::cout << "No saved wallets.\n";
        return;
    }

    std::cout << "\nrecord_id\tnetwork\taddress\tsecret_ref\n"
              << file.rdbuf() << '\n';
}

void run_menu(WalletLifecycle &lifecycle) {
    for (;;) {
        std::cout << "\nWallet Engine generated C++ example\n"
                  << "1. Create wallet\n"
                  << "2. List saved wallets\n"
                  << "3. Exit\n> ";
        const auto choice = read_line();
        if (!std::cin || choice == "3") {
            return;
        }
        if (choice == "1") {
            create_wallet(lifecycle);
        } else if (choice == "2") {
            list_wallets();
        } else {
            std::cout << "Unknown menu item.\n";
        }
    }
}

} // namespace

int main() {
    std::cout << "WARNING: this example stores recovery phrases in plaintext files.\n";

    try {
        auto host = std::make_shared<FilePlatformHost>();
        auto lifecycle = wallet_engine::WalletLifecycle::init(host);
        run_menu(*lifecycle);
        return 0;
    } catch (const std::exception &error) {
        std::cerr << "Failed to initialize Wallet Engine: " << error.what()
                  << '\n';
        return 1;
    }
}
