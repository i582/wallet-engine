#pragma once

#include "wallet_engine.hpp"

#include <QNetworkReply>
#include <QPointer>
#include <QString>

#include <map>
#include <mutex>
#include <set>

class QtHttpHost final : public wallet_engine::WalletHttpHost {
public:
    explicit QtHttpHost(QString api_key = {});

    void set_api_key(QString api_key);
    bool has_api_key() const;

    wallet_engine::HttpResponse execute_http(
        const wallet_engine::HttpRequest &request
    ) override;
    void cancel_http(
        const wallet_engine::HttpRequestId &request_id
    ) override;

private:
    QString api_key_;
    mutable std::mutex mutex_;
    std::map<uint64_t, QPointer<QNetworkReply>> active_;
    std::set<uint64_t> cancelled_before_start_;
};
