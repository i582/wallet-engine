use wasm_bindgen::prelude::*;

use crate::error::engine_error;
use crate::serde::{from_value, to_value};

/// Parses a raw or user-friendly TON address and returns its identity and flags.
#[wasm_bindgen(js_name = parseTonAddress)]
pub fn parse_ton_address(value: String) -> Result<JsValue, JsValue> {
    let info = wallet_engine::parse_ton_address(value).map_err(|error| engine_error(&error))?;
    to_value(&info)
}

/// Reports whether a string is a valid raw or user-friendly TON address.
#[wasm_bindgen(js_name = isValidTonAddress)]
#[must_use]
pub fn is_valid_ton_address(value: String) -> bool {
    wallet_engine::is_valid_ton_address(value)
}

/// Converts a TON address to the requested canonical representation.
#[wasm_bindgen(js_name = convertTonAddress)]
pub fn convert_ton_address(value: String, format: JsValue) -> Result<String, JsValue> {
    let format = from_value(format)?;
    wallet_engine::convert_ton_address(value, format).map_err(|error| engine_error(&error))
}
