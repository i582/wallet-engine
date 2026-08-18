#pragma once

#include <QString>

enum class AppLogLevel {
    Info,
    Warning,
    Error,
};

void app_log(
    AppLogLevel level,
    const QString &component,
    const QString &message
);

QString app_log_file_path();
