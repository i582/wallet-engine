use wasm_bindgen::prelude::*;

use crate::serde::to_value;

/// Returns the English word list accepted by recovery-phrase validation.
#[wasm_bindgen(js_name = mnemonicWordlist)]
pub fn mnemonic_wordlist() -> Result<JsValue, JsValue> {
    to_value(&wallet_engine::mnemonic_wordlist())
}
