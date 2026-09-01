use std::sync::Arc;

use wasm_bindgen::prelude::*;

use crate::error::engine_error;
use crate::host::{
    HttpHostAdapter, PlatformHostAdapter, StatuslessHostAdapter, WalletHttpHost,
    WalletPlatformHost, WalletStatuslessHost,
};
use crate::serde::{from_value, to_value};

#[wasm_bindgen(js_name = WalletClient)]
pub struct WalletClient {
    inner: Arc<wallet_engine::WalletClient>,
}

#[wasm_bindgen(js_class = WalletClient)]
impl WalletClient {
    #[wasm_bindgen(constructor)]
    pub fn new(
        config: JsValue,
        http_host: WalletHttpHost,
        platform_host: WalletPlatformHost,
    ) -> Result<Self, JsValue> {
        let config = from_value(config)?;
        let http_host = Arc::new(HttpHostAdapter::register(http_host));
        let platform_host = Arc::new(PlatformHostAdapter::register(platform_host));
        let inner = wallet_engine::WalletClient::new(config, http_host, platform_host)
            .map_err(|error| engine_error(&error))?;
        Ok(Self { inner })
    }

    /// Creates a client whose provider requests run through a body-or-error host.
    #[wasm_bindgen(js_name = newStatusless)]
    pub fn new_statusless(
        config: JsValue,
        statusless_host: WalletStatuslessHost,
        platform_host: WalletPlatformHost,
    ) -> Result<Self, JsValue> {
        let config = from_value(config)?;
        let statusless_host = Arc::new(StatuslessHostAdapter::register(statusless_host));
        let platform_host = Arc::new(PlatformHostAdapter::register(platform_host));
        let inner =
            wallet_engine::WalletClient::new_statusless(config, statusless_host, platform_host)
                .map_err(|error| engine_error(&error))?;
        Ok(Self { inner })
    }

    pub fn snapshot(&self) -> Result<JsValue, JsValue> {
        let snapshot = self
            .inner
            .snapshot()
            .map_err(|error| engine_error(&error))?;
        to_value(&snapshot)
    }

    #[wasm_bindgen(js_name = waitForChange)]
    pub async fn wait_for_change(&self, after_revision: u64) -> Result<JsValue, JsValue> {
        let snapshot = self
            .inner
            .wait_for_change(after_revision)
            .await
            .map_err(|error| engine_error(&error))?;
        to_value(&snapshot)
    }

    pub async fn refresh(&self) -> Result<JsValue, JsValue> {
        let update = self
            .inner
            .refresh()
            .await
            .map_err(|error| engine_error(&error))?;
        to_value(&update)
    }

    #[wasm_bindgen(js_name = resolvePending)]
    pub async fn resolve_pending(&self) -> Result<JsValue, JsValue> {
        let snapshot = self
            .inner
            .resolve_pending()
            .await
            .map_err(|error| engine_error(&error))?;
        to_value(&snapshot)
    }

    /// Resolves the standard TON DNS wallet record for a `.ton` name.
    #[wasm_bindgen(js_name = resolveDns)]
    pub async fn resolve_dns(&self, name: String) -> Result<JsValue, JsValue> {
        let address = self
            .inner
            .resolve_dns(name)
            .await
            .map_err(|error| engine_error(&error))?;
        to_value(&address)
    }

    #[wasm_bindgen(js_name = cancelRefresh)]
    pub async fn cancel_refresh(&self) -> Result<(), JsValue> {
        self.inner
            .cancel_refresh()
            .await
            .map_err(|error| engine_error(&error))
    }

    #[wasm_bindgen(js_name = loadMoreActivity)]
    pub async fn load_more_activity(&self) -> Result<JsValue, JsValue> {
        let update = self
            .inner
            .load_more_activity()
            .await
            .map_err(|error| engine_error(&error))?;
        to_value(&update)
    }

    #[wasm_bindgen(js_name = cancelLoadMoreActivity)]
    pub async fn cancel_load_more_activity(&self) -> Result<(), JsValue> {
        self.inner
            .cancel_load_more_activity()
            .await
            .map_err(|error| engine_error(&error))
    }

    #[wasm_bindgen(js_name = refreshNfts)]
    pub async fn refresh_nfts(&self) -> Result<JsValue, JsValue> {
        let update = self
            .inner
            .refresh_nfts()
            .await
            .map_err(|error| engine_error(&error))?;
        to_value(&update)
    }

    #[wasm_bindgen(js_name = cancelRefreshNfts)]
    pub async fn cancel_refresh_nfts(&self) -> Result<(), JsValue> {
        self.inner
            .cancel_refresh_nfts()
            .await
            .map_err(|error| engine_error(&error))
    }

    #[wasm_bindgen(js_name = loadMoreNfts)]
    pub async fn load_more_nfts(&self) -> Result<JsValue, JsValue> {
        let update = self
            .inner
            .load_more_nfts()
            .await
            .map_err(|error| engine_error(&error))?;
        to_value(&update)
    }

    #[wasm_bindgen(js_name = cancelLoadMoreNfts)]
    pub async fn cancel_load_more_nfts(&self) -> Result<(), JsValue> {
        self.inner
            .cancel_load_more_nfts()
            .await
            .map_err(|error| engine_error(&error))
    }

    pub async fn send(&self, request: JsValue) -> Result<JsValue, JsValue> {
        let request = from_value(request)?;
        let result = self
            .inner
            .send(request)
            .await
            .map_err(|error| engine_error(&error))?;
        to_value(&result)
    }

    /// Durably records and submits an already signed external-message BOC.
    #[wasm_bindgen(js_name = sendBoc)]
    pub async fn send_boc(&self, request: JsValue) -> Result<JsValue, JsValue> {
        let request = from_value(request)?;
        let result = self
            .inner
            .send_boc(request)
            .await
            .map_err(|error| engine_error(&error))?;
        to_value(&result)
    }

    /// Fetches the current seqno and prepares a signed Wallet rev00 key rotation.
    #[wasm_bindgen(js_name = prepareKeyRotation)]
    pub async fn prepare_key_rotation(&self, request: JsValue) -> Result<JsValue, JsValue> {
        let request = from_value(request)?;
        let prepared = self
            .inner
            .prepare_key_rotation(request)
            .await
            .map_err(|error| engine_error(&error))?;
        to_value(&prepared)
    }

    /// Builds an authorized TON encrypted-comment body.
    #[wasm_bindgen(js_name = createEncryptedComment)]
    pub async fn create_encrypted_comment(&self, request: JsValue) -> Result<JsValue, JsValue> {
        let request = from_value(request)?;
        let body = self
            .inner
            .create_encrypted_comment(request)
            .await
            .map_err(|error| engine_error(&error))?;
        to_value(&body)
    }

    /// Explicitly authorizes and decrypts a TON encrypted-comment body.
    #[wasm_bindgen(js_name = decryptComment)]
    pub async fn decrypt_comment(&self, request: JsValue) -> Result<JsValue, JsValue> {
        let request = from_value(request)?;
        let comment = self
            .inner
            .decrypt_comment(request)
            .await
            .map_err(|error| engine_error(&error))?;
        to_value(&comment)
    }

    #[wasm_bindgen(js_name = sendNftTransfer)]
    pub async fn send_nft_transfer(&self, request: JsValue) -> Result<JsValue, JsValue> {
        let request = from_value(request)?;
        let result = self
            .inner
            .send_nft_transfer(request)
            .await
            .map_err(|error| engine_error(&error))?;
        to_value(&result)
    }

    /// Signs an internal Wallet V5 message and returns it without submission.
    #[wasm_bindgen(js_name = signMessage)]
    pub async fn sign_message(&self, request: JsValue) -> Result<JsValue, JsValue> {
        let request = from_value(request)?;
        let result = self
            .inner
            .sign_message(request)
            .await
            .map_err(|error| engine_error(&error))?;
        to_value(&result)
    }

    #[wasm_bindgen(js_name = previewSend)]
    pub async fn preview_send(&self, request: JsValue) -> Result<JsValue, JsValue> {
        let request = from_value(request)?;
        let preview = self
            .inner
            .preview_send(request)
            .await
            .map_err(|error| engine_error(&error))?;
        to_value(&preview)
    }

    #[wasm_bindgen(js_name = previewNftTransfer)]
    pub async fn preview_nft_transfer(&self, request: JsValue) -> Result<JsValue, JsValue> {
        let request = from_value(request)?;
        let preview = self
            .inner
            .preview_nft_transfer(request)
            .await
            .map_err(|error| engine_error(&error))?;
        to_value(&preview)
    }

    #[wasm_bindgen(js_name = previewTonConnect)]
    pub async fn preview_ton_connect(&self, request: JsValue) -> Result<JsValue, JsValue> {
        let request = from_value(request)?;
        let preview = self
            .inner
            .preview_ton_connect(request)
            .await
            .map_err(|error| engine_error(&error))?;
        to_value(&preview)
    }

    /// Validates a sign-only request without claiming a wallet-paid network fee.
    #[wasm_bindgen(js_name = previewSignMessage)]
    pub async fn preview_sign_message(&self, request: JsValue) -> Result<JsValue, JsValue> {
        let request = from_value(request)?;
        let preview = self
            .inner
            .preview_sign_message(request)
            .await
            .map_err(|error| engine_error(&error))?;
        to_value(&preview)
    }

    #[wasm_bindgen(js_name = cancelSendPreview)]
    pub async fn cancel_send_preview(&self) -> Result<(), JsValue> {
        self.inner
            .cancel_send_preview()
            .await
            .map_err(|error| engine_error(&error))
    }

    #[wasm_bindgen(js_name = cancelSend)]
    pub async fn cancel_send(&self) -> Result<(), JsValue> {
        self.inner
            .cancel_send()
            .await
            .map_err(|error| engine_error(&error))
    }

    pub async fn shutdown(&self) -> Result<(), JsValue> {
        self.inner
            .shutdown()
            .await
            .map_err(|error| engine_error(&error))
    }
}
