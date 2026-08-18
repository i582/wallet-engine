#include "app_log.hpp"

#include <QDateTime>
#include <QDebug>
#include <QFile>
#include <QFileInfo>
#include <QTextStream>

#include <mutex>

namespace {

constexpr const char *log_file_name = "wallet_engine_cpp.log";
std::mutex log_mutex;

QString level_name(AppLogLevel level) {
    switch (level) {
    case AppLogLevel::Info:
        return QStringLiteral("INFO");
    case AppLogLevel::Warning:
        return QStringLiteral("WARN");
    case AppLogLevel::Error:
        return QStringLiteral("ERROR");
    }
    return QStringLiteral("UNKNOWN");
}

} // namespace

void app_log(
    AppLogLevel level,
    const QString &component,
    const QString &message
) {
    auto safe_message = message.left(1'000);
    safe_message.replace('\r', ' ');
    safe_message.replace('\n', ' ');
    const auto line = QStringLiteral("%1 [%2] [%3] %4")
        .arg(
            QDateTime::currentDateTimeUtc().toString(
                QStringLiteral("yyyy-MM-ddTHH:mm:ss.zzzZ")
            ),
            level_name(level),
            component,
            safe_message
        );

    std::lock_guard<std::mutex> lock(log_mutex);
    QFile file(QString::fromUtf8(log_file_name));
    if (file.open(QIODevice::WriteOnly | QIODevice::Append | QIODevice::Text)) {
        QTextStream(&file) << line << '\n';
    }
    switch (level) {
    case AppLogLevel::Info:
        qInfo().noquote() << line;
        break;
    case AppLogLevel::Warning:
        qWarning().noquote() << line;
        break;
    case AppLogLevel::Error:
        qCritical().noquote() << line;
        break;
    }
}

QString app_log_file_path() {
    return QFileInfo(QString::fromUtf8(log_file_name)).absoluteFilePath();
}
