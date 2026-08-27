use wasm_bindgen::prelude::*;

use crate::serde::{from_value, to_value};

/// Returns the English word list accepted by recovery-phrase validation.
#[wasm_bindgen(js_name = mnemonicWordlist)]
pub fn mnemonic_wordlist() -> Result<JsValue, JsValue> {
    to_value(&wallet_engine::mnemonic_wordlist())
}

/// Reports every recovery scheme under which the entered words validate.
#[wasm_bindgen(js_name = detectMnemonicSchemes)]
pub fn detect_mnemonic_schemes(words: JsValue) -> Result<JsValue, JsValue> {
    let words: Vec<String> = from_value(words)?;
    to_value(&wallet_engine::detect_mnemonic_schemes(words))
}
