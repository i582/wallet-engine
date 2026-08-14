use serde::Serialize;
use serde::de::DeserializeOwned;
use wasm_bindgen::JsValue;

use crate::error::binding_error;

pub(crate) fn from_value<T: DeserializeOwned>(value: JsValue) -> Result<T, JsValue> {
    serde_wasm_bindgen::from_value(value).map_err(|error| binding_error(&error.to_string()))
}

pub(crate) fn to_value<T: Serialize>(value: &T) -> Result<JsValue, JsValue> {
    serde_wasm_bindgen::to_value(value).map_err(|error| binding_error(&error.to_string()))
}
