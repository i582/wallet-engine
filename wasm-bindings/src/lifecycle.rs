use std::sync::Arc;

use wasm_bindgen::prelude::*;

use crate::error::engine_error;
use crate::host::{PlatformHostAdapter, WalletPlatformHost};
use crate::serde::{from_value, to_value};

#[wasm_bindgen(js_name = WalletLifecycle)]
pub struct WalletLifecycle {
    inner: Arc<wallet_engine::WalletLifecycle>,
}

#[wasm_bindgen(js_class = WalletLifecycle)]
impl WalletLifecycle {
    #[wasm_bindgen(constructor)]
    #[must_use]
    pub fn new(platform_host: WalletPlatformHost) -> Self {
        let platform_host = Arc::new(PlatformHostAdapter::register(platform_host));
        Self {
            inner: wallet_engine::WalletLifecycle::new(platform_host),
        }
    }

    #[wasm_bindgen(js_name = createWallet)]
    pub async fn create_wallet(&self, request: JsValue) -> Result<JsValue, JsValue> {
        let request = from_value(request)?;
        let wallet = self
            .inner
            .create_wallet(request)
            .await
            .map_err(|error| engine_error(&error))?;
        to_value(&wallet)
    }

    #[wasm_bindgen(js_name = importWallet)]
    pub async fn import_wallet(&self, request: JsValue) -> Result<JsValue, JsValue> {
        let request = from_value(request)?;
        let descriptor = self
            .inner
            .import_wallet(request)
            .await
            .map_err(|error| engine_error(&error))?;
        to_value(&descriptor)
    }

    #[wasm_bindgen(js_name = revealRecoveryPhrase)]
    pub async fn reveal_recovery_phrase(&self, descriptor: JsValue) -> Result<JsValue, JsValue> {
        let descriptor = from_value(descriptor)?;
        let phrase = self
            .inner
            .reveal_recovery_phrase(descriptor)
            .await
            .map_err(|error| engine_error(&error))?;
        to_value(&phrase)
    }

    #[wasm_bindgen(js_name = deleteWallet)]
    pub async fn delete_wallet(&self, descriptor: JsValue) -> Result<(), JsValue> {
        let descriptor = from_value(descriptor)?;
        self.inner
            .delete_wallet(descriptor)
            .await
            .map_err(|error| engine_error(&error))
    }
}
