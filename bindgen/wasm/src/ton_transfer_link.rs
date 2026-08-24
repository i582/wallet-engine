use wasm_bindgen::prelude::*;

use crate::error::engine_error;
use crate::serde::to_value_with_bigints;

/// Parses a strict-baseline `ton://transfer/` link into a transfer invoice.
#[wasm_bindgen(js_name = parseTonTransferLink)]
pub fn parse_ton_transfer_link(value: String) -> Result<JsValue, JsValue> {
    let parsed =
        wallet_engine::parse_ton_transfer_link(value).map_err(|error| engine_error(&error))?;
    to_value_with_bigints(&parsed)
}
