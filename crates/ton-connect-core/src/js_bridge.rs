//! Runtime-neutral contract for an injected TON Connect JavaScript bridge.

use std::future::Future;

use thiserror::Error;

use crate::{
    AppRequest, ConnectEvent, ConnectRequest, DeviceInfo, DeviceInfoValidationError, HttpsUrl,
    WalletResponse,
};

/// Optional wallet metadata exposed directly by an injected bridge.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InjectedWalletInfo {
    name: String,
    image: HttpsUrl,
    tondns: Option<String>,
    about_url: HttpsUrl,
}

impl InjectedWalletInfo {
    /// Creates metadata that can override a wallets-list entry.
    #[allow(
        clippy::case_sensitive_file_extension_comparisons,
        reason = "the normative wallet metadata field requires lowercase .ton"
    )]
    pub fn new(
        name: String,
        image: HttpsUrl,
        tondns: Option<String>,
        about_url: HttpsUrl,
    ) -> Result<Self, JsBridgeContractError> {
        if name.is_empty()
            || tondns
                .as_ref()
                .is_some_and(|value| value == ".ton" || !value.ends_with(".ton"))
        {
            return Err(JsBridgeContractError::InvalidWalletInfo);
        }
        Ok(Self {
            name,
            image,
            tondns,
            about_url,
        })
    }

    /// Returns the wallet display name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the wallet icon URL.
    #[must_use]
    pub const fn image(&self) -> &HttpsUrl {
        &self.image
    }

    /// Returns the optional TON DNS identity.
    #[must_use]
    pub fn tondns(&self) -> Option<&str> {
        self.tondns.as_deref()
    }

    /// Returns the wallet information URL.
    #[must_use]
    pub const fn about_url(&self) -> &HttpsUrl {
        &self.about_url
    }
}

/// Static properties exposed by `window.<key>.tonconnect`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JsBridgeDescriptor {
    device_info: DeviceInfo,
    wallet_info: Option<InjectedWalletInfo>,
    protocol_version: u32,
    is_wallet_browser: bool,
}

impl JsBridgeDescriptor {
    /// Creates a self-consistent injected bridge descriptor.
    pub fn new(
        device_info: DeviceInfo,
        wallet_info: Option<InjectedWalletInfo>,
        protocol_version: u32,
        is_wallet_browser: bool,
    ) -> Result<Self, JsBridgeContractError> {
        device_info.validate()?;
        if protocol_version == 0 || protocol_version != device_info.max_protocol_version {
            return Err(JsBridgeContractError::ProtocolVersionMismatch);
        }
        Ok(Self {
            device_info,
            wallet_info,
            protocol_version,
            is_wallet_browser,
        })
    }

    /// Returns runtime-authoritative wallet metadata and features.
    #[must_use]
    pub const fn device_info(&self) -> &DeviceInfo {
        &self.device_info
    }

    /// Returns optional metadata supplied outside wallets-list.
    #[must_use]
    pub const fn wallet_info(&self) -> Option<&InjectedWalletInfo> {
        self.wallet_info.as_ref()
    }

    /// Returns the maximum protocol version accepted by `connect()`.
    #[must_use]
    pub const fn protocol_version(&self) -> u32 {
        self.protocol_version
    }

    /// Reports whether the dApp runs inside the wallet browser.
    #[must_use]
    pub const fn is_wallet_browser(&self) -> bool {
        self.is_wallet_browser
    }

    /// Reports whether a requested protocol revision is supported.
    #[must_use]
    pub const fn supports(&self, requested_version: u32) -> bool {
        requested_version > 0 && requested_version <= self.protocol_version
    }
}

/// Callback receiving wallet-initiated events from an injected bridge.
pub type JsBridgeEventListener = Box<dyn Fn(ConnectEvent) + Send + Sync + 'static>;

/// Runtime-neutral host contract matching `TonConnectBridge` from `bridge.md`.
///
/// Platform adapters implement this trait with JavaScript promises, Swift
/// continuations, Kotlin coroutines, or another executor. Core owns no runtime.
pub trait JsBridge: Send + Sync {
    /// Adapter-specific call failure.
    type Error;
    /// Handle whose drop or explicit adapter operation removes a listener.
    type Subscription;

    /// Returns the injected bridge's static descriptor.
    fn descriptor(&self) -> &JsBridgeDescriptor;

    /// Starts a user-initiated connection.
    fn connect(
        &self,
        protocol_version: u32,
        request: ConnectRequest,
    ) -> impl Future<Output = Result<ConnectEvent, Self::Error>> + Send;

    /// Restores a previously approved session without a new prompt.
    fn restore_connection(&self) -> impl Future<Output = Result<ConnectEvent, Self::Error>> + Send;

    /// Sends one ordinary dApp RPC request and awaits its correlated response.
    fn send(
        &self,
        request: AppRequest,
    ) -> impl Future<Output = Result<WalletResponse, Self::Error>> + Send;

    /// Registers a listener for wallet-initiated events.
    fn listen(&self, listener: JsBridgeEventListener) -> Result<Self::Subscription, Self::Error>;
}

/// Injected bridge metadata is internally inconsistent.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum JsBridgeContractError {
    /// Display metadata is empty or has an invalid TON DNS value.
    #[error("injected wallet metadata is invalid")]
    InvalidWalletInfo,
    /// Bridge and `DeviceInfo` protocol versions differ or are zero.
    #[error("injected bridge protocol version does not match DeviceInfo")]
    ProtocolVersionMismatch,
    /// Injected runtime metadata violates `DeviceInfo` invariants.
    #[error(transparent)]
    InvalidDeviceInfo(#[from] DeviceInfoValidationError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DevicePlatform, PROTOCOL_VERSION};

    fn device_info(version: u32) -> DeviceInfo {
        DeviceInfo {
            platform: DevicePlatform::Browser,
            app_name: "examplewallet".to_owned(),
            app_version: "1.0.0".to_owned(),
            max_protocol_version: version,
            features: Vec::new(),
        }
    }

    #[test]
    fn descriptor_requires_one_consistent_nonzero_protocol_version() {
        let version = u32::from(PROTOCOL_VERSION);
        let descriptor = JsBridgeDescriptor::new(device_info(version), None, version, true);
        assert!(
            descriptor
                .as_ref()
                .is_ok_and(|value| value.supports(version))
        );
        assert!(
            descriptor
                .as_ref()
                .is_ok_and(JsBridgeDescriptor::is_wallet_browser)
        );
        assert!(matches!(
            JsBridgeDescriptor::new(device_info(version), None, version.saturating_add(1), false),
            Err(JsBridgeContractError::ProtocolVersionMismatch)
        ));
    }

    #[test]
    fn injected_wallet_metadata_validates_identity() -> Result<(), Box<dyn std::error::Error>> {
        let image = HttpsUrl::try_from("https://wallet.example/icon.png")?;
        let about = HttpsUrl::try_from("https://wallet.example/about")?;
        assert!(
            InjectedWalletInfo::new(
                "Example Wallet".to_owned(),
                image.clone(),
                Some("example.ton".to_owned()),
                about.clone(),
            )
            .is_ok()
        );
        assert!(matches!(
            InjectedWalletInfo::new(String::new(), image, None, about),
            Err(JsBridgeContractError::InvalidWalletInfo)
        ));
        Ok(())
    }
}
