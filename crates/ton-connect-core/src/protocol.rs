//! Top-level TON Connect message and method discriminators.

use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{AppRequest, ConnectEvent, ConnectRequest, KnownAppRequest, WalletResponse};

/// RPC methods defined by the current TON Connect specification.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum RpcMethod {
    /// Sign and broadcast outgoing messages.
    #[serde(rename = "sendTransaction")]
    SendTransaction,
    /// Sign outgoing messages without broadcasting them.
    #[serde(rename = "signMessage")]
    SignMessage,
    /// Sign application data.
    #[serde(rename = "signData")]
    SignData,
    /// End an established session.
    #[serde(rename = "disconnect")]
    Disconnect,
}

impl RpcMethod {
    /// Returns the exact method name carried on the wire and in bridge topics.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SendTransaction => "sendTransaction",
            Self::SignMessage => "signMessage",
            Self::SignData => "signData",
            Self::Disconnect => "disconnect",
        }
    }
}

impl fmt::Display for RpcMethod {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for RpcMethod {
    type Err = RpcMethodError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "sendTransaction" => Ok(Self::SendTransaction),
            "signMessage" => Ok(Self::SignMessage),
            "signData" => Ok(Self::SignData),
            "disconnect" => Ok(Self::Disconnect),
            _ => Err(RpcMethodError),
        }
    }
}

/// A method name is not part of the current TON Connect RPC catalogue.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error("unsupported TON Connect RPC method")]
pub struct RpcMethodError;

/// Anything a dApp can send to a wallet through TON Connect.
///
/// The initial connect request is transported by a link or the JS bridge.
/// Subsequent RPC requests are encrypted on the HTTP bridge.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AppMessage {
    /// Initial connection request.
    Connect(ConnectRequest),
    /// Post-connect RPC request.
    Request(AppRequest),
}

/// Wallet-initiated event carried by TON Connect.
pub type WalletEvent = ConnectEvent;

/// Anything a wallet can send to a dApp through TON Connect.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum WalletMessage {
    /// Wallet-initiated event, including connect and disconnect.
    Event(WalletEvent),
    /// Response correlated to an RPC request.
    Response(WalletResponse),
}

impl From<&KnownAppRequest> for RpcMethod {
    fn from(request: &KnownAppRequest) -> Self {
        match request {
            KnownAppRequest::SendTransaction(_) => Self::SendTransaction,
            KnownAppRequest::SignMessage(_) => Self::SignMessage,
            KnownAppRequest::SignData(_) => Self::SignData,
            KnownAppRequest::Disconnect(_) => Self::Disconnect,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_rpc_method_round_trips_with_the_normative_name() {
        for (method, name) in [
            (RpcMethod::SendTransaction, "sendTransaction"),
            (RpcMethod::SignMessage, "signMessage"),
            (RpcMethod::SignData, "signData"),
            (RpcMethod::Disconnect, "disconnect"),
        ] {
            assert_eq!(method.as_str(), name);
            assert_eq!(method.to_string(), name);
            assert_eq!(name.parse::<RpcMethod>(), Ok(method));
            assert_eq!(
                serde_json::from_str::<RpcMethod>(&format!("{name:?}")).ok(),
                Some(method)
            );
        }
        assert!("futureMethod".parse::<RpcMethod>().is_err());
    }

    #[test]
    fn top_level_message_types_select_every_protocol_envelope() {
        let connect = r#"{
            "manifestUrl":"https://app.example/tonconnect-manifest.json",
            "items":[{"name":"ton_addr"}]
        }"#;
        assert!(matches!(
            serde_json::from_str::<AppMessage>(connect),
            Ok(AppMessage::Connect(_))
        ));

        let request = r#"{"method":"disconnect","params":[],"id":"1"}"#;
        assert!(matches!(
            serde_json::from_str::<AppMessage>(request),
            Ok(AppMessage::Request(_))
        ));

        let event = r#"{"event":"disconnect","id":1,"payload":{}}"#;
        assert!(matches!(
            serde_json::from_str::<WalletMessage>(event),
            Ok(WalletMessage::Event(_))
        ));

        let response = r#"{"result":{},"id":"1"}"#;
        assert!(matches!(
            serde_json::from_str::<WalletMessage>(response),
            Ok(WalletMessage::Response(_))
        ));
    }
}
