#include "wallet_engine.hpp"
#include "app_log.hpp"
#include "qt_http_host.hpp"

#include <QApplication>
#include <QByteArray>
#include <QButtonGroup>
#include <QCheckBox>
#include <QClipboard>
#include <QDateTime>
#include <QDialog>
#include <QFile>
#include <QFrame>
#include <QFutureWatcher>
#include <QGridLayout>
#include <QGuiApplication>
#include <QHBoxLayout>
#include <QLabel>
#include <QLineEdit>
#include <QMainWindow>
#include <QMessageBox>
#include <QPlainTextEdit>
#include <QPushButton>
#include <QScrollArea>
#include <QScrollBar>
#include <QSaveFile>
#include <QString>
#include <QStringList>
#include <QTextStream>
#include <QTextCursor>
#include <QTimer>
#include <QUuid>
#include <QVBoxLayout>
#include <QWidget>
#include <QtConcurrent/QtConcurrentRun>

#include <algorithm>
#include <cstdint>
#include <filesystem>
#include <fstream>
#include <map>
#include <memory>
#include <mutex>
#include <optional>
#include <sstream>
#include <string>
#include <utility>
#include <vector>

namespace {

using namespace wallet_engine;

constexpr const char *wallet_metadata_file = "wallet_engine_wallets.tsv";
constexpr const char *wallet_secrets_file = "wallet_engine_secrets.tsv";
constexpr const char *wallet_journal_file = "wallet_engine_journal.tsv";
constexpr const char *toncenter_api_key_file = "wallet_engine_toncenter_api_key";
constexpr qsizetype max_toncenter_api_key_bytes = 4 * 1024;

QString load_persisted_toncenter_api_key() {
    QFile file(QString::fromUtf8(toncenter_api_key_file));
    if (!file.exists()) {
        return {};
    }
    if (!file.open(QIODevice::ReadOnly)) {
        app_log(
            AppLogLevel::Warning,
            QStringLiteral("settings"),
            QStringLiteral("could not open persisted Toncenter API key")
        );
        return {};
    }
    const auto bytes = file.read(max_toncenter_api_key_bytes + 1);
    if (bytes.size() > max_toncenter_api_key_bytes) {
        app_log(
            AppLogLevel::Warning,
            QStringLiteral("settings"),
            QStringLiteral("persisted Toncenter API key exceeds size limit")
        );
        return {};
    }
    const auto api_key = QString::fromUtf8(bytes).trimmed();
    if (!api_key.isEmpty()) {
        app_log(
            AppLogLevel::Info,
            QStringLiteral("settings"),
            QStringLiteral("persisted Toncenter API key loaded")
        );
    }
    return api_key;
}

bool persist_toncenter_api_key(const QString &api_key, QString &error) {
    const auto bytes = api_key.trimmed().toUtf8();
    if (bytes.isEmpty() || bytes.size() > max_toncenter_api_key_bytes) {
        error = QStringLiteral("The API key has an invalid size.");
        return false;
    }

    QSaveFile file(QString::fromUtf8(toncenter_api_key_file));
    if (!file.open(QIODevice::WriteOnly)) {
        error = QStringLiteral("Could not open the local API-key file.");
        return false;
    }
    if (!file.setPermissions(QFileDevice::ReadOwner | QFileDevice::WriteOwner)) {
        file.cancelWriting();
        error = QStringLiteral("Could not restrict the API-key file permissions.");
        return false;
    }
    if (file.write(bytes) != bytes.size()) {
        file.cancelWriting();
        error = QStringLiteral("Could not write the local API-key file.");
        return false;
    }
    if (!file.commit()) {
        error = QStringLiteral("Could not commit the local API-key file.");
        return false;
    }
    app_log(
        AppLogLevel::Info,
        QStringLiteral("settings"),
        QStringLiteral("Toncenter API key persisted locally")
    );
    return true;
}

bool clear_persisted_toncenter_api_key(QString &error) {
    QFile file(QString::fromUtf8(toncenter_api_key_file));
    if (file.exists() && !file.remove()) {
        error = QStringLiteral("Could not remove the local API-key file.");
        return false;
    }
    app_log(
        AppLogLevel::Info,
        QStringLiteral("settings"),
        QStringLiteral("persisted Toncenter API key cleared")
    );
    return true;
}

[[noreturn]] void throw_secret_error(
    ProtectedSecretHostErrorKind kind,
    const std::string &diagnostic
) {
    protected_secret_host_error::Failed error(diagnostic);
    error.kind = kind;
    error.diagnostic = diagnostic;
    throw error;
}

[[noreturn]] void throw_journal_error(
    JournalHostErrorKind kind,
    const std::string &diagnostic
) {
    journal_host_error::Failed error(diagnostic);
    error.kind = kind;
    error.diagnostic = diagnostic;
    throw error;
}

void restrict_file_permissions(const char *path) {
    std::error_code permission_error;
    std::filesystem::permissions(
        path,
        std::filesystem::perms::owner_read |
            std::filesystem::perms::owner_write,
        std::filesystem::perm_options::replace,
        permission_error
    );
    if (permission_error) {
        throw std::runtime_error("failed to restrict demo storage permissions");
    }
}

QString secret_reason_text(SecretAccessReason reason) {
    switch (reason) {
    case SecretAccessReason::kCreateWallet:
        return QStringLiteral("create-wallet");
    case SecretAccessReason::kSignTransfer:
        return QStringLiteral("sign-transfer");
    case SecretAccessReason::kSignTonConnectProof:
        return QStringLiteral("sign-ton-connect-proof");
    case SecretAccessReason::kRevealRecoveryPhrase:
        return QStringLiteral("reveal-recovery-phrase");
    }
    return QStringLiteral("unknown");
}

class FilePlatformHost final : public WalletPlatformHost {
public:
    FilePlatformHost() {
        load_secrets_from_disk();
        load_journal_from_disk();
        app_log(
            AppLogLevel::Info,
            QStringLiteral("storage"),
            QStringLiteral("loaded secrets=%1 journal_records=%2")
                .arg(static_cast<qulonglong>(secrets_.size()))
                .arg(static_cast<qulonglong>(journal_.size()))
        );
    }

    std::vector<uint8_t> read_protected_secret(
        const ProtectedSecretRead &request
    ) override {
        app_log(
            AppLogLevel::Info,
            QStringLiteral("storage"),
            QStringLiteral("protected secret read reason=%1")
                .arg(secret_reason_text(request.reason))
        );
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
        app_log(
            AppLogLevel::Info,
            QStringLiteral("storage"),
            QStringLiteral("storing protected secret user_presence=%1")
                .arg(request.require_user_presence ? QStringLiteral("yes") :
                                                     QStringLiteral("no"))
        );
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

        try {
            restrict_file_permissions(wallet_secrets_file);
        } catch (const std::exception &) {
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
        app_log(
            AppLogLevel::Info,
            QStringLiteral("storage"),
            QStringLiteral("protected secret stored")
        );
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

        std::ofstream file(wallet_journal_file, std::ios::app);
        if (!file) {
            throw_journal_error(
                JournalHostErrorKind::kUnavailable,
                "failed to open the journal file"
            );
        }
        try {
            restrict_file_permissions(wallet_journal_file);
        } catch (const std::exception &) {
            throw_journal_error(
                JournalHostErrorKind::kUnavailable,
                "failed to restrict permissions on the journal file"
            );
        }
        const QByteArray payload(
            reinterpret_cast<const char *>(mutation.replacement.payload.data()),
            static_cast<qsizetype>(mutation.replacement.payload.size())
        );
        file << mutation.key.record_id << '\t' << mutation.key.slot << '\t'
             << mutation.replacement.version << '\t'
             << payload.toBase64().toStdString() << '\n';
        file.close();
        if (!file) {
            throw_journal_error(
                JournalHostErrorKind::kUnavailable,
                "failed to append the journal record"
            );
        }

        journal_[key] = mutation.replacement;
        app_log(
            AppLogLevel::Info,
            QStringLiteral("journal"),
            QStringLiteral("record committed slot=%1 version=%2")
                .arg(QString::fromStdString(mutation.key.slot))
                .arg(mutation.replacement.version)
        );
        return {true, mutation.replacement};
    }

private:
    void load_secrets_from_disk() {
        std::ifstream file(wallet_secrets_file, std::ios::binary);
        std::string line;
        while (std::getline(file, line)) {
            const auto first_tab = line.find('\t');
            const auto second_tab = first_tab == std::string::npos ?
                std::string::npos : line.find('\t', first_tab + 1);
            if (second_tab == std::string::npos) {
                continue;
            }
            const auto secret_ref = line.substr(0, first_tab);
            const auto secret = line.substr(second_tab + 1);
            secrets_[secret_ref] = std::vector<uint8_t>(
                secret.begin(),
                secret.end()
            );
        }
    }

    void load_journal_from_disk() {
        std::ifstream file(wallet_journal_file);
        std::string line;
        while (std::getline(file, line)) {
            const auto first_tab = line.find('\t');
            const auto second_tab = first_tab == std::string::npos ?
                std::string::npos : line.find('\t', first_tab + 1);
            const auto third_tab = second_tab == std::string::npos ?
                std::string::npos : line.find('\t', second_tab + 1);
            if (third_tab == std::string::npos) {
                continue;
            }
            try {
                const auto version = std::stoull(
                    line.substr(second_tab + 1, third_tab - second_tab - 1)
                );
                const auto payload = QByteArray::fromBase64(
                    QByteArray::fromStdString(line.substr(third_tab + 1))
                );
                journal_[{
                    line.substr(0, first_tab),
                    line.substr(first_tab + 1, second_tab - first_tab - 1),
                }] = {
                    version,
                    std::vector<uint8_t>(payload.begin(), payload.end()),
                };
            } catch (const std::exception &) {
                continue;
            }
        }
    }

    std::mutex mutex_;
    std::map<std::string, std::vector<uint8_t>> secrets_;
    std::map<std::pair<std::string, std::string>, JournalRecord> journal_;
};

bool append_wallet_metadata(const WalletDescriptor &descriptor) {
    std::ofstream file(wallet_metadata_file, std::ios::app);
    if (!file) {
        return false;
    }

    file << descriptor.record_id << '\t'
         << (descriptor.network == Network::kMainnet ? "mainnet" : "testnet")
         << '\t' << descriptor.address << '\t'
         << descriptor.secret_ref.value << '\t';
    const QByteArray public_key(
        reinterpret_cast<const char *>(descriptor.public_key.data()),
        static_cast<qsizetype>(descriptor.public_key.size())
    );
    file << public_key.toBase64().toStdString() << '\n';
    return static_cast<bool>(file);
}

QString describe_lifecycle_error(const WalletLifecycleError &error) {
    if (dynamic_cast<const wallet_lifecycle_error::InvalidRecordId *>(&error)) {
        return QStringLiteral("The record ID is invalid.");
    }
    if (
        dynamic_cast<const wallet_lifecycle_error::AddressDerivationFailed *>(
            &error
        )
    ) {
        return QStringLiteral("Wallet address derivation failed.");
    }
    if (const auto *host_error =
            dynamic_cast<const wallet_lifecycle_error::ProtectedSecretHost *>(
                &error
            )) {
        return QStringLiteral("Protected-storage error: %1")
            .arg(QString::fromStdString(host_error->diagnostic));
    }
    return QStringLiteral("Wallet lifecycle operation failed.");
}

struct CreateResult {
    std::optional<CreatedWallet> wallet;
    QString error;
};

struct SavedWallet {
    QString record_id;
    QString network;
    QString address;
    QString secret_ref;
    std::vector<uint8_t> public_key;
};

std::vector<SavedWallet> load_wallet_metadata() {
    QFile file(QString::fromUtf8(wallet_metadata_file));
    if (!file.open(QIODevice::ReadOnly | QIODevice::Text)) {
        return {};
    }

    std::vector<SavedWallet> wallets;
    QTextStream stream(&file);
    while (!stream.atEnd()) {
        const auto columns = stream.readLine().split('\t');
        if (columns.size() < 4) {
            continue;
        }
        std::vector<uint8_t> public_key;
        if (columns.size() >= 5) {
            const auto decoded = QByteArray::fromBase64(columns[4].toUtf8());
            public_key.assign(decoded.begin(), decoded.end());
        }
        SavedWallet wallet {
            columns[0],
            columns[1],
            columns[2],
            columns[3],
            std::move(public_key),
        };
        bool replaced = false;
        for (auto &existing : wallets) {
            if (
                existing.record_id.compare(
                    wallet.record_id,
                    Qt::CaseInsensitive
                ) == 0
            ) {
                existing = std::move(wallet);
                replaced = true;
                break;
            }
        }
        if (!replaced) {
            wallets.push_back(std::move(wallet));
        }
    }
    return wallets;
}

SavedWallet saved_wallet_from(const WalletDescriptor &descriptor) {
    return {
        QString::fromStdString(descriptor.record_id),
        descriptor.network == Network::kMainnet ? QStringLiteral("mainnet") :
                                                   QStringLiteral("testnet"),
        QString::fromStdString(descriptor.address),
        QString::fromStdString(descriptor.secret_ref.value),
        descriptor.public_key,
    };
}

bool wallet_name_exists(const QString &record_id) {
    for (const auto &wallet : load_wallet_metadata()) {
        if (
            wallet.record_id.compare(record_id, Qt::CaseInsensitive) == 0
        ) {
            return true;
        }
    }
    return false;
}

QString shortened_address(const QString &address) {
    if (address.size() <= 28) {
        return address;
    }
    return address.left(14) + QStringLiteral("…") + address.right(10);
}

CreateResult create_wallet(
    const std::shared_ptr<WalletLifecycle> &lifecycle,
    const CreateWalletRequest &request
) {
    try {
        return {lifecycle->create_wallet(request), {}};
    } catch (const WalletLifecycleError &error) {
        return {std::nullopt, describe_lifecycle_error(error)};
    } catch (const std::exception &error) {
        return {
            std::nullopt,
            QStringLiteral("FFI error: %1").arg(
                QString::fromUtf8(error.what())
            ),
        };
    }
}

QString format_ton(const std::string &nanograms, int maximum_decimals = 4) {
    auto digits = QString::fromStdString(nanograms);
    while (digits.size() > 1 && digits.startsWith('0')) {
        digits.removeFirst();
    }
    while (digits.size() <= 9) {
        digits.prepend('0');
    }
    const auto whole = digits.left(digits.size() - 9);
    auto fraction = digits.right(9).left(maximum_decimals);
    while (fraction.endsWith('0')) {
        fraction.chop(1);
    }
    return fraction.isEmpty() ? whole : whole + QStringLiteral(".") + fraction;
}

std::optional<std::string> parse_ton_amount(
    const QString &input,
    QString &error
) {
    const auto value = input.trimmed();
    if (value.isEmpty() || value.count('.') > 1) {
        error = QStringLiteral("Enter a valid TON amount.");
        return std::nullopt;
    }
    for (const auto character : value) {
        if (!character.isDigit() && character != '.') {
            error = QStringLiteral("The amount can contain only digits and a decimal point.");
            return std::nullopt;
        }
    }

    const auto parts = value.split('.');
    auto whole = parts.value(0);
    auto fraction = parts.size() == 2 ? parts[1] : QString();
    if (whole.isEmpty()) {
        whole = QStringLiteral("0");
    }
    if (fraction.size() > 9) {
        error = QStringLiteral("TON supports at most 9 decimal places.");
        return std::nullopt;
    }
    if (whole.size() > 30) {
        error = QStringLiteral("The amount is too large.");
        return std::nullopt;
    }
    fraction = fraction.leftJustified(9, '0');
    auto nanograms = whole + fraction;
    while (nanograms.size() > 1 && nanograms.startsWith('0')) {
        nanograms.removeFirst();
    }
    if (nanograms == QStringLiteral("0")) {
        error = QStringLiteral("The amount must be greater than zero.");
        return std::nullopt;
    }
    return nanograms.toStdString();
}

QString account_status_text(AccountStatus status);

QString with_diagnostic(
    const QString &message,
    const std::string &diagnostic
) {
    const auto details = QString::fromStdString(diagnostic).trimmed();
    return details.isEmpty() ? message :
        message + QStringLiteral("\n\nDetails: ") + details;
}

QString describe_domain_error(const DomainError &error) {
    QString message;
    switch (error.code) {
    case ErrorCode::kInvalidProviderResponse:
        message = QStringLiteral("Toncenter returned an invalid response.");
        break;
    case ErrorCode::kHttpRejected:
        message = QStringLiteral("Toncenter rejected the request.");
        break;
    case ErrorCode::kRateLimited:
        message = QStringLiteral(
            "Toncenter rate limit reached. Add an API key in Settings or retry later."
        );
        break;
    case ErrorCode::kTransportFailed:
        message = QStringLiteral(
            "Could not connect to Toncenter. Check the network connection and provider settings."
        );
        break;
    case ErrorCode::kHostCancelled:
        message = QStringLiteral("The provider request was cancelled.");
        break;
    case ErrorCode::kResponseTooLarge:
        message = QStringLiteral(
            "Toncenter returned a response larger than the safety limit."
        );
        break;
    case ErrorCode::kHostPolicyViolation:
        message = QStringLiteral(
            "The HTTP host rejected the request because it violated a security policy."
        );
        break;
    }
    if (error.provider_status.has_value()) {
        message += QStringLiteral("\nHTTP status: %1")
            .arg(error.provider_status.value());
    }
    if (error.retry_after_ms.has_value()) {
        message += QStringLiteral("\nRetry after: %1 seconds")
            .arg(error.retry_after_ms.value() / 1000.0, 0, 'f', 1);
    }
    return with_diagnostic(message, error.developer_message);
}

QString describe_client_error(const WalletClientError &error) {
    if (
        dynamic_cast<
            const wallet_client_error::InvalidLocalSecretReference *
        >(&error)
    ) {
        return QStringLiteral(
            "The wallet metadata does not contain a valid secret reference."
        );
    }
    if (
        dynamic_cast<const wallet_client_error::InvalidWalletPublicKey *>(
            &error
        )
    ) {
        return QStringLiteral("The saved wallet public key is invalid.");
    }
    if (
        dynamic_cast<const wallet_client_error::WalletIdentityMismatch *>(
            &error
        )
    ) {
        return QStringLiteral(
            "The wallet address does not match its public key and selected network."
        );
    }
    if (
        dynamic_cast<const wallet_client_error::InvalidProviderBaseUrl *>(
            &error
        )
    ) {
        return QStringLiteral("The configured Toncenter endpoint is invalid.");
    }
    if (
        dynamic_cast<const wallet_client_error::InvalidSendRequest *>(&error)
    ) {
        return QStringLiteral(
            "The transfer request is invalid. Check the destination address, amount, and comment."
        );
    }
    if (
        dynamic_cast<const wallet_client_error::LocalSigningUnavailable *>(
            &error
        )
    ) {
        return QStringLiteral(
            "Local signing is unavailable because this wallet has no stored recovery phrase."
        );
    }
    if (
        dynamic_cast<const wallet_client_error::IdentifierExhausted *>(&error)
    ) {
        return QStringLiteral(
            "Wallet Engine could not allocate another operation identifier. Restart the session."
        );
    }
    if (
        dynamic_cast<const wallet_client_error::StateUnavailable *>(&error)
    ) {
        return QStringLiteral(
            "Wallet state is temporarily unavailable. Wait for the current operation and retry."
        );
    }
    if (
        dynamic_cast<const wallet_client_error::SendAlreadyInProgress *>(
            &error
        )
    ) {
        return QStringLiteral("Another transfer is already in progress.");
    }
    if (
        dynamic_cast<
            const wallet_client_error::SendPreviewAlreadyInProgress *
        >(&error)
    ) {
        return QStringLiteral("Another transfer preview is already in progress.");
    }
    if (const auto *balance =
            dynamic_cast<const wallet_client_error::InsufficientBalance *>(
                &error
            )) {
        return QStringLiteral("Insufficient balance: %1 TON available.")
            .arg(format_ton(balance->available_nanograms));
    }
    if (const auto *fees =
            dynamic_cast<
                const wallet_client_error::InsufficientBalanceForFees *
            >(&error)) {
        return QStringLiteral(
            "The balance covers the amount but not the estimated %1 TON fee."
        ).arg(format_ton(fees->estimated_fee_nanograms, 9));
    }
    if (
        dynamic_cast<const wallet_client_error::PreviousSubmissionUnresolved *>(
            &error
        )
    ) {
        return QStringLiteral(
            "A previous transfer is still unresolved. Refresh before sending again."
        );
    }
    if (
        dynamic_cast<const wallet_client_error::WalletSeqnoNotAdvanced *>(
            &error
        )
    ) {
        return QStringLiteral(
            "The previous transfer sequence number is still active. Refresh and wait for confirmation."
        );
    }
    if (const auto *account =
            dynamic_cast<const wallet_client_error::SendAccountUnavailable *>(
                &error
            )) {
        return QStringLiteral("The account cannot send in its current state: %1.")
            .arg(account_status_text(account->status));
    }
    if (
        dynamic_cast<const wallet_client_error::InvalidProtectedSecret *>(
            &error
        )
    ) {
        return QStringLiteral(
            "The stored recovery phrase is invalid or belongs to another wallet."
        );
    }
    if (const auto *preview =
            dynamic_cast<const wallet_client_error::SendPreviewFailed *>(
                &error
            )) {
        return with_diagnostic(
            QStringLiteral("Could not prepare the transfer preview."),
            preview->diagnostic
        );
    }
    if (const auto *emulation =
            dynamic_cast<const wallet_client_error::EmulationFailed *>(
                &error
            )) {
        return with_diagnostic(
            QStringLiteral("Toncenter could not emulate the transfer."),
            emulation->diagnostic
        );
    }
    if (const auto *message =
            dynamic_cast<
                const wallet_client_error::EmulationMessageNotAccepted *
            >(&error)) {
        return with_diagnostic(
            QStringLiteral(
                "The wallet contract did not accept the emulated message. Refresh and retry."
            ),
            message->diagnostic
        );
    }
    if (const auto *rejected =
            dynamic_cast<const wallet_client_error::EmulationRejected *>(
                &error
            )) {
        auto message = QStringLiteral(
            "The emulated transaction would fail, so it was not submitted."
        );
        if (rejected->compute_exit_code.has_value()) {
            message += QStringLiteral("\nCompute exit code: %1")
                .arg(rejected->compute_exit_code.value());
        }
        if (rejected->action_result_code.has_value()) {
            message += QStringLiteral("\nAction result code: %1")
                .arg(rejected->action_result_code.value());
        }
        return with_diagnostic(message, rejected->diagnostic);
    }
    if (const auto *send =
            dynamic_cast<const wallet_client_error::SendFailed *>(&error)) {
        return with_diagnostic(
            QStringLiteral("The transfer failed before it was submitted."),
            send->diagnostic
        );
    }
    if (const auto *unknown =
            dynamic_cast<const wallet_client_error::SubmissionUnknown *>(
                &error
            )) {
        return with_diagnostic(
            QStringLiteral(
                "Submission outcome is unknown. Do not repeat the transfer."
            ),
            unknown->diagnostic
        );
    }
    if (
        dynamic_cast<const wallet_client_error::SendCancellationTooLate *>(
            &error
        )
    ) {
        return QStringLiteral(
            "The transfer is already durable and can no longer be cancelled. Wait for its result."
        );
    }
    if (dynamic_cast<const wallet_client_error::Shutdown *>(&error)) {
        return QStringLiteral(
            "This wallet session has already been closed. Open the wallet again."
        );
    }
    const auto message = QString::fromUtf8(error.what()).trimmed();
    return message.isEmpty() ?
        QStringLiteral(
            "Wallet Engine returned an unrecognized operational error. "
            "Regenerate the C++ bindings and rebuild the application."
        ) : message;
}

struct ClientUpdateResult {
    std::optional<WalletUpdate> update;
    QString error;
};

ClientUpdateResult update_wallet(
    const std::shared_ptr<WalletClient> &client,
    bool load_more
) {
    try {
        return {
            load_more ? client->load_more_activity() : client->refresh(),
            {},
        };
    } catch (const WalletClientError &error) {
        return {std::nullopt, describe_client_error(error)};
    } catch (const std::exception &error) {
        return {std::nullopt, QString::fromUtf8(error.what())};
    }
}

struct TransferDraft {
    std::string destination;
    std::string amount_nanograms;
    std::optional<std::string> comment;
};

struct PreviewResult {
    std::optional<SendPreview> preview;
    QString error;
};

/** Builds the transfer intent used by both preview and signed submission. */
SendIntent transfer_intent(const TransferDraft &draft) {
    const auto body = draft.comment.has_value() ?
        SendMessageBody(SendMessageBody::kComment {*draft.comment}) :
        SendMessageBody(SendMessageBody::kEmpty {});
    return {
        SendExpiration(SendExpiration::kEngineDefault {}),
        SendMessage {
            draft.destination,
            SendAmount(SendAmount::kExact {draft.amount_nanograms}),
            body,
            std::nullopt,
        },
    };
}

PreviewResult preview_transfer(
    const std::shared_ptr<WalletClient> &client,
    const TransferDraft &draft
) {
    try {
        return {
            client->preview_send({
                transfer_intent(draft),
            }),
            {},
        };
    } catch (const WalletClientError &error) {
        return {std::nullopt, describe_client_error(error)};
    } catch (const std::exception &error) {
        return {std::nullopt, QString::fromUtf8(error.what())};
    }
}

struct SubmitResult {
    std::optional<SendResult> result;
    QString error;
};

SubmitResult submit_transfer(
    const std::shared_ptr<WalletClient> &client,
    const TransferDraft &draft,
    const std::string &operation_id
) {
    try {
        return {
            client->send({
                operation_id,
                transfer_intent(draft),
            }),
            {},
        };
    } catch (const WalletClientError &error) {
        return {std::nullopt, describe_client_error(error)};
    } catch (const std::exception &error) {
        return {std::nullopt, QString::fromUtf8(error.what())};
    }
}

QString account_status_text(AccountStatus status) {
    switch (status) {
    case AccountStatus::kNonexistent:
        return QStringLiteral("Not deployed");
    case AccountStatus::kUninitialized:
        return QStringLiteral("Uninitialized");
    case AccountStatus::kActive:
        return QStringLiteral("Active");
    case AccountStatus::kFrozen:
        return QStringLiteral("Frozen");
    case AccountStatus::kUnknown:
        return QStringLiteral("Unknown state");
    }
    return QStringLiteral("Unknown state");
}

QString update_outcome_text(WalletOperationOutcome outcome) {
    switch (outcome) {
    case WalletOperationOutcome::kCompleted:
        return QStringLiteral("completed");
    case WalletOperationOutcome::kPartiallyCompleted:
        return QStringLiteral("partially-completed");
    case WalletOperationOutcome::kFailed:
        return QStringLiteral("failed");
    case WalletOperationOutcome::kCancelled:
        return QStringLiteral("cancelled");
    case WalletOperationOutcome::kSuperseded:
        return QStringLiteral("superseded");
    case WalletOperationOutcome::kSkipped:
        return QStringLiteral("skipped");
    }
    return QStringLiteral("unknown");
}

QString send_phase_text(SendPhase phase) {
    switch (phase) {
    case SendPhase::kIdle:
        return QStringLiteral("idle");
    case SendPhase::kValidating:
        return QStringLiteral("validating");
    case SendPhase::kAuthorizing:
        return QStringLiteral("authorizing");
    case SendPhase::kPreparing:
        return QStringLiteral("preparing");
    case SendPhase::kPersisting:
        return QStringLiteral("persisting");
    case SendPhase::kReadyToSubmit:
        return QStringLiteral("ready-to-submit");
    case SendPhase::kSubmitting:
        return QStringLiteral("submitting");
    case SendPhase::kSubmissionUnknown:
        return QStringLiteral("submission-unknown");
    case SendPhase::kSubmitted:
        return QStringLiteral("submitted");
    case SendPhase::kConfirmed:
        return QStringLiteral("confirmed");
    case SendPhase::kReplaced:
        return QStringLiteral("replaced");
    case SendPhase::kExpired:
        return QStringLiteral("expired");
    case SendPhase::kSuperseded:
        return QStringLiteral("superseded");
    case SendPhase::kFailed:
        return QStringLiteral("failed");
    case SendPhase::kCancelled:
        return QStringLiteral("cancelled");
    }
    return QStringLiteral("unknown");
}

QString describe_update_errors(
    const WalletUpdate &update,
    bool loading_more
) {
    QStringList messages;
    if (
        update.snapshot.account_resource.phase == ResourcePhase::kFailed &&
        update.snapshot.account_resource.error.has_value()
    ) {
        messages.push_back(
            QStringLiteral("Account\n%1")
                .arg(describe_domain_error(
                    update.snapshot.account_resource.error.value()
                ))
        );
    }
    const auto &activity_resource = loading_more ?
        update.snapshot.activity_pagination_resource :
        update.snapshot.activity_resource;
    if (
        activity_resource.phase == ResourcePhase::kFailed &&
        activity_resource.error.has_value()
    ) {
        messages.push_back(
            QStringLiteral("Activity\n%1")
                .arg(describe_domain_error(activity_resource.error.value()))
        );
    }
    if (!messages.isEmpty()) {
        return messages.join(QStringLiteral("\n\n"));
    }
    switch (update.outcome) {
    case WalletOperationOutcome::kCancelled:
        return QStringLiteral("The wallet update was cancelled.");
    case WalletOperationOutcome::kSuperseded:
        return QStringLiteral("A newer refresh replaced this wallet update.");
    case WalletOperationOutcome::kSkipped:
        return QStringLiteral("There is no additional wallet data to load.");
    case WalletOperationOutcome::kFailed:
        return QStringLiteral("Wallet Engine could not update the requested resources.");
    case WalletOperationOutcome::kPartiallyCompleted:
        return QStringLiteral("Only part of the wallet data could be updated.");
    case WalletOperationOutcome::kCompleted:
        return {};
    }
    return QStringLiteral("The wallet update ended with an unknown outcome.");
}

class MainWindow final : public QMainWindow {
public:
    explicit MainWindow(
        std::shared_ptr<WalletLifecycle> lifecycle,
        std::shared_ptr<FilePlatformHost> platform_host,
        std::shared_ptr<QtHttpHost> http_host,
        QWidget *parent = nullptr
    ) :
        QMainWindow(parent),
        lifecycle_(std::move(lifecycle)),
        platform_host_(std::move(platform_host)),
        http_host_(std::move(http_host)) {
        setWindowTitle(QStringLiteral("Wallet Engine"));
        setMinimumSize(1040, 680);
        resize(1180, 760);
        setStyleSheet(application_style());

        auto *central = new QWidget(this);
        central->setObjectName(QStringLiteral("appShell"));
        auto *shell = new QHBoxLayout(central);
        shell->setContentsMargins(0, 0, 0, 0);
        shell->setSpacing(0);

        shell->addWidget(build_sidebar(central));

        auto *scroll = new QScrollArea(central);
        content_scroll_ = scroll;
        scroll->setObjectName(QStringLiteral("contentScroll"));
        scroll->setWidgetResizable(true);
        scroll->setFrameShape(QFrame::NoFrame);
        scroll->setHorizontalScrollBarPolicy(Qt::ScrollBarAlwaysOff);

        auto *content = new QWidget(scroll);
        content->setObjectName(QStringLiteral("content"));
        auto *root = new QVBoxLayout(content);
        root->setContentsMargins(36, 30, 36, 36);
        root->setSpacing(24);
        root->addLayout(build_header(content));

        auto *overview = new QHBoxLayout;
        overview->setSpacing(20);
        overview->addWidget(build_portfolio_card(content), 3);
        overview->addWidget(build_create_card(content), 2);
        root->addLayout(overview);
        wallets_card_ = build_wallets_card(content);
        activity_card_ = build_activity_card(content);
        root->addWidget(wallets_card_, 1);
        root->addWidget(activity_card_, 1);

        scroll->setWidget(content);
        shell->addWidget(scroll, 1);

        setCentralWidget(central);

        connect(create_button_, &QPushButton::clicked, this, [this] {
            start_create_wallet();
        });
        connect(record_id_, &QLineEdit::returnPressed, this, [this] {
            start_create_wallet();
        });
        connect(
            &create_watcher_,
            &QFutureWatcher<CreateResult>::finished,
            this,
            [this] { finish_create_wallet(); }
        );
        connect(refresh_button_, &QPushButton::clicked, this, [this] {
            start_wallet_update(false);
        });
        connect(receive_button_, &QPushButton::clicked, this, [this] {
            show_receive_dialog();
        });
        connect(send_button_, &QPushButton::clicked, this, [this] {
            show_send_dialog();
        });
        connect(portfolio_nav_, &QPushButton::clicked, this, [this] {
            select_navigation(portfolio_nav_);
            content_scroll_->verticalScrollBar()->setValue(0);
        });
        connect(wallets_nav_, &QPushButton::clicked, this, [this] {
            select_navigation(wallets_nav_);
            content_scroll_->ensureWidgetVisible(wallets_card_, 0, 24);
        });
        connect(activity_nav_, &QPushButton::clicked, this, [this] {
            select_navigation(activity_nav_);
            content_scroll_->ensureWidgetVisible(activity_card_, 0, 24);
        });
        connect(settings_nav_, &QPushButton::clicked, this, [this] {
            settings_nav_->setChecked(false);
            show_toncenter_settings();
        });
        connect(logs_nav_, &QPushButton::clicked, this, [this] {
            logs_nav_->setChecked(false);
            show_logs_dialog();
        });
        connect(
            &update_watcher_,
            &QFutureWatcher<ClientUpdateResult>::finished,
            this,
            [this] { finish_wallet_update(); }
        );
        connect(
            &preview_watcher_,
            &QFutureWatcher<PreviewResult>::finished,
            this,
            [this] { finish_send_preview(); }
        );
        connect(
            &send_watcher_,
            &QFutureWatcher<SubmitResult>::finished,
            this,
            [this] { finish_send(); }
        );

        reload_wallets();
    }

private:
    static QString application_style() {
        return QStringLiteral(R"(
            QMainWindow, QWidget#appShell, QWidget#content {
                background: #0d0d14;
                color: #f5f3ff;
            }
            * {
                font-family: "Inter", "SF Pro Display", "Segoe UI";
            }
            QScrollArea#contentScroll {
                background: #0d0d14;
                border: none;
            }
            QScrollBar:vertical {
                background: #0d0d14;
                width: 10px;
                margin: 0;
            }
            QScrollBar::handle:vertical {
                background: #383548;
                border-radius: 5px;
                min-height: 32px;
            }
            QScrollBar::handle:vertical:hover { background: #4b4660; }
            QScrollBar::add-line:vertical, QScrollBar::sub-line:vertical {
                height: 0;
            }
            QScrollBar::add-page:vertical, QScrollBar::sub-page:vertical {
                background: transparent;
            }
            QFrame#sidebar {
                background: #12121c;
                border-right: 1px solid #242435;
            }
            QLabel#brandMark {
                background: #7c5cff;
                color: white;
                border-radius: 12px;
                font-size: 18px;
                font-weight: 800;
            }
            QLabel#brandName, QLabel#pageTitle {
                color: #ffffff;
                font-weight: 750;
            }
            QLabel#brandName { font-size: 16px; }
            QLabel#brandCaption, QLabel#eyebrow, QLabel#fieldLabel,
            QLabel#muted, QLabel#address, QLabel#securityCaption {
                color: #9291a4;
            }
            QLabel#brandCaption, QLabel#eyebrow, QLabel#fieldLabel {
                font-size: 10px;
                font-weight: 700;
                letter-spacing: 1px;
            }
            QLabel#pageTitle { font-size: 27px; }
            QLabel#pageSubtitle { color: #9291a4; font-size: 13px; }
            QPushButton#navButton {
                background: transparent;
                color: #8f8e9f;
                border: none;
                border-radius: 10px;
                padding: 11px 14px;
                text-align: left;
                font-size: 13px;
                font-weight: 600;
            }
            QPushButton#navButton:hover { background: #1b1a28; color: #ffffff; }
            QPushButton#navButton:checked {
                background: #28223f;
                color: #b7a7ff;
            }
            QPushButton#navButton:disabled { color: #4f4e5c; }
            QFrame#securityCard {
                background: #191824;
                border: 1px solid #29283a;
                border-radius: 12px;
            }
            QLabel#securityTitle { color: #d8d5e4; font-size: 12px; font-weight: 700; }
            QLabel#statusDot { color: #50e3aa; font-size: 16px; }
            QLabel#engineBadge, QLabel#countBadge, QLabel#networkBadge {
                background: #1b1a27;
                border: 1px solid #302e42;
                border-radius: 12px;
                color: #bcb8cb;
                padding: 5px 10px;
                font-size: 11px;
                font-weight: 650;
            }
            QLabel#engineBadge { color: #62ddb1; }
            QLabel#networkBadge[network="testnet"] {
                background: #24203b;
                border-color: #403568;
                color: #b9a7ff;
            }
            QLabel#networkBadge[network="mainnet"] {
                background: #19302b;
                border-color: #285247;
                color: #70e0bc;
            }
            QFrame#card {
                background: #171721;
                border: 1px solid #292838;
                border-radius: 18px;
            }
            QFrame#balanceCard {
                background: #211a38;
                border: 1px solid #3c3064;
                border-radius: 18px;
            }
            QLabel#cardTitle { color: #ffffff; font-size: 17px; font-weight: 700; }
            QLabel#balance { color: #ffffff; font-size: 38px; font-weight: 750; }
            QLabel#balanceHint { color: #aaa1c5; font-size: 12px; }
            QLabel#statValue { color: #ffffff; font-size: 15px; font-weight: 700; }
            QLabel#statLabel { color: #918ba5; font-size: 10px; }
            QFrame#statDivider { background: #44365f; border: none; }
            QPushButton#primaryButton {
                background: #7c5cff;
                color: white;
                border: none;
                border-radius: 11px;
                padding: 11px 18px;
                font-size: 12px;
                font-weight: 700;
            }
            QPushButton#primaryButton:hover { background: #8b70ff; }
            QPushButton#primaryButton:pressed { background: #6848ea; }
            QPushButton#primaryButton:disabled { background: #3b3459; color: #77718c; }
            QPushButton#secondaryButton, QPushButton#quietButton {
                background: #252337;
                color: #ddd8ee;
                border: 1px solid #3a3750;
                border-radius: 11px;
                padding: 10px 16px;
                font-size: 12px;
                font-weight: 650;
            }
            QPushButton#secondaryButton:hover, QPushButton#quietButton:hover {
                background: #302d44;
                color: white;
            }
            QPushButton#secondaryButton:disabled { color: #656174; background: #201f2b; }
            QPushButton#quietButton { padding: 7px 12px; }
            QLineEdit {
                min-height: 40px;
                background: #101019;
                color: #f4f1ff;
                border: 1px solid #343244;
                border-radius: 10px;
                padding: 0 12px;
                selection-background-color: #7c5cff;
            }
            QLineEdit:focus { border: 1px solid #8167f5; }
            QFrame#networkSelector {
                min-height: 40px;
                background: #101019;
                border: 1px solid #343244;
                border-radius: 10px;
            }
            QPushButton#networkOption {
                min-height: 34px;
                background: transparent;
                color: #858293;
                border: none;
                border-radius: 8px;
                padding: 0 12px;
                font-size: 12px;
                font-weight: 650;
            }
            QPushButton#networkOption:hover {
                background: #1c1a29;
                color: #d9d5e7;
            }
            QPushButton#networkOption:checked {
                background: #342957;
                color: #c5b8ff;
            }
            QPushButton#networkOption:disabled {
                background: transparent;
                color: #575464;
            }
            QLabel#inlineStatus { color: #8f8d9e; font-size: 11px; }
            QLabel#warningText { color: #d5ad65; font-size: 11px; }
            QFrame#walletRow {
                background: #11111a;
                border: 1px solid #282737;
                border-radius: 13px;
            }
            QFrame#walletRow:hover { border-color: #4a426b; background: #151420; }
            QLabel#walletIcon {
                background: #2d2450;
                color: #bcaeff;
                border-radius: 18px;
                font-size: 14px;
                font-weight: 800;
            }
            QLabel#walletName { color: #f8f6ff; font-size: 13px; font-weight: 700; }
            QLabel#address {
                font-family: "SF Mono", "Menlo", "Consolas", monospace;
                font-size: 11px;
            }
            QFrame#emptyState {
                background: #11111a;
                border: 1px dashed #343244;
                border-radius: 13px;
            }
            QLabel#emptyIcon { color: #7765bd; font-size: 28px; }
            QLabel#emptyTitle { color: #e8e4f5; font-size: 14px; font-weight: 700; }
            QDialog#recoveryDialog, QDialog#actionDialog {
                background: #11111a;
                color: #f5f3ff;
            }
            QLabel#recoveryIcon {
                background: #2b2350;
                color: #b7a6ff;
                border-radius: 22px;
                font-size: 20px;
            }
            QLabel#dialogTitle { color: white; font-size: 23px; font-weight: 750; }
            QFrame#warningCard {
                background: #302717;
                border: 1px solid #5d4823;
                border-radius: 11px;
            }
            QLabel#dialogWarning { color: #e3be78; font-size: 12px; }
            QFrame#phraseCard {
                background: #171721;
                border: 1px solid #302e42;
                border-radius: 14px;
            }
            QScrollArea#phraseScroll {
                background: transparent;
                border: none;
            }
            QScrollArea#phraseScroll > QWidget > QWidget {
                background: transparent;
            }
            QFrame#phraseWord {
                background: #101019;
                border: 1px solid #292737;
                border-radius: 8px;
                min-height: 40px;
            }
            QLabel#wordNumber { color: #777386; font-size: 11px; }
            QLabel#wordText {
                color: #f2efff;
                font-family: "SF Mono", "Menlo", "Consolas", monospace;
                font-size: 13px;
                font-weight: 650;
            }
            QCheckBox { color: #c4c0d1; font-size: 12px; spacing: 8px; }
            QCheckBox::indicator {
                width: 17px; height: 17px; border-radius: 4px;
                border: 1px solid #4a465e; background: #16151f;
            }
            QCheckBox::indicator:checked { background: #7c5cff; border-color: #7c5cff; }
            QFrame#activityRow {
                background: #11111a;
                border: 1px solid #282737;
                border-radius: 12px;
            }
            QLabel#activityIconSent, QLabel#activityIconReceived {
                border-radius: 17px;
                font-size: 15px;
                font-weight: 800;
            }
            QLabel#activityIconSent { background: #352338; color: #f09ca4; }
            QLabel#activityIconReceived { background: #17342d; color: #62dcb1; }
            QLabel#activityAmountSent { color: #f0a0aa; font-size: 13px; font-weight: 700; }
            QLabel#activityAmountReceived { color: #62dcb1; font-size: 13px; font-weight: 700; }
            QLabel#activityTitle { color: #f6f3ff; font-size: 13px; font-weight: 700; }
            QLabel#dialogSection { color: #ffffff; font-size: 15px; font-weight: 700; }
            QLabel#feeValue { color: #c9beff; font-size: 15px; font-weight: 700; }
            QLabel#transactionAmount {
                color: #ffffff;
                font-size: 28px;
                font-weight: 750;
            }
            QFrame#detailsCard {
                background: #171721;
                border: 1px solid #302e42;
                border-radius: 13px;
            }
            QLabel#detailValue {
                color: #e9e5f2;
                font-family: "SF Mono", "Menlo", "Consolas", monospace;
                font-size: 11px;
            }
            QPlainTextEdit#logView {
                background: #090910;
                color: #c9c5d5;
                border: 1px solid #302e42;
                border-radius: 10px;
                padding: 10px;
                font-family: "SF Mono", "Menlo", "Consolas", monospace;
                font-size: 11px;
                selection-background-color: #493b79;
            }
            QFrame#walletRow[selected="true"] {
                background: #1d1830;
                border-color: #7056c9;
            }
        )");
    }

    QWidget *build_sidebar(QWidget *parent) {
        auto *sidebar = new QFrame(parent);
        sidebar->setObjectName(QStringLiteral("sidebar"));
        sidebar->setFixedWidth(220);
        auto *layout = new QVBoxLayout(sidebar);
        layout->setContentsMargins(20, 25, 20, 22);
        layout->setSpacing(8);

        auto *brand = new QHBoxLayout;
        brand->setSpacing(11);
        auto *mark = new QLabel(QStringLiteral("W"), sidebar);
        mark->setObjectName(QStringLiteral("brandMark"));
        mark->setAlignment(Qt::AlignCenter);
        mark->setFixedSize(40, 40);
        brand->addWidget(mark);

        auto *brand_text = new QVBoxLayout;
        brand_text->setSpacing(1);
        auto *name = new QLabel(QStringLiteral("Wallet Engine"), sidebar);
        name->setObjectName(QStringLiteral("brandName"));
        auto *caption = new QLabel(QStringLiteral("C++ DESKTOP"), sidebar);
        caption->setObjectName(QStringLiteral("brandCaption"));
        brand_text->addWidget(name);
        brand_text->addWidget(caption);
        brand->addLayout(brand_text);
        brand->addStretch();
        layout->addLayout(brand);
        layout->addSpacing(31);

        auto *menu = new QLabel(QStringLiteral("MENU"), sidebar);
        menu->setObjectName(QStringLiteral("eyebrow"));
        layout->addWidget(menu);
        layout->addSpacing(5);
        portfolio_nav_ = nav_button(
            QStringLiteral("◈   Portfolio"),
            true,
            true,
            sidebar
        );
        wallets_nav_ = nav_button(
            QStringLiteral("○   Wallets"),
            false,
            true,
            sidebar
        );
        activity_nav_ = nav_button(
            QStringLiteral("↗   Activity"),
            false,
            true,
            sidebar
        );
        layout->addWidget(portfolio_nav_);
        layout->addWidget(wallets_nav_);
        layout->addWidget(activity_nav_);
        settings_nav_ = nav_button(
            QStringLiteral("⚙   Settings"),
            false,
            true,
            sidebar
        );
        layout->addWidget(settings_nav_);
        logs_nav_ = nav_button(
            QStringLiteral("≡   Logs"),
            false,
            true,
            sidebar
        );
        layout->addWidget(logs_nav_);
        layout->addStretch();

        auto *security = new QFrame(sidebar);
        security->setObjectName(QStringLiteral("securityCard"));
        auto *security_layout = new QVBoxLayout(security);
        security_layout->setContentsMargins(13, 12, 13, 12);
        security_layout->setSpacing(4);
        auto *status_row = new QHBoxLayout;
        status_row->setSpacing(6);
        auto *dot = new QLabel(QStringLiteral("●"), security);
        dot->setObjectName(QStringLiteral("statusDot"));
        auto *title = new QLabel(QStringLiteral("Local engine ready"), security);
        title->setObjectName(QStringLiteral("securityTitle"));
        status_row->addWidget(dot);
        status_row->addWidget(title);
        status_row->addStretch();
        security_layout->addLayout(status_row);
        auto *security_caption = new QLabel(
            QStringLiteral("Generated C++ bindings\nDemo storage adapter"),
            security
        );
        security_caption->setObjectName(QStringLiteral("securityCaption"));
        security_layout->addWidget(security_caption);
        layout->addWidget(security);
        return sidebar;
    }

    static QPushButton *nav_button(
        const QString &text,
        bool active,
        bool enabled,
        QWidget *parent
    ) {
        auto *button = new QPushButton(text, parent);
        button->setObjectName(QStringLiteral("navButton"));
        button->setCheckable(true);
        button->setChecked(active);
        button->setEnabled(enabled);
        button->setCursor(enabled ? Qt::PointingHandCursor : Qt::ArrowCursor);
        return button;
    }

    void select_navigation(QPushButton *selected) {
        for (auto *button : {portfolio_nav_, wallets_nav_, activity_nav_}) {
            button->setChecked(button == selected);
        }
    }

    QHBoxLayout *build_header(QWidget *parent) {
        auto *header = new QHBoxLayout;
        auto *titles = new QVBoxLayout;
        titles->setSpacing(3);
        auto *title = new QLabel(QStringLiteral("Portfolio"), parent);
        title->setObjectName(QStringLiteral("pageTitle"));
        auto *subtitle = new QLabel(
            QStringLiteral("Manage wallets created through the native C++ API"),
            parent
        );
        subtitle->setObjectName(QStringLiteral("pageSubtitle"));
        titles->addWidget(title);
        titles->addWidget(subtitle);
        header->addLayout(titles);
        header->addStretch();
        engine_badge_ = new QLabel(parent);
        engine_badge_->setObjectName(QStringLiteral("engineBadge"));
        update_provider_badge();
        header->addWidget(engine_badge_, 0, Qt::AlignVCenter);
        return header;
    }

    QWidget *build_portfolio_card(QWidget *parent) {
        auto *card = new QFrame(parent);
        card->setObjectName(QStringLiteral("balanceCard"));
        auto *layout = new QVBoxLayout(card);
        layout->setContentsMargins(25, 23, 25, 23);
        layout->setSpacing(10);

        auto *eyebrow = new QLabel(QStringLiteral("SELECTED WALLET BALANCE"), card);
        eyebrow->setObjectName(QStringLiteral("eyebrow"));
        layout->addWidget(eyebrow);
        balance_ = new QLabel(QStringLiteral("— TON"), card);
        balance_->setObjectName(QStringLiteral("balance"));
        layout->addWidget(balance_);
        balance_hint_ = new QLabel(
            QStringLiteral("Select a wallet to load its account and activity"),
            card
        );
        balance_hint_->setObjectName(QStringLiteral("balanceHint"));
        layout->addWidget(balance_hint_);
        layout->addStretch();

        auto *stats = new QHBoxLayout;
        stats->setSpacing(16);
        saved_count_ = add_stat(stats, QStringLiteral("0"), QStringLiteral("SAVED WALLETS"), card);
        add_divider(stats, card);
        add_stat(stats, QStringLiteral("2"), QStringLiteral("NETWORKS"), card);
        add_divider(stats, card);
        add_stat(stats, QStringLiteral("Wallet"), QStringLiteral("CONTRACT"), card);
        stats->addStretch();
        layout->addLayout(stats);
        layout->addSpacing(8);

        auto *actions = new QHBoxLayout;
        actions->setSpacing(9);
        auto *new_wallet = new QPushButton(QStringLiteral("+  New wallet"), card);
        new_wallet->setObjectName(QStringLiteral("primaryButton"));
        receive_button_ = new QPushButton(QStringLiteral("↓  Receive"), card);
        receive_button_->setObjectName(QStringLiteral("secondaryButton"));
        receive_button_->setEnabled(false);
        send_button_ = new QPushButton(QStringLiteral("↗  Send"), card);
        send_button_->setObjectName(QStringLiteral("secondaryButton"));
        send_button_->setEnabled(false);
        refresh_button_ = new QPushButton(QStringLiteral("↻  Refresh"), card);
        refresh_button_->setObjectName(QStringLiteral("secondaryButton"));
        refresh_button_->setEnabled(false);
        actions->addWidget(new_wallet);
        actions->addWidget(receive_button_);
        actions->addWidget(send_button_);
        actions->addWidget(refresh_button_);
        actions->addStretch();
        layout->addLayout(actions);

        connect(new_wallet, &QPushButton::clicked, this, [this] {
            record_id_->setFocus();
            record_id_->selectAll();
        });
        return card;
    }

    static QLabel *add_stat(
        QHBoxLayout *layout,
        const QString &value,
        const QString &label,
        QWidget *parent
    ) {
        auto *column = new QVBoxLayout;
        column->setSpacing(2);
        auto *value_label = new QLabel(value, parent);
        value_label->setObjectName(QStringLiteral("statValue"));
        auto *caption = new QLabel(label, parent);
        caption->setObjectName(QStringLiteral("statLabel"));
        column->addWidget(value_label);
        column->addWidget(caption);
        layout->addLayout(column);
        return value_label;
    }

    static void add_divider(QHBoxLayout *layout, QWidget *parent) {
        auto *divider = new QFrame(parent);
        divider->setObjectName(QStringLiteral("statDivider"));
        divider->setFixedSize(1, 34);
        layout->addWidget(divider);
    }

    QWidget *build_create_card(QWidget *parent) {
        auto *card = new QFrame(parent);
        card->setObjectName(QStringLiteral("card"));
        auto *layout = new QVBoxLayout(card);
        layout->setContentsMargins(23, 21, 23, 21);
        layout->setSpacing(9);

        auto *title = new QLabel(QStringLiteral("Create wallet"), card);
        title->setObjectName(QStringLiteral("cardTitle"));
        layout->addWidget(title);
        auto *subtitle = new QLabel(
            QStringLiteral("A new wallet and recovery phrase"),
            card
        );
        subtitle->setObjectName(QStringLiteral("muted"));
        layout->addWidget(subtitle);
        layout->addSpacing(6);

        auto *record_label = new QLabel(QStringLiteral("RECORD ID"), card);
        record_label->setObjectName(QStringLiteral("fieldLabel"));
        layout->addWidget(record_label);
        record_id_ = new QLineEdit(card);
        record_id_->setPlaceholderText(QStringLiteral("wallet-0001"));
        layout->addWidget(record_id_);

        auto *network_label = new QLabel(QStringLiteral("NETWORK"), card);
        network_label->setObjectName(QStringLiteral("fieldLabel"));
        layout->addWidget(network_label);
        network_selector_ = new QFrame(card);
        network_selector_->setObjectName(QStringLiteral("networkSelector"));
        auto *network_layout = new QHBoxLayout(network_selector_);
        network_layout->setContentsMargins(3, 3, 3, 3);
        network_layout->setSpacing(3);
        network_group_ = new QButtonGroup(this);
        network_group_->setExclusive(true);
        auto *testnet = new QPushButton(QStringLiteral("Testnet"), network_selector_);
        auto *mainnet = new QPushButton(QStringLiteral("Mainnet"), network_selector_);
        for (auto *option : {testnet, mainnet}) {
            option->setObjectName(QStringLiteral("networkOption"));
            option->setCheckable(true);
            option->setCursor(Qt::PointingHandCursor);
            network_layout->addWidget(option, 1);
        }
        network_group_->addButton(testnet, 0);
        network_group_->addButton(mainnet, 1);
        testnet->setChecked(true);
        layout->addWidget(network_selector_);

        create_button_ = new QPushButton(QStringLiteral("Create securely"), card);
        create_button_->setObjectName(QStringLiteral("primaryButton"));
        create_button_->setMinimumHeight(42);
        layout->addWidget(create_button_);
        status_ = new QLabel(QStringLiteral("Ready to create"), card);
        status_->setObjectName(QStringLiteral("inlineStatus"));
        status_->setAlignment(Qt::AlignCenter);
        layout->addWidget(status_);

        auto *warning = new QLabel(
            QStringLiteral("⚠  Demo: the recovery phrase is stored in plaintext"),
            card
        );
        warning->setObjectName(QStringLiteral("warningText"));
        warning->setWordWrap(true);
        layout->addWidget(warning);
        return card;
    }

    QWidget *build_wallets_card(QWidget *parent) {
        auto *card = new QFrame(parent);
        card->setObjectName(QStringLiteral("card"));
        auto *layout = new QVBoxLayout(card);
        layout->setContentsMargins(23, 20, 23, 22);
        layout->setSpacing(14);

        auto *header = new QHBoxLayout;
        auto *title = new QLabel(QStringLiteral("Your wallets"), card);
        title->setObjectName(QStringLiteral("cardTitle"));
        wallet_count_ = new QLabel(QStringLiteral("0 wallets"), card);
        wallet_count_->setObjectName(QStringLiteral("countBadge"));
        auto *reload = new QPushButton(QStringLiteral("↻  Reload"), card);
        reload->setObjectName(QStringLiteral("quietButton"));
        header->addWidget(title);
        header->addWidget(wallet_count_);
        header->addStretch();
        header->addWidget(reload);
        layout->addLayout(header);

        wallet_list_layout_ = new QVBoxLayout;
        wallet_list_layout_->setSpacing(9);
        layout->addLayout(wallet_list_layout_);
        layout->addStretch();

        connect(reload, &QPushButton::clicked, this, [this] { reload_wallets(); });
        return card;
    }

    QWidget *build_activity_card(QWidget *parent) {
        auto *card = new QFrame(parent);
        card->setObjectName(QStringLiteral("card"));
        auto *layout = new QVBoxLayout(card);
        layout->setContentsMargins(23, 20, 23, 22);
        layout->setSpacing(14);

        auto *header = new QHBoxLayout;
        auto *title = new QLabel(QStringLiteral("Activity"), card);
        title->setObjectName(QStringLiteral("cardTitle"));
        activity_status_ = new QLabel(QStringLiteral("Select a wallet"), card);
        activity_status_->setObjectName(QStringLiteral("countBadge"));
        load_more_button_ = new QPushButton(QStringLiteral("Load older"), card);
        load_more_button_->setObjectName(QStringLiteral("quietButton"));
        load_more_button_->setEnabled(false);
        header->addWidget(title);
        header->addWidget(activity_status_);
        header->addStretch();
        header->addWidget(load_more_button_);
        layout->addLayout(header);

        activity_list_layout_ = new QVBoxLayout;
        activity_list_layout_->setSpacing(9);
        layout->addLayout(activity_list_layout_);
        render_activity({});

        connect(load_more_button_, &QPushButton::clicked, this, [this] {
            start_wallet_update(true);
        });
        return card;
    }

    void start_create_wallet() {
        if (create_watcher_.isRunning()) {
            return;
        }

        const auto record_id = record_id_->text().trimmed();
        if (record_id.isEmpty()) {
            QMessageBox::warning(
                this,
                QStringLiteral("Missing record ID"),
                QStringLiteral("Enter a record ID before creating a wallet.")
            );
            record_id_->setFocus();
            return;
        }

        if (wallet_name_exists(record_id)) {
            app_log(
                AppLogLevel::Warning,
                QStringLiteral("lifecycle"),
                QStringLiteral("create rejected duplicate record_id=%1")
                    .arg(record_id)
            );
            status_->setText(QStringLiteral("This wallet name is already in use"));
            QMessageBox::warning(
                this,
                QStringLiteral("Wallet already exists"),
                QStringLiteral(
                    "A wallet named \"%1\" already exists. Choose a different "
                    "record ID."
                ).arg(record_id)
            );
            record_id_->setFocus();
            record_id_->selectAll();
            return;
        }

        CreateWalletRequest request {
            record_id.toStdString(),
            network_group_->checkedId() == 0 ? Network::kTestnet :
                                               Network::kMainnet,
        };

        app_log(
            AppLogLevel::Info,
            QStringLiteral("lifecycle"),
            QStringLiteral("create started record_id=%1 network=%2")
                .arg(
                    record_id,
                    request.network == Network::kMainnet ?
                        QStringLiteral("mainnet") : QStringLiteral("testnet")
                )
        );

        create_button_->setEnabled(false);
        create_button_->setText(QStringLiteral("Creating…"));
        record_id_->setEnabled(false);
        network_selector_->setEnabled(false);
        status_->setText(QStringLiteral("Generating keys through Wallet Engine…"));

        create_watcher_.setFuture(QtConcurrent::run(
            [lifecycle = lifecycle_, request = std::move(request)] {
                return create_wallet(lifecycle, request);
            }
        ));
    }

    void finish_create_wallet() {
        create_button_->setEnabled(true);
        create_button_->setText(QStringLiteral("Create securely"));
        record_id_->setEnabled(true);
        network_selector_->setEnabled(true);

        const auto result = create_watcher_.result();
        if (!result.wallet.has_value()) {
            app_log(
                AppLogLevel::Error,
                QStringLiteral("lifecycle"),
                QStringLiteral("create failed error=%1").arg(result.error)
            );
            status_->setText(QStringLiteral("Creation failed · try again"));
            QMessageBox::critical(
                this,
                QStringLiteral("Wallet creation failed"),
                result.error
            );
            return;
        }

        if (!append_wallet_metadata(result.wallet->descriptor)) {
            app_log(
                AppLogLevel::Error,
                QStringLiteral("storage"),
                QStringLiteral("wallet metadata append failed")
            );
            status_->setText(QStringLiteral("Could not save metadata"));
            QMessageBox::critical(
                this,
                QStringLiteral("Storage error"),
                QStringLiteral("Could not append public wallet metadata.")
            );
            return;
        }

        status_->setText(QStringLiteral("✓ Wallet created successfully"));
        record_id_->clear();
        const auto saved_wallet = saved_wallet_from(
            result.wallet->descriptor
        );
        app_log(
            AppLogLevel::Info,
            QStringLiteral("lifecycle"),
            QStringLiteral("create completed record_id=%1 address=%2")
                .arg(saved_wallet.record_id, shortened_address(saved_wallet.address))
        );
        reload_wallets();
        show_recovery_phrase(result.wallet.value());
        activate_wallet(saved_wallet);
    }

    void reload_wallets() {
        while (auto *item = wallet_list_layout_->takeAt(0)) {
            if (auto *widget = item->widget()) {
                widget->deleteLater();
            }
            delete item;
        }

        const auto wallets = load_wallet_metadata();
        const auto count = static_cast<qulonglong>(wallets.size());
        wallet_count_->setText(
            count == 1 ? QStringLiteral("1 wallet") :
                         QStringLiteral("%1 wallets").arg(count)
        );
        saved_count_->setText(QString::number(count));

        if (wallets.empty()) {
            auto *empty = new QFrame;
            empty->setObjectName(QStringLiteral("emptyState"));
            auto *empty_layout = new QVBoxLayout(empty);
            empty_layout->setContentsMargins(20, 22, 20, 22);
            empty_layout->setSpacing(4);
            auto *icon = new QLabel(QStringLiteral("◇"), empty);
            icon->setObjectName(QStringLiteral("emptyIcon"));
            icon->setAlignment(Qt::AlignCenter);
            auto *title = new QLabel(QStringLiteral("No wallets yet"), empty);
            title->setObjectName(QStringLiteral("emptyTitle"));
            title->setAlignment(Qt::AlignCenter);
            auto *hint = new QLabel(
                QStringLiteral("Create your first wallet to see it here"),
                empty
            );
            hint->setObjectName(QStringLiteral("muted"));
            hint->setAlignment(Qt::AlignCenter);
            empty_layout->addWidget(icon);
            empty_layout->addWidget(title);
            empty_layout->addWidget(hint);
            wallet_list_layout_->addWidget(empty);
            return;
        }

        for (const auto &wallet : wallets) {
            wallet_list_layout_->addWidget(build_wallet_row(wallet));
        }
    }

    QWidget *build_wallet_row(const SavedWallet &wallet) {
        auto *row = new QFrame;
        row->setObjectName(QStringLiteral("walletRow"));
        row->setProperty(
            "selected",
            active_wallet_.has_value() &&
                active_wallet_->record_id == wallet.record_id
        );
        auto *layout = new QHBoxLayout(row);
        layout->setContentsMargins(14, 12, 14, 12);
        layout->setSpacing(12);

        auto *icon = new QLabel(
            wallet.network == QStringLiteral("mainnet") ? QStringLiteral("M") :
                                                          QStringLiteral("T"),
            row
        );
        icon->setObjectName(QStringLiteral("walletIcon"));
        icon->setAlignment(Qt::AlignCenter);
        icon->setFixedSize(36, 36);
        layout->addWidget(icon);

        auto *identity = new QVBoxLayout;
        identity->setSpacing(3);
        auto *name_row = new QHBoxLayout;
        name_row->setSpacing(8);
        auto *name = new QLabel(wallet.record_id, row);
        name->setObjectName(QStringLiteral("walletName"));
        auto *network = new QLabel(wallet.network.toUpper(), row);
        network->setObjectName(QStringLiteral("networkBadge"));
        network->setProperty("network", wallet.network);
        name_row->addWidget(name);
        name_row->addWidget(network);
        name_row->addStretch();
        auto *address = new QLabel(shortened_address(wallet.address), row);
        address->setObjectName(QStringLiteral("address"));
        address->setToolTip(wallet.address);
        identity->addLayout(name_row);
        identity->addWidget(address);
        layout->addLayout(identity, 1);

        auto *open = new QPushButton(
            active_wallet_.has_value() &&
                    active_wallet_->record_id == wallet.record_id ?
                QStringLiteral("Selected") : QStringLiteral("Open"),
            row
        );
        open->setObjectName(QStringLiteral("quietButton"));
        open->setEnabled(
            (!active_wallet_.has_value() ||
             active_wallet_->record_id != wallet.record_id)
        );
        if (wallet.public_key.empty()) {
            open->setText(QStringLiteral("Upgrade"));
            open->setToolTip(
                QStringLiteral(
                    "Restore the public key from this demo wallet's stored "
                    "recovery phrase."
                )
            );
        }
        layout->addWidget(open);
        auto *copy = new QPushButton(QStringLiteral("Copy address"), row);
        copy->setObjectName(QStringLiteral("quietButton"));
        layout->addWidget(copy);
        connect(open, &QPushButton::clicked, this, [this, wallet] {
            if (wallet.public_key.empty()) {
                upgrade_legacy_wallet(wallet);
            } else {
                activate_wallet(wallet);
            }
        });
        connect(copy, &QPushButton::clicked, this, [this, address = wallet.address] {
            QGuiApplication::clipboard()->setText(address);
            status_->setText(QStringLiteral("✓ Address copied"));
            QTimer::singleShot(1800, this, [this] {
                if (!create_watcher_.isRunning()) {
                    status_->setText(QStringLiteral("Ready to create"));
                }
            });
        });
        return row;
    }

    void activate_wallet(const SavedWallet &wallet) {
        if (
            create_watcher_.isRunning() || update_watcher_.isRunning() ||
            preview_watcher_.isRunning() || send_watcher_.isRunning()
        ) {
            return;
        }
        if (wallet.public_key.empty()) {
            QMessageBox::information(
                this,
                QStringLiteral("Wallet metadata is incomplete"),
                QStringLiteral(
                    "This wallet was saved by an older version of the example "
                    "without its public key. Create a new wallet to enable "
                    "balance, activity, and transfers."
                )
            );
            return;
        }

        app_log(
            AppLogLevel::Info,
            QStringLiteral("wallet"),
            QStringLiteral("opening record_id=%1 network=%2 address=%3")
                .arg(
                    wallet.record_id,
                    wallet.network,
                    shortened_address(wallet.address)
                )
        );

        if (client_) {
            try {
                client_->shutdown();
            } catch (const std::exception &) {
                // The old demo session is discarded even if shutdown reports an error.
            }
            client_.reset();
        }

        const auto network = wallet.network == QStringLiteral("mainnet") ?
            Network::kMainnet : Network::kTestnet;
        const ProviderConfig providers {
            network == Network::kMainnet ?
                "https://toncenter.com" : "https://testnet.toncenter.com",
            15'000,
        };
        try {
            client_ = WalletClient::init(
                {
                    wallet.record_id.toStdString(),
                    wallet.address.toStdString(),
                    wallet.public_key,
                    ProtectedSecretRef {wallet.secret_ref.toStdString()},
                    network,
                    300,
                    60,
                    providers,
                },
                http_host_,
                platform_host_
            );
        } catch (const WalletClientError &error) {
            app_log(
                AppLogLevel::Error,
                QStringLiteral("wallet"),
                QStringLiteral("open failed error=%1")
                    .arg(describe_client_error(error))
            );
            QMessageBox::critical(
                this,
                QStringLiteral("Could not open wallet"),
                describe_client_error(error)
            );
            return;
        } catch (const std::exception &error) {
            app_log(
                AppLogLevel::Error,
                QStringLiteral("wallet"),
                QStringLiteral("open FFI failure error=%1")
                    .arg(QString::fromUtf8(error.what()))
            );
            QMessageBox::critical(
                this,
                QStringLiteral("Could not open wallet"),
                QString::fromUtf8(error.what())
            );
            return;
        }

        active_wallet_ = wallet;
        app_log(
            AppLogLevel::Info,
            QStringLiteral("wallet"),
            QStringLiteral("session opened record_id=%1").arg(wallet.record_id)
        );
        balance_->setText(QStringLiteral("— TON"));
        balance_hint_->setText(
            QStringLiteral("%1 · synchronizing…").arg(wallet.record_id)
        );
        receive_button_->setEnabled(true);
        send_button_->setEnabled(true);
        refresh_button_->setEnabled(true);
        activity_status_->setText(QStringLiteral("Refreshing…"));
        reload_wallets();
        start_wallet_update(false);
    }

    void upgrade_legacy_wallet(const SavedWallet &wallet) {
        app_log(
            AppLogLevel::Info,
            QStringLiteral("lifecycle"),
            QStringLiteral("legacy metadata upgrade started record_id=%1")
                .arg(wallet.record_id)
        );
        try {
            auto secret = platform_host_->read_protected_secret({
                ProtectedSecretRef {wallet.secret_ref.toStdString()},
                SecretAccessReason::kRevealRecoveryPhrase,
                "Upgrade legacy wallet metadata",
            });
            std::string phrase(secret.begin(), secret.end());
            std::istringstream stream(phrase);
            std::vector<std::string> words;
            for (std::string word; stream >> word;) {
                words.push_back(std::move(word));
            }
            std::fill(secret.begin(), secret.end(), 0);
            std::fill(phrase.begin(), phrase.end(), '\0');

            const auto descriptor = lifecycle_->import_wallet({
                wallet.record_id.toStdString(),
                wallet.network == QStringLiteral("mainnet") ?
                    Network::kMainnet : Network::kTestnet,
                std::move(words),
            });
            if (!append_wallet_metadata(descriptor)) {
                QMessageBox::critical(
                    this,
                    QStringLiteral("Could not upgrade wallet"),
                    QStringLiteral("Could not update public wallet metadata.")
                );
                return;
            }
            const auto upgraded = saved_wallet_from(descriptor);
            app_log(
                AppLogLevel::Info,
                QStringLiteral("lifecycle"),
                QStringLiteral("legacy metadata upgrade completed record_id=%1")
                    .arg(wallet.record_id)
            );
            reload_wallets();
            activate_wallet(upgraded);
        } catch (const WalletLifecycleError &error) {
            app_log(
                AppLogLevel::Error,
                QStringLiteral("lifecycle"),
                QStringLiteral("legacy metadata upgrade failed error=%1")
                    .arg(describe_lifecycle_error(error))
            );
            QMessageBox::critical(
                this,
                QStringLiteral("Could not upgrade wallet"),
                describe_lifecycle_error(error)
            );
        } catch (const ProtectedSecretHostError &error) {
            QMessageBox::critical(
                this,
                QStringLiteral("Could not upgrade wallet"),
                QString::fromUtf8(error.what())
            );
        } catch (const std::exception &error) {
            QMessageBox::critical(
                this,
                QStringLiteral("Could not upgrade wallet"),
                QString::fromUtf8(error.what())
            );
        }
    }

    void start_wallet_update(bool load_more) {
        if (!client_ || update_watcher_.isRunning()) {
            app_log(
                AppLogLevel::Warning,
                QStringLiteral("wallet"),
                QStringLiteral("update ignored client=%1 operation_running=%2")
                    .arg(client_ ? QStringLiteral("available") :
                                   QStringLiteral("missing"))
                    .arg(update_watcher_.isRunning() ? QStringLiteral("yes") :
                                                       QStringLiteral("no"))
            );
            return;
        }
        loading_more_ = load_more;
        app_log(
            AppLogLevel::Info,
            QStringLiteral("wallet"),
            QStringLiteral("%1 started record_id=%2")
                .arg(
                    load_more ? QStringLiteral("activity pagination") :
                                QStringLiteral("refresh"),
                    active_wallet_->record_id
                )
        );
        refresh_button_->setEnabled(false);
        load_more_button_->setEnabled(false);
        if (load_more) {
            load_more_button_->setText(QStringLiteral("Loading…"));
        } else {
            refresh_button_->setText(QStringLiteral("↻  Refreshing…"));
        }
        activity_status_->setText(
            load_more ? QStringLiteral("Loading older…") :
                        QStringLiteral("Refreshing…")
        );
        if (!load_more) {
            balance_hint_->setText(
                QStringLiteral("%1 · refreshing account…")
                    .arg(active_wallet_->record_id)
            );
        }
        update_watcher_.setFuture(QtConcurrent::run(
            [client = client_, load_more] {
                return update_wallet(client, load_more);
            }
        ));
    }

    void finish_wallet_update() {
        refresh_button_->setEnabled(client_ != nullptr);
        refresh_button_->setText(QStringLiteral("↻  Refresh"));
        const auto result = update_watcher_.result();
        if (!result.update.has_value()) {
            if (loading_more_) {
                load_more_button_->setText(QStringLiteral("Retry older"));
                load_more_button_->setEnabled(client_ != nullptr);
            }
            app_log(
                AppLogLevel::Error,
                QStringLiteral("wallet"),
                QStringLiteral("%1 failed error=%2")
                    .arg(
                        loading_more_ ? QStringLiteral("activity pagination") :
                                        QStringLiteral("refresh"),
                        result.error
                    )
            );
            activity_status_->setText(QStringLiteral("Update failed"));
            if (active_wallet_) {
                balance_hint_->setText(
                    QStringLiteral("%1 · could not refresh")
                        .arg(active_wallet_->record_id)
                );
            }
            QMessageBox::warning(
                this,
                loading_more_ ? QStringLiteral("Could not load activity") :
                                QStringLiteral("Could not refresh wallet"),
                result.error
            );
            return;
        }
        app_log(
            result.update->outcome == WalletOperationOutcome::kCompleted ?
                AppLogLevel::Info : AppLogLevel::Warning,
            QStringLiteral("wallet"),
            QStringLiteral(
                "%1 finished outcome=%2 activity_items=%3 added=%4 account=%5"
            ).arg(
                loading_more_ ? QStringLiteral("activity pagination") :
                                QStringLiteral("refresh"),
                update_outcome_text(result.update->outcome)
            ).arg(
                static_cast<qulonglong>(result.update->snapshot.activity.size())
            ).arg(result.update->activity_items_added)
             .arg(result.update->snapshot.account.has_value() ?
                QStringLiteral("available") : QStringLiteral("unavailable"))
        );
        apply_snapshot(result.update->snapshot);
        if (
            loading_more_ &&
            result.update->outcome == WalletOperationOutcome::kCompleted
        ) {
            if (result.update->activity_items_added == 0) {
                activity_status_->setText(QStringLiteral("No older transactions found"));
            } else {
                activity_status_->setText(
                    QStringLiteral("%1 older loaded · %2 total")
                        .arg(result.update->activity_items_added)
                        .arg(static_cast<qulonglong>(
                            result.update->snapshot.activity.size()
                        ))
                );
            }
        } else if (
            loading_more_ &&
            result.update->outcome == WalletOperationOutcome::kSkipped
        ) {
            activity_status_->setText(QStringLiteral("All available activity is loaded"));
        }
        if (
            result.update->outcome == WalletOperationOutcome::kFailed ||
            result.update->outcome ==
                WalletOperationOutcome::kPartiallyCompleted
        ) {
            QMessageBox::warning(
                this,
                result.update->outcome == WalletOperationOutcome::kFailed ?
                    QStringLiteral("Wallet update failed") :
                    QStringLiteral("Wallet partially updated"),
                describe_update_errors(result.update.value(), loading_more_)
            );
        }
    }

    void apply_snapshot(const WalletSnapshot &snapshot) {
        if (snapshot.account.has_value()) {
            balance_->setText(
                format_ton(snapshot.account->balance_nanograms) +
                QStringLiteral(" TON")
            );
            const auto synchronized = QDateTime::fromSecsSinceEpoch(
                static_cast<qint64>(snapshot.account->sync_utime)
            ).toLocalTime();
            balance_hint_->setText(
                QStringLiteral("%1 · %2 · updated %3")
                    .arg(
                        active_wallet_->record_id,
                        account_status_text(snapshot.account->status),
                        synchronized.toString(QStringLiteral("dd MMM, HH:mm"))
                    )
            );
            balance_hint_->setToolTip({});
        } else {
            balance_->setText(QStringLiteral("— TON"));
            const auto account_error = snapshot.account_resource.error.has_value() ?
                describe_domain_error(snapshot.account_resource.error.value()) :
                QStringLiteral("Account data is unavailable.");
            balance_hint_->setText(
                QStringLiteral("%1 · %2")
                    .arg(
                        active_wallet_->record_id,
                        account_error.section('\n', 0, 0)
                    )
            );
            balance_hint_->setToolTip(account_error);
        }
        render_activity(snapshot.activity);
        const auto &activity_resource = loading_more_ ?
            snapshot.activity_pagination_resource : snapshot.activity_resource;
        if (
            activity_resource.phase == ResourcePhase::kFailed &&
            activity_resource.error.has_value()
        ) {
            activity_status_->setText(QStringLiteral("Activity unavailable"));
            activity_status_->setToolTip(
                describe_domain_error(activity_resource.error.value())
            );
        } else {
            activity_status_->setText(
                snapshot.activity.empty() ? QStringLiteral("No transactions") :
                    QStringLiteral("%1 transactions")
                        .arg(static_cast<qulonglong>(snapshot.activity.size()))
            );
            activity_status_->setToolTip({});
        }
        load_more_button_->setText(
            snapshot.activity_has_more ? QStringLiteral("Load older") :
                                         QStringLiteral("All loaded")
        );
        load_more_button_->setEnabled(snapshot.activity_has_more);
    }

    void render_activity(const std::vector<ActivityItem> &activity) {
        if (!activity_list_layout_) {
            return;
        }
        while (auto *item = activity_list_layout_->takeAt(0)) {
            if (auto *widget = item->widget()) {
                widget->deleteLater();
            }
            delete item;
        }
        if (activity.empty()) {
            auto *empty = new QLabel(
                active_wallet_.has_value() ?
                    QStringLiteral("No transactions found for this wallet.") :
                    QStringLiteral("Open a wallet to view its transaction history.")
            );
            empty->setObjectName(QStringLiteral("muted"));
            empty->setAlignment(Qt::AlignCenter);
            empty->setMinimumHeight(54);
            activity_list_layout_->addWidget(empty);
            return;
        }
        for (const auto &item : activity) {
            auto *row = new QFrame;
            row->setObjectName(QStringLiteral("activityRow"));
            auto *layout = new QHBoxLayout(row);
            layout->setContentsMargins(13, 11, 13, 11);
            layout->setSpacing(11);
            const bool received = item.direction == ActivityDirection::kReceived;
            auto *icon = new QLabel(
                received ? QStringLiteral("↓") : QStringLiteral("↗"),
                row
            );
            icon->setObjectName(
                received ? QStringLiteral("activityIconReceived") :
                           QStringLiteral("activityIconSent")
            );
            icon->setAlignment(Qt::AlignCenter);
            icon->setFixedSize(34, 34);
            layout->addWidget(icon);

            auto *details = new QVBoxLayout;
            details->setSpacing(3);
            auto *title = new QLabel(
                received ? QStringLiteral("Received") : QStringLiteral("Sent"),
                row
            );
            title->setObjectName(QStringLiteral("activityTitle"));
            const auto counterparty = item.counterparty.has_value() ?
                shortened_address(QString::fromStdString(*item.counterparty)) :
                QStringLiteral("Unknown counterparty");
            auto *subtitle = new QLabel(
                QStringLiteral("%1 · %2")
                    .arg(
                        counterparty,
                        QDateTime::fromSecsSinceEpoch(
                            static_cast<qint64>(item.timestamp)
                        ).toLocalTime().toString(
                            QStringLiteral("dd MMM yyyy, HH:mm")
                        )
                    ),
                row
            );
            subtitle->setObjectName(QStringLiteral("address"));
            subtitle->setToolTip(
                QString::fromStdString(item.transaction_hash)
            );
            details->addWidget(title);
            details->addWidget(subtitle);
            layout->addLayout(details, 1);

            auto *amount = new QLabel(
                (received ? QStringLiteral("+ ") : QStringLiteral("− ")) +
                    format_ton(item.amount_nanograms, 9) + QStringLiteral(" TON"),
                row
            );
            amount->setObjectName(
                received ? QStringLiteral("activityAmountReceived") :
                           QStringLiteral("activityAmountSent")
            );
            layout->addWidget(amount);
            auto *details_button = new QPushButton(
                QStringLiteral("Details"),
                row
            );
            details_button->setObjectName(QStringLiteral("quietButton"));
            layout->addWidget(details_button);
            connect(
                details_button,
                &QPushButton::clicked,
                this,
                [this, item] { show_transaction_details(item); }
            );
            activity_list_layout_->addWidget(row);
        }
    }

    void show_transaction_details(const ActivityItem &item) {
        const bool received = item.direction == ActivityDirection::kReceived;
        app_log(
            AppLogLevel::Info,
            QStringLiteral("activity"),
            QStringLiteral("transaction details opened hash=%1")
                .arg(shortened_address(
                    QString::fromStdString(item.transaction_hash)
                ))
        );

        QDialog dialog(this);
        dialog.setObjectName(QStringLiteral("actionDialog"));
        dialog.setWindowTitle(QStringLiteral("Transaction details"));
        dialog.setMinimumWidth(700);
        auto *layout = new QVBoxLayout(&dialog);
        layout->setContentsMargins(28, 26, 28, 26);
        layout->setSpacing(12);

        auto *header = new QHBoxLayout;
        auto *icon = new QLabel(
            received ? QStringLiteral("↓") : QStringLiteral("↗"),
            &dialog
        );
        icon->setObjectName(
            received ? QStringLiteral("activityIconReceived") :
                       QStringLiteral("activityIconSent")
        );
        icon->setAlignment(Qt::AlignCenter);
        icon->setFixedSize(34, 34);
        auto *titles = new QVBoxLayout;
        titles->setSpacing(2);
        auto *title = new QLabel(
            received ? QStringLiteral("Received TON") : QStringLiteral("Sent TON"),
            &dialog
        );
        title->setObjectName(QStringLiteral("dialogTitle"));
        auto *status = new QLabel(QStringLiteral("●  Confirmed activity"), &dialog);
        status->setObjectName(QStringLiteral("engineBadge"));
        titles->addWidget(title);
        titles->addWidget(status, 0, Qt::AlignLeft);
        header->addWidget(icon);
        header->addLayout(titles);
        header->addStretch();
        layout->addLayout(header);

        auto *amount = new QLabel(
            (received ? QStringLiteral("+ ") : QStringLiteral("− ")) +
                format_ton(item.amount_nanograms, 9) + QStringLiteral(" TON"),
            &dialog
        );
        amount->setObjectName(QStringLiteral("transactionAmount"));
        layout->addWidget(amount);

        auto *details = new QFrame(&dialog);
        details->setObjectName(QStringLiteral("detailsCard"));
        auto *grid = new QGridLayout(details);
        grid->setContentsMargins(17, 15, 17, 15);
        grid->setHorizontalSpacing(18);
        grid->setVerticalSpacing(13);
        grid->setColumnStretch(1, 1);
        int row = 0;
        const auto add_detail = [details, grid, &row](
            const QString &name,
            const QString &value
        ) {
            auto *label = new QLabel(name, details);
            label->setObjectName(QStringLiteral("fieldLabel"));
            label->setAlignment(Qt::AlignTop | Qt::AlignLeft);
            auto *content = new QLabel(value, details);
            content->setObjectName(QStringLiteral("detailValue"));
            content->setTextInteractionFlags(Qt::TextSelectableByMouse);
            content->setWordWrap(true);
            grid->addWidget(label, row, 0);
            grid->addWidget(content, row, 1);
            ++row;
        };
        add_detail(
            QStringLiteral("DIRECTION"),
            received ? QStringLiteral("Incoming") : QStringLiteral("Outgoing")
        );
        add_detail(
            QStringLiteral("DATE"),
            QDateTime::fromSecsSinceEpoch(static_cast<qint64>(item.timestamp))
                .toLocalTime()
                .toString(QStringLiteral("dd MMMM yyyy, HH:mm:ss t"))
        );
        add_detail(
            QStringLiteral("NETWORK"),
            active_wallet_.has_value() ? active_wallet_->network.toUpper() :
                                         QStringLiteral("Unknown")
        );
        add_detail(
            QStringLiteral("COUNTERPARTY"),
            item.counterparty.has_value() ?
                QString::fromStdString(item.counterparty.value()) :
                QStringLiteral("Not supplied by the provider")
        );
        add_detail(
            QStringLiteral("TRANSACTION HASH"),
            QString::fromStdString(item.transaction_hash)
        );
        add_detail(
            QStringLiteral("LOGICAL TIME"),
            QString::fromStdString(item.logical_time)
        );
        layout->addWidget(details);

        auto *actions = new QHBoxLayout;
        auto *copy_counterparty = new QPushButton(
            QStringLiteral("Copy address"),
            &dialog
        );
        copy_counterparty->setObjectName(QStringLiteral("secondaryButton"));
        copy_counterparty->setEnabled(item.counterparty.has_value());
        auto *copy_hash = new QPushButton(QStringLiteral("Copy hash"), &dialog);
        copy_hash->setObjectName(QStringLiteral("secondaryButton"));
        auto *close = new QPushButton(QStringLiteral("Close"), &dialog);
        close->setObjectName(QStringLiteral("primaryButton"));
        actions->addWidget(copy_counterparty);
        actions->addWidget(copy_hash);
        actions->addStretch();
        actions->addWidget(close);
        layout->addLayout(actions);
        connect(
            copy_counterparty,
            &QPushButton::clicked,
            &dialog,
            [item] {
                if (item.counterparty.has_value()) {
                    QGuiApplication::clipboard()->setText(
                        QString::fromStdString(item.counterparty.value())
                    );
                }
            }
        );
        connect(copy_hash, &QPushButton::clicked, &dialog, [item] {
            QGuiApplication::clipboard()->setText(
                QString::fromStdString(item.transaction_hash)
            );
        });
        connect(close, &QPushButton::clicked, &dialog, &QDialog::accept);
        dialog.exec();
    }

    void update_provider_badge() {
        if (!engine_badge_) {
            return;
        }
        engine_badge_->setText(
            http_host_->has_api_key() ?
                QStringLiteral("●  Toncenter key active") :
                QStringLiteral("●  Public Toncenter")
        );
    }

    void show_toncenter_settings() {
        QDialog dialog(this);
        dialog.setObjectName(QStringLiteral("actionDialog"));
        dialog.setWindowTitle(QStringLiteral("Toncenter settings"));
        dialog.setMinimumWidth(610);
        auto *layout = new QVBoxLayout(&dialog);
        layout->setContentsMargins(28, 26, 28, 26);
        layout->setSpacing(12);

        auto *title = new QLabel(QStringLiteral("Toncenter API key"), &dialog);
        title->setObjectName(QStringLiteral("dialogTitle"));
        layout->addWidget(title);
        auto *subtitle = new QLabel(
            QStringLiteral(
                "The key increases provider limits for balance, activity, "
                "emulation, and send requests."
            ),
            &dialog
        );
        subtitle->setObjectName(QStringLiteral("muted"));
        subtitle->setWordWrap(true);
        layout->addWidget(subtitle);

        auto *current = new QLabel(
            http_host_->has_api_key() ?
                QStringLiteral("✓ An API key is configured") :
                QStringLiteral("No API key configured · public endpoint mode"),
            &dialog
        );
        current->setObjectName(QStringLiteral("engineBadge"));
        layout->addWidget(current, 0, Qt::AlignLeft);
        layout->addSpacing(5);

        auto *key_label = new QLabel(QStringLiteral("NEW API KEY"), &dialog);
        key_label->setObjectName(QStringLiteral("fieldLabel"));
        layout->addWidget(key_label);
        auto *key = new QLineEdit(&dialog);
        key->setEchoMode(QLineEdit::Password);
        key->setMaxLength(max_toncenter_api_key_bytes);
        key->setPlaceholderText(
            http_host_->has_api_key() ?
                QStringLiteral("Enter a replacement key") :
                QStringLiteral("Paste Toncenter API key")
        );
        layout->addWidget(key);
        auto *show_key = new QCheckBox(QStringLiteral("Show API key"), &dialog);
        layout->addWidget(show_key);

        auto *notice = new QFrame(&dialog);
        notice->setObjectName(QStringLiteral("warningCard"));
        auto *notice_layout = new QHBoxLayout(notice);
        notice_layout->setContentsMargins(13, 10, 13, 10);
        auto *notice_text = new QLabel(
            QStringLiteral(
                "The key is saved locally with owner-only file permissions and "
                "is attached only to official Toncenter HTTPS origins. This "
                "demo does not use the operating-system keychain."
            ),
            notice
        );
        notice_text->setObjectName(QStringLiteral("dialogWarning"));
        notice_text->setWordWrap(true);
        notice_layout->addWidget(notice_text);
        layout->addWidget(notice);

        auto *actions = new QHBoxLayout;
        auto *clear = new QPushButton(QStringLiteral("Clear key"), &dialog);
        clear->setObjectName(QStringLiteral("quietButton"));
        clear->setEnabled(http_host_->has_api_key());
        auto *cancel = new QPushButton(QStringLiteral("Cancel"), &dialog);
        cancel->setObjectName(QStringLiteral("secondaryButton"));
        auto *apply = new QPushButton(QStringLiteral("Use API key"), &dialog);
        apply->setObjectName(QStringLiteral("primaryButton"));
        actions->addWidget(clear);
        actions->addStretch();
        actions->addWidget(cancel);
        actions->addWidget(apply);
        layout->addLayout(actions);

        connect(show_key, &QCheckBox::toggled, key, [key](bool visible) {
            key->setEchoMode(visible ? QLineEdit::Normal : QLineEdit::Password);
        });
        connect(cancel, &QPushButton::clicked, &dialog, &QDialog::reject);
        connect(clear, &QPushButton::clicked, &dialog, [this, &dialog] {
            QString error;
            if (!clear_persisted_toncenter_api_key(error)) {
                app_log(
                    AppLogLevel::Error,
                    QStringLiteral("settings"),
                    QStringLiteral("could not clear persisted Toncenter API key")
                );
                QMessageBox::critical(
                    &dialog,
                    QStringLiteral("Could not clear API key"),
                    error
                );
                return;
            }
            http_host_->set_api_key({});
            update_provider_badge();
            dialog.accept();
        });
        connect(apply, &QPushButton::clicked, &dialog, [this, &dialog, key] {
            const auto value = key->text().trimmed();
            if (value.isEmpty()) {
                QMessageBox::warning(
                    &dialog,
                    QStringLiteral("Missing API key"),
                    QStringLiteral("Paste an API key or choose Clear key.")
                );
                key->setFocus();
                return;
            }
            QString error;
            if (!persist_toncenter_api_key(value, error)) {
                app_log(
                    AppLogLevel::Error,
                    QStringLiteral("settings"),
                    QStringLiteral("could not persist Toncenter API key")
                );
                QMessageBox::critical(
                    &dialog,
                    QStringLiteral("Could not save API key"),
                    error
                );
                return;
            }
            http_host_->set_api_key(value);
            key->clear();
            update_provider_badge();
            dialog.accept();
        });
        dialog.exec();
    }

    void show_logs_dialog() {
        app_log(
            AppLogLevel::Info,
            QStringLiteral("ui"),
            QStringLiteral("log viewer opened")
        );
        QDialog dialog(this);
        dialog.setObjectName(QStringLiteral("actionDialog"));
        dialog.setWindowTitle(QStringLiteral("Wallet Engine logs"));
        dialog.resize(900, 620);
        auto *layout = new QVBoxLayout(&dialog);
        layout->setContentsMargins(24, 22, 24, 22);
        layout->setSpacing(10);

        auto *title = new QLabel(QStringLiteral("Application logs"), &dialog);
        title->setObjectName(QStringLiteral("dialogTitle"));
        layout->addWidget(title);
        auto *path = new QLabel(app_log_file_path(), &dialog);
        path->setObjectName(QStringLiteral("address"));
        path->setTextInteractionFlags(Qt::TextSelectableByMouse);
        layout->addWidget(path);

        auto *view = new QPlainTextEdit(&dialog);
        view->setObjectName(QStringLiteral("logView"));
        view->setReadOnly(true);
        view->setMaximumBlockCount(10'000);
        layout->addWidget(view, 1);

        const auto reload = [view] {
            QFile file(app_log_file_path());
            if (!file.open(QIODevice::ReadOnly | QIODevice::Text)) {
                view->setPlainText(QStringLiteral("The log file is not available."));
                return;
            }
            view->setPlainText(QString::fromUtf8(file.readAll()));
            view->moveCursor(QTextCursor::End);
        };
        reload();

        auto *actions = new QHBoxLayout;
        auto *reload_button = new QPushButton(QStringLiteral("↻  Reload"), &dialog);
        reload_button->setObjectName(QStringLiteral("secondaryButton"));
        auto *copy = new QPushButton(QStringLiteral("Copy logs"), &dialog);
        copy->setObjectName(QStringLiteral("secondaryButton"));
        auto *close = new QPushButton(QStringLiteral("Close"), &dialog);
        close->setObjectName(QStringLiteral("primaryButton"));
        actions->addWidget(reload_button);
        actions->addWidget(copy);
        actions->addStretch();
        actions->addWidget(close);
        layout->addLayout(actions);
        connect(reload_button, &QPushButton::clicked, &dialog, reload);
        connect(copy, &QPushButton::clicked, &dialog, [view] {
            QGuiApplication::clipboard()->setText(view->toPlainText());
        });
        connect(close, &QPushButton::clicked, &dialog, &QDialog::accept);
        dialog.exec();
    }

    void show_receive_dialog() {
        if (!active_wallet_) {
            return;
        }
        app_log(
            AppLogLevel::Info,
            QStringLiteral("receive"),
            QStringLiteral("receive address opened record_id=%1")
                .arg(active_wallet_->record_id)
        );
        QDialog dialog(this);
        dialog.setObjectName(QStringLiteral("actionDialog"));
        dialog.setWindowTitle(QStringLiteral("Receive TON"));
        dialog.setMinimumWidth(600);
        auto *layout = new QVBoxLayout(&dialog);
        layout->setContentsMargins(28, 26, 28, 26);
        layout->setSpacing(13);

        auto *title = new QLabel(QStringLiteral("Receive TON"), &dialog);
        title->setObjectName(QStringLiteral("dialogTitle"));
        layout->addWidget(title);
        auto *subtitle = new QLabel(
            QStringLiteral(
                "Share this address only for the selected %1 wallet."
            ).arg(active_wallet_->network.toUpper()),
            &dialog
        );
        subtitle->setObjectName(QStringLiteral("muted"));
        layout->addWidget(subtitle);

        auto *address_label = new QLabel(QStringLiteral("WALLET ADDRESS"), &dialog);
        address_label->setObjectName(QStringLiteral("fieldLabel"));
        layout->addWidget(address_label);
        auto *address = new QLineEdit(active_wallet_->address, &dialog);
        address->setReadOnly(true);
        address->setCursorPosition(0);
        layout->addWidget(address);

        auto *actions = new QHBoxLayout;
        auto *close = new QPushButton(QStringLiteral("Close"), &dialog);
        close->setObjectName(QStringLiteral("secondaryButton"));
        auto *copy = new QPushButton(QStringLiteral("Copy address"), &dialog);
        copy->setObjectName(QStringLiteral("primaryButton"));
        actions->addStretch();
        actions->addWidget(close);
        actions->addWidget(copy);
        layout->addLayout(actions);
        connect(close, &QPushButton::clicked, &dialog, &QDialog::reject);
        connect(copy, &QPushButton::clicked, &dialog, [this, &dialog] {
            QGuiApplication::clipboard()->setText(active_wallet_->address);
            dialog.accept();
        });
        dialog.exec();
    }

    void show_send_dialog() {
        if (!client_ || preview_watcher_.isRunning() || send_watcher_.isRunning()) {
            return;
        }
        QDialog dialog(this);
        dialog.setObjectName(QStringLiteral("actionDialog"));
        dialog.setWindowTitle(QStringLiteral("Send TON"));
        dialog.setMinimumWidth(620);
        auto *layout = new QVBoxLayout(&dialog);
        layout->setContentsMargins(28, 26, 28, 26);
        layout->setSpacing(11);

        auto *title = new QLabel(QStringLiteral("Send TON"), &dialog);
        title->setObjectName(QStringLiteral("dialogTitle"));
        layout->addWidget(title);
        auto *subtitle = new QLabel(
            QStringLiteral(
                "The transfer will be emulated before the wallet unlocks its secret."
            ),
            &dialog
        );
        subtitle->setObjectName(QStringLiteral("muted"));
        subtitle->setWordWrap(true);
        layout->addWidget(subtitle);
        layout->addSpacing(5);

        auto *destination_label = new QLabel(
            QStringLiteral("DESTINATION ADDRESS"),
            &dialog
        );
        destination_label->setObjectName(QStringLiteral("fieldLabel"));
        layout->addWidget(destination_label);
        auto *destination = new QLineEdit(&dialog);
        destination->setPlaceholderText(QStringLiteral("EQ… or UQ…"));
        layout->addWidget(destination);

        auto *amount_label = new QLabel(QStringLiteral("AMOUNT"), &dialog);
        amount_label->setObjectName(QStringLiteral("fieldLabel"));
        layout->addWidget(amount_label);
        auto *amount = new QLineEdit(&dialog);
        amount->setPlaceholderText(QStringLiteral("0.1 TON"));
        layout->addWidget(amount);

        auto *comment_label = new QLabel(
            QStringLiteral("COMMENT · OPTIONAL"),
            &dialog
        );
        comment_label->setObjectName(QStringLiteral("fieldLabel"));
        layout->addWidget(comment_label);
        auto *comment = new QLineEdit(&dialog);
        comment->setPlaceholderText(QStringLiteral("Message for the recipient"));
        comment->setMaxLength(120);
        layout->addWidget(comment);

        auto *actions = new QHBoxLayout;
        auto *cancel = new QPushButton(QStringLiteral("Cancel"), &dialog);
        cancel->setObjectName(QStringLiteral("secondaryButton"));
        auto *preview = new QPushButton(QStringLiteral("Review transfer"), &dialog);
        preview->setObjectName(QStringLiteral("primaryButton"));
        cancel->setAutoDefault(false);
        preview->setAutoDefault(true);
        preview->setDefault(true);
        actions->addStretch();
        actions->addWidget(cancel);
        actions->addWidget(preview);
        layout->addLayout(actions);
        connect(cancel, &QPushButton::clicked, &dialog, &QDialog::reject);
        std::optional<TransferDraft> reviewed_transfer;
        connect(
            preview,
            &QPushButton::clicked,
            &dialog,
            [
                &dialog,
                destination,
                amount,
                comment,
                &reviewed_transfer
            ] {
                const auto destination_value = destination->text().trimmed();
                if (destination_value.isEmpty()) {
                    QMessageBox::warning(
                        &dialog,
                        QStringLiteral("Missing destination"),
                        QStringLiteral("Enter a TON destination address.")
                    );
                    destination->setFocus();
                    return;
                }
                QString amount_error;
                const auto nanograms = parse_ton_amount(
                    amount->text(),
                    amount_error
                );
                if (!nanograms.has_value()) {
                    QMessageBox::warning(
                        &dialog,
                        QStringLiteral("Invalid amount"),
                        amount_error
                    );
                    amount->setFocus();
                    amount->selectAll();
                    return;
                }
                const auto comment_value = comment->text().trimmed();
                reviewed_transfer = TransferDraft {
                    destination_value.toStdString(),
                    nanograms.value(),
                    comment_value.isEmpty() ? std::nullopt :
                                              std::optional<std::string>(
                                                  comment_value.toStdString()
                                              ),
                };
                app_log(
                    AppLogLevel::Info,
                    QStringLiteral("send"),
                    QStringLiteral("send form submitted for review")
                );
                dialog.done(QDialog::Accepted);
            }
        );

        const auto dialog_result = dialog.exec();
        if (!reviewed_transfer.has_value()) {
            app_log(
                AppLogLevel::Info,
                QStringLiteral("send"),
                QStringLiteral("send form dismissed result=%1")
                    .arg(dialog_result)
            );
            return;
        }
        pending_transfer_ = std::move(reviewed_transfer.value());
        const auto destination_value = QString::fromStdString(
            pending_transfer_->destination
        );
        app_log(
            AppLogLevel::Info,
            QStringLiteral("send"),
            QStringLiteral("preview started amount=%1 TON destination=%2 comment=%3")
                .arg(
                    format_ton(pending_transfer_->amount_nanograms, 9),
                    shortened_address(destination_value),
                    pending_transfer_->comment.has_value() ?
                        QStringLiteral("yes") : QStringLiteral("no")
                )
        );
        send_button_->setEnabled(false);
        refresh_button_->setEnabled(false);
        activity_status_->setText(QStringLiteral("Checking transfer…"));
        preview_watcher_.setFuture(QtConcurrent::run(
            [client = client_, draft = pending_transfer_.value()] {
                return preview_transfer(client, draft);
            }
        ));
    }

    void finish_send_preview() {
        const auto result = preview_watcher_.result();
        if (!result.preview.has_value() || !pending_transfer_.has_value()) {
            app_log(
                AppLogLevel::Error,
                QStringLiteral("send"),
                QStringLiteral("preview failed error=%1").arg(result.error)
            );
            send_button_->setEnabled(client_ != nullptr);
            refresh_button_->setEnabled(client_ != nullptr);
            activity_status_->setText(QStringLiteral("Transfer check failed"));
            QMessageBox::critical(
                this,
                QStringLiteral("Could not review transfer"),
                result.error
            );
            pending_transfer_.reset();
            return;
        }
        if (!result.preview->emulation.trace_succeeded) {
            app_log(
                AppLogLevel::Warning,
                QStringLiteral("send"),
                QStringLiteral("preview rejected by emulation")
            );
            send_button_->setEnabled(true);
            refresh_button_->setEnabled(true);
            activity_status_->setText(QStringLiteral("Transfer rejected"));
            QMessageBox::critical(
                this,
                QStringLiteral("Emulation rejected the transfer"),
                QStringLiteral(
                    "The emulated transaction did not complete successfully. "
                    "Nothing was signed or submitted."
                )
            );
            pending_transfer_.reset();
            return;
        }

        const auto &preview = result.preview.value();
        app_log(
            AppLogLevel::Info,
            QStringLiteral("send"),
            QStringLiteral(
                "preview completed wallet_fee=%1 TON trace_transactions=%2 incomplete=%3"
            ).arg(
                format_ton(preview.emulation.wallet_fees_nanograms, 9)
            ).arg(preview.emulation.transaction_count)
             .arg(preview.emulation.is_incomplete ? QStringLiteral("yes") :
                                                    QStringLiteral("no"))
        );
        QString details = QStringLiteral(
            "Send %1 TON\n\nTo: %2\nEstimated wallet fee: %3 TON\n"
            "Trace transactions: %4"
        ).arg(
            format_ton(pending_transfer_->amount_nanograms, 9),
            shortened_address(QString::fromStdString(pending_transfer_->destination)),
            format_ton(preview.emulation.wallet_fees_nanograms, 9),
            QString::number(preview.emulation.transaction_count)
        );
        if (preview.emulation.is_incomplete) {
            details += QStringLiteral(
                "\n\nThe provider reports unresolved messages in this preview."
            );
        }
        const auto choice = QMessageBox::question(
            this,
            QStringLiteral("Confirm transfer"),
            details,
            QMessageBox::Yes | QMessageBox::No,
            QMessageBox::No
        );
        if (choice != QMessageBox::Yes) {
            app_log(
                AppLogLevel::Info,
                QStringLiteral("send"),
                QStringLiteral("send cancelled at confirmation")
            );
            send_button_->setEnabled(true);
            refresh_button_->setEnabled(true);
            activity_status_->setText(QStringLiteral("Transfer cancelled"));
            pending_transfer_.reset();
            return;
        }

        activity_status_->setText(QStringLiteral("Signing and submitting…"));
        const auto operation_id = QUuid::createUuid()
            .toString(QUuid::WithoutBraces)
            .toLower()
            .toStdString();
        app_log(
            AppLogLevel::Info,
            QStringLiteral("send"),
            QStringLiteral("sign and submit started")
        );
        send_watcher_.setFuture(QtConcurrent::run(
            [
                client = client_,
                draft = pending_transfer_.value(),
                operation_id
            ] {
                return submit_transfer(client, draft, operation_id);
            }
        ));
    }

    void finish_send() {
        send_button_->setEnabled(client_ != nullptr);
        refresh_button_->setEnabled(client_ != nullptr);
        const auto result = send_watcher_.result();
        pending_transfer_.reset();
        if (!result.result.has_value()) {
            app_log(
                AppLogLevel::Error,
                QStringLiteral("send"),
                QStringLiteral("send failed error=%1").arg(result.error)
            );
            activity_status_->setText(QStringLiteral("Transfer failed"));
            QMessageBox::critical(
                this,
                QStringLiteral("Transfer failed"),
                result.error
            );
            return;
        }

        app_log(
            result.result->phase == SendPhase::kSubmitted ||
                    result.result->phase == SendPhase::kConfirmed ?
                AppLogLevel::Info : AppLogLevel::Warning,
            QStringLiteral("send"),
            QStringLiteral("send finished phase=%1 message_hash=%2")
                .arg(
                    send_phase_text(result.result->phase),
                    shortened_address(
                        QString::fromStdString(result.result->message_hash)
                    )
                )
        );

        if (result.result->phase == SendPhase::kSubmissionUnknown) {
            activity_status_->setText(QStringLiteral("Submission unknown"));
            QMessageBox::critical(
                this,
                QStringLiteral("Check transfer status"),
                QStringLiteral(
                    "The transfer may have reached the network, but its result "
                    "is unknown. Do not send it again. Refresh the wallet to "
                    "resolve the pending operation."
                )
            );
            return;
        }
        if (
            result.result->phase != SendPhase::kSubmitted &&
            result.result->phase != SendPhase::kConfirmed
        ) {
            activity_status_->setText(QStringLiteral("Transfer not submitted"));
            QMessageBox::warning(
                this,
                QStringLiteral("Transfer not submitted"),
                QStringLiteral("Wallet Engine did not submit the transfer.")
            );
            return;
        }

        activity_status_->setText(QStringLiteral("Transfer submitted"));
        QMessageBox::information(
            this,
            QStringLiteral("Transfer submitted"),
            QStringLiteral(
                "The provider accepted the transfer. Refreshing wallet activity now."
            )
        );
        start_wallet_update(false);
    }

    void show_recovery_phrase(const CreatedWallet &wallet) {
        QDialog dialog(this);
        dialog.setObjectName(QStringLiteral("recoveryDialog"));
        dialog.setWindowTitle(QStringLiteral("Recovery phrase"));
        dialog.setMinimumSize(720, 600);
        dialog.resize(780, 680);

        QVBoxLayout layout(&dialog);
        layout.setContentsMargins(32, 28, 32, 28);
        layout.setSpacing(13);

        auto *icon = new QLabel(QStringLiteral("✦"), &dialog);
        icon->setObjectName(QStringLiteral("recoveryIcon"));
        icon->setAlignment(Qt::AlignCenter);
        icon->setFixedSize(44, 44);
        layout.addWidget(icon, 0, Qt::AlignHCenter);
        auto *title = new QLabel(QStringLiteral("Back up your wallet"), &dialog);
        title->setObjectName(QStringLiteral("dialogTitle"));
        title->setAlignment(Qt::AlignCenter);
        layout.addWidget(title);
        auto *subtitle = new QLabel(
            QStringLiteral(
                "Write these words down in order. This is the only way to "
                "recover your wallet."
            ),
            &dialog
        );
        subtitle->setObjectName(QStringLiteral("muted"));
        subtitle->setAlignment(Qt::AlignCenter);
        subtitle->setWordWrap(true);
        layout.addWidget(subtitle);

        auto *warning_card = new QFrame(&dialog);
        warning_card->setObjectName(QStringLiteral("warningCard"));
        auto *warning_layout = new QHBoxLayout(warning_card);
        warning_layout->setContentsMargins(13, 10, 13, 10);
        auto *warning = new QLabel(
            QStringLiteral(
                "Never share this phrase. Anyone with these words can access "
                "your wallet."
            ),
            warning_card
        );
        warning->setObjectName(QStringLiteral("dialogWarning"));
        warning->setWordWrap(true);
        warning_layout->addWidget(warning);
        layout.addWidget(warning_card);

        auto *phrase_scroll = new QScrollArea(&dialog);
        phrase_scroll->setObjectName(QStringLiteral("phraseScroll"));
        phrase_scroll->setWidgetResizable(true);
        phrase_scroll->setFrameShape(QFrame::NoFrame);
        phrase_scroll->setHorizontalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
        phrase_scroll->setMinimumHeight(290);

        auto *phrase_card = new QFrame(phrase_scroll);
        phrase_card->setObjectName(QStringLiteral("phraseCard"));
        auto *words_layout = new QGridLayout(phrase_card);
        words_layout->setContentsMargins(14, 14, 14, 14);
        words_layout->setHorizontalSpacing(10);
        words_layout->setVerticalSpacing(9);
        words_layout->setColumnStretch(0, 1);
        words_layout->setColumnStretch(1, 1);
        const auto words = QString::fromStdString(wallet.recovery_phrase.phrase)
                               .split(' ', Qt::SkipEmptyParts);
        constexpr qsizetype column_count = 2;
        const qsizetype row_count =
            (words.size() + column_count - 1) / column_count;
        for (qsizetype index = 0; index < words.size(); ++index) {
            auto *word_frame = new QFrame(phrase_card);
            word_frame->setObjectName(QStringLiteral("phraseWord"));
            auto *word_layout = new QHBoxLayout(word_frame);
            word_layout->setContentsMargins(11, 8, 11, 8);
            word_layout->setSpacing(9);
            auto *number = new QLabel(QString::number(index + 1), word_frame);
            number->setObjectName(QStringLiteral("wordNumber"));
            number->setAlignment(Qt::AlignRight | Qt::AlignVCenter);
            number->setFixedWidth(22);
            auto *word = new QLabel(words[index], word_frame);
            word->setObjectName(QStringLiteral("wordText"));
            word->setWordWrap(true);
            word->setTextInteractionFlags(Qt::TextSelectableByMouse);
            word_layout->addWidget(number);
            word_layout->addWidget(word, 1);
            words_layout->addWidget(
                word_frame,
                static_cast<int>(index % row_count),
                static_cast<int>(index / row_count)
            );
        }
        phrase_scroll->setWidget(phrase_card);
        layout.addWidget(phrase_scroll, 1);

        auto *confirmation = new QCheckBox(
            QStringLiteral("I wrote down the recovery phrase in a safe place"),
            &dialog
        );
        layout.addWidget(confirmation);
        auto *done = new QPushButton(QStringLiteral("Finish backup"), &dialog);
        done->setObjectName(QStringLiteral("primaryButton"));
        done->setMinimumHeight(42);
        done->setEnabled(false);
        layout.addWidget(done);
        connect(confirmation, &QCheckBox::toggled, done, &QPushButton::setEnabled);
        connect(done, &QPushButton::clicked, &dialog, &QDialog::accept);
        dialog.exec();
    }

    std::shared_ptr<WalletLifecycle> lifecycle_;
    std::shared_ptr<FilePlatformHost> platform_host_;
    std::shared_ptr<QtHttpHost> http_host_;
    std::shared_ptr<WalletClient> client_;
    std::optional<SavedWallet> active_wallet_;
    std::optional<TransferDraft> pending_transfer_;
    QLineEdit *record_id_ = nullptr;
    QButtonGroup *network_group_ = nullptr;
    QWidget *network_selector_ = nullptr;
    QPushButton *create_button_ = nullptr;
    QLabel *status_ = nullptr;
    QLabel *wallet_count_ = nullptr;
    QLabel *saved_count_ = nullptr;
    QLabel *balance_ = nullptr;
    QLabel *balance_hint_ = nullptr;
    QLabel *activity_status_ = nullptr;
    QPushButton *receive_button_ = nullptr;
    QPushButton *send_button_ = nullptr;
    QPushButton *refresh_button_ = nullptr;
    QPushButton *load_more_button_ = nullptr;
    QPushButton *portfolio_nav_ = nullptr;
    QPushButton *wallets_nav_ = nullptr;
    QPushButton *activity_nav_ = nullptr;
    QPushButton *settings_nav_ = nullptr;
    QPushButton *logs_nav_ = nullptr;
    QLabel *engine_badge_ = nullptr;
    QScrollArea *content_scroll_ = nullptr;
    QWidget *wallets_card_ = nullptr;
    QWidget *activity_card_ = nullptr;
    QVBoxLayout *wallet_list_layout_ = nullptr;
    QVBoxLayout *activity_list_layout_ = nullptr;
    bool loading_more_ = false;
    QFutureWatcher<CreateResult> create_watcher_;
    QFutureWatcher<ClientUpdateResult> update_watcher_;
    QFutureWatcher<PreviewResult> preview_watcher_;
    QFutureWatcher<SubmitResult> send_watcher_;
};

} // namespace

int main(int argc, char *argv[]) {
    QApplication application(argc, argv);
    app_log(
        AppLogLevel::Info,
        QStringLiteral("app"),
        QStringLiteral("Wallet Engine Qt example starting log=%1")
            .arg(app_log_file_path())
    );

    try {
        auto host = std::make_shared<FilePlatformHost>();
        auto lifecycle = wallet_engine::WalletLifecycle::init(host);
        auto api_key = load_persisted_toncenter_api_key();
        const auto environment_api_key =
            qEnvironmentVariable("TONCENTER_API_KEY").trimmed();
        if (!environment_api_key.isEmpty()) {
            api_key = environment_api_key;
            app_log(
                AppLogLevel::Info,
                QStringLiteral("settings"),
                QStringLiteral("Toncenter API key overridden by environment")
            );
        }
        auto http_host = std::make_shared<QtHttpHost>(std::move(api_key));
        MainWindow window(
            std::move(lifecycle),
            host,
            std::move(http_host)
        );
        window.show();
        app_log(
            AppLogLevel::Info,
            QStringLiteral("app"),
            QStringLiteral("main window shown")
        );
        if (qEnvironmentVariableIsSet("WALLET_ENGINE_QT_SMOKE_TEST")) {
            QTimer::singleShot(0, &application, &QCoreApplication::quit);
        }
        const auto exit_code = application.exec();
        app_log(
            AppLogLevel::Info,
            QStringLiteral("app"),
            QStringLiteral("application exiting code=%1").arg(exit_code)
        );
        return exit_code;
    } catch (const std::exception &error) {
        app_log(
            AppLogLevel::Error,
            QStringLiteral("app"),
            QStringLiteral("initialization failed error=%1")
                .arg(QString::fromUtf8(error.what()))
        );
        QMessageBox::critical(
            nullptr,
            QStringLiteral("Wallet Engine initialization failed"),
            QString::fromUtf8(error.what())
        );
        return 1;
    }
}
