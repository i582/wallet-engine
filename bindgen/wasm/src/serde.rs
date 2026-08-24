use serde::Serialize;
use serde::de::DeserializeOwned;
use wasm_bindgen::JsValue;

use crate::error::binding_error;

pub(crate) fn from_value<T: DeserializeOwned>(value: JsValue) -> Result<T, JsValue> {
    serde_wasm_bindgen::from_value(value).map_err(|error| binding_error(&error.to_string()))
}

pub(crate) fn to_value<T: Serialize>(value: &T) -> Result<JsValue, JsValue> {
    // Public TypeScript records use plain objects; the default serializer emits JavaScript Map.
    value
        .serialize(&serde_wasm_bindgen::Serializer::new().serialize_maps_as_objects(true))
        .map_err(|error| binding_error(&error.to_string()))
}

pub(crate) fn to_value_with_bigints<T: Serialize>(value: &T) -> Result<JsValue, JsValue> {
    value
        .serialize(
            &serde_wasm_bindgen::Serializer::new()
                .serialize_maps_as_objects(true)
                .serialize_large_number_types_as_bigints(true),
        )
        .map_err(|error| binding_error(&error.to_string()))
}
