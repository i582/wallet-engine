use js_sys::{Error, Reflect};
use wasm_bindgen::{JsCast, JsValue};

pub(crate) fn binding_error(message: &str) -> JsValue {
    Error::new(message).into()
}

pub(crate) fn engine_error(error: &impl std::fmt::Display) -> JsValue {
    binding_error(&error.to_string())
}

pub(crate) fn rejection_kind(value: &JsValue) -> Option<String> {
    Reflect::get(value, &JsValue::from_str("kind"))
        .ok()
        .and_then(|value| value.as_string())
}

pub(crate) fn rejection_diagnostic(value: &JsValue) -> String {
    Reflect::get(value, &JsValue::from_str("diagnostic"))
        .ok()
        .and_then(|value| value.as_string())
        .or_else(|| value.dyn_ref::<Error>().map(Error::message).map(Into::into))
        .or_else(|| value.as_string())
        .unwrap_or_else(|| "JavaScript host callback failed".to_owned())
}
