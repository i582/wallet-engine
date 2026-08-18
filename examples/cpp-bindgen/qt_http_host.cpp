#include "qt_http_host.hpp"

#include "app_log.hpp"

#include <QByteArray>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QMetaObject>
#include <QNetworkAccessManager>
#include <QNetworkRequest>
#include <QTimer>
#include <QUrl>

#include <algorithm>
#include <cstdint>
#include <limits>
#include <string>
#include <utility>
#include <vector>

namespace {

using namespace wallet_engine;

constexpr qsizetype max_request_body_bytes = 256 * 1024;
constexpr qsizetype max_response_body_bytes = 4 * 1024 * 1024;
constexpr qsizetype max_response_header_bytes = 64 * 1024;
constexpr qsizetype max_response_headers = 64;
constexpr uint64_t max_timeout_ms = 5 * 60 * 1000;
constexpr std::size_t max_early_cancellations = 256;

[[noreturn]] void throw_http_error(
    HttpHostErrorKind kind,
    const QString &message
) {
    const auto diagnostic = message.left(256).toStdString();
    app_log(AppLogLevel::Error, QStringLiteral("http"), message.left(256));
    http_host_error::Failed error(diagnostic);
    error.kind = kind;
    error.diagnostic = diagnostic;
    throw error;
}

HttpHostErrorKind network_error_kind(QNetworkReply::NetworkError error) {
    switch (error) {
    case QNetworkReply::HostNotFoundError:
        return HttpHostErrorKind::kDns;
    case QNetworkReply::TimeoutError:
        return HttpHostErrorKind::kTimeout;
    case QNetworkReply::RemoteHostClosedError:
    case QNetworkReply::TemporaryNetworkFailureError:
    case QNetworkReply::NetworkSessionFailedError:
        return HttpHostErrorKind::kConnectionLost;
    case QNetworkReply::SslHandshakeFailedError:
        return HttpHostErrorKind::kTls;
    case QNetworkReply::OperationCanceledError:
        return HttpHostErrorKind::kCancelled;
    default:
        return HttpHostErrorKind::kOther;
    }
}

bool is_standard_toncenter_origin(const QUrl &url) {
    const auto host = url.host().toLower();
    return url.scheme() == QStringLiteral("https") && url.port(443) == 443 &&
        (host == QStringLiteral("toncenter.com") ||
         host == QStringLiteral("testnet.toncenter.com"));
}

} // namespace

QtHttpHost::QtHttpHost(QString api_key) : api_key_(api_key.trimmed()) {
    app_log(
        AppLogLevel::Info,
        QStringLiteral("http"),
        api_key_.isEmpty() ? QStringLiteral("Toncenter API key is not configured") :
                             QStringLiteral("Toncenter API key is configured")
    );
}

void QtHttpHost::set_api_key(QString api_key) {
    bool configured = false;
    {
        std::lock_guard<std::mutex> lock(mutex_);
        api_key_ = api_key.trimmed();
        configured = !api_key_.isEmpty();
    }
    app_log(
        AppLogLevel::Info,
        QStringLiteral("http"),
        configured ? QStringLiteral("Toncenter API key was updated") :
                     QStringLiteral("Toncenter API key was cleared")
    );
}

bool QtHttpHost::has_api_key() const {
    std::lock_guard<std::mutex> lock(mutex_);
    return !api_key_.isEmpty();
}

wallet_engine::HttpResponse QtHttpHost::execute_http(
    const wallet_engine::HttpRequest &request
) {
    QString api_key;
    {
        std::lock_guard<std::mutex> lock(mutex_);
        api_key = api_key_;
    }
    const QUrl url(QString::fromStdString(request.url));
    if (
        !url.isValid() || url.scheme() != QStringLiteral("https") ||
        !url.userInfo().isEmpty()
    ) {
        throw_http_error(
            HttpHostErrorKind::kPolicyViolation,
            QStringLiteral("Only credential-free HTTPS URLs are allowed")
        );
    }

    QUrl safe_url = url;
    safe_url.setQuery(QString{});
    safe_url.setFragment(QString{});
    const auto method = request.method == HttpMethod::kGet ?
        QStringLiteral("GET") : QStringLiteral("POST");
    app_log(
        AppLogLevel::Info,
        QStringLiteral("http"),
        QStringLiteral("request id=%1 method=%2 endpoint=%3 timeout_ms=%4 key=%5")
            .arg(
                request.id.value
            )
            .arg(method, safe_url.toString())
            .arg(request.timeout_ms)
            .arg(api_key.isEmpty() ? QStringLiteral("none") :
                                     QStringLiteral("configured"))
    );
    QElapsedTimer elapsed;
    elapsed.start();
    if (
        request.timeout_ms == 0 || request.timeout_ms > max_timeout_ms ||
        request.timeout_ms >
            static_cast<uint64_t>(std::numeric_limits<int>::max())
    ) {
        throw_http_error(
            HttpHostErrorKind::kPolicyViolation,
            QStringLiteral("The provider timeout is invalid")
        );
    }
    if (
        request.body.size() >
        static_cast<std::size_t>(max_request_body_bytes)
    ) {
        throw_http_error(
            HttpHostErrorKind::kPolicyViolation,
            QStringLiteral("The request body exceeds its limit")
        );
    }

    QNetworkRequest qt_request(url);
    qt_request.setAttribute(
        QNetworkRequest::RedirectPolicyAttribute,
        QNetworkRequest::ManualRedirectPolicy
    );
    qt_request.setAttribute(
        QNetworkRequest::MaximumDownloadBufferSizeAttribute,
        max_response_body_bytes + 1
    );
    qt_request.setTransferTimeout(static_cast<int>(request.timeout_ms));
    for (const auto &header : request.headers) {
        const auto name = QByteArray::fromStdString(header.name);
        const auto lower_name = name.toLower();
        if (
            lower_name == "authorization" || lower_name == "cookie" ||
            lower_name == "x-api-key"
        ) {
            throw_http_error(
                HttpHostErrorKind::kPolicyViolation,
                QStringLiteral("The request contains a host-owned header")
            );
        }
        qt_request.setRawHeader(
            name,
            QByteArray::fromStdString(header.value)
        );
    }
    if (!api_key.isEmpty() && is_standard_toncenter_origin(url)) {
        qt_request.setRawHeader("X-API-Key", api_key.toUtf8());
    }

    QNetworkAccessManager manager;
    QNetworkReply *reply = nullptr;
    const QByteArray body(
        reinterpret_cast<const char *>(request.body.data()),
        static_cast<qsizetype>(request.body.size())
    );
    switch (request.method) {
    case HttpMethod::kGet:
        reply = manager.get(qt_request);
        break;
    case HttpMethod::kPost:
        reply = manager.post(qt_request, body);
        break;
    }

    {
        std::lock_guard<std::mutex> lock(mutex_);
        if (cancelled_before_start_.erase(request.id.value) != 0) {
            reply->abort();
            throw_http_error(
                HttpHostErrorKind::kCancelled,
                QStringLiteral("The HTTP request was cancelled")
            );
        }
        if (active_.contains(request.id.value)) {
            reply->abort();
            throw_http_error(
                HttpHostErrorKind::kPolicyViolation,
                QStringLiteral("The HTTP request ID is already active")
            );
        }
        active_.emplace(request.id.value, reply);
    }

    bool timed_out = false;
    QEventLoop event_loop;
    QTimer timeout;
    timeout.setSingleShot(true);
    QObject::connect(reply, &QNetworkReply::finished, &event_loop, &QEventLoop::quit);
    QObject::connect(&timeout, &QTimer::timeout, reply, [&timed_out, reply] {
        timed_out = true;
        reply->abort();
    });
    timeout.start(static_cast<int>(request.timeout_ms));
    event_loop.exec();
    timeout.stop();

    {
        std::lock_guard<std::mutex> lock(mutex_);
        active_.erase(request.id.value);
    }

    if (timed_out) {
        throw_http_error(
            HttpHostErrorKind::kTimeout,
            QStringLiteral("The provider request timed out")
        );
    }

    const auto status_attribute = reply->attribute(
        QNetworkRequest::HttpStatusCodeAttribute
    );
    if (!status_attribute.isValid()) {
        throw_http_error(
            network_error_kind(reply->error()),
            reply->errorString()
        );
    }
    const auto status = status_attribute.toInt();
    if (status >= 300 && status < 400) {
        throw_http_error(
            HttpHostErrorKind::kPolicyViolation,
            QStringLiteral("HTTP redirects are not allowed")
        );
    }
    if (status < 0 || status > std::numeric_limits<uint16_t>::max()) {
        throw_http_error(
            HttpHostErrorKind::kPolicyViolation,
            QStringLiteral("The provider returned an invalid status code")
        );
    }

    const auto response_body = reply->readAll();
    if (response_body.size() > max_response_body_bytes) {
        throw_http_error(
            HttpHostErrorKind::kResponseTooLarge,
            QStringLiteral("The response body exceeds its limit")
        );
    }

    const auto raw_headers = reply->rawHeaderPairs();
    if (raw_headers.size() > max_response_headers) {
        throw_http_error(
            HttpHostErrorKind::kResponseTooLarge,
            QStringLiteral("The response contains too many headers")
        );
    }
    qsizetype header_bytes = 0;
    std::vector<HttpHeader> headers;
    headers.reserve(static_cast<std::size_t>(raw_headers.size()));
    for (const auto &[name, value] : raw_headers) {
        header_bytes += name.size() + value.size();
        if (header_bytes > max_response_header_bytes) {
            throw_http_error(
                HttpHostErrorKind::kResponseTooLarge,
                QStringLiteral("The response headers exceed their limit")
            );
        }
        if (!api_key.isEmpty() && value == api_key.toUtf8()) {
            continue;
        }
        headers.push_back({name.toStdString(), value.toStdString()});
    }

    app_log(
        AppLogLevel::Info,
        QStringLiteral("http"),
        QStringLiteral("response id=%1 status=%2 bytes=%3 elapsed_ms=%4")
            .arg(request.id.value)
            .arg(status)
            .arg(response_body.size())
            .arg(elapsed.elapsed())
    );

    return {
        static_cast<uint16_t>(status),
        std::move(headers),
        std::vector<uint8_t>(response_body.begin(), response_body.end()),
        request.url,
    };
}

void QtHttpHost::cancel_http(
    const wallet_engine::HttpRequestId &request_id
) {
    app_log(
        AppLogLevel::Warning,
        QStringLiteral("http"),
        QStringLiteral("cancel request id=%1").arg(request_id.value)
    );
    QPointer<QNetworkReply> reply;
    {
        std::lock_guard<std::mutex> lock(mutex_);
        const auto entry = active_.find(request_id.value);
        if (entry != active_.end()) {
            reply = entry->second;
            active_.erase(entry);
        } else {
            cancelled_before_start_.insert(request_id.value);
            while (cancelled_before_start_.size() > max_early_cancellations) {
                cancelled_before_start_.erase(cancelled_before_start_.begin());
            }
        }
    }
    if (reply) {
        QMetaObject::invokeMethod(
            reply.data(),
            &QNetworkReply::abort,
            Qt::QueuedConnection
        );
    }
}
