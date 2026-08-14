mod error;
mod host;
mod lifecycle;
mod serde;
mod wallet;

pub use host::{WalletHttpHost, WalletPlatformHost};
pub use lifecycle::WalletLifecycle;
pub use wallet::WalletClient;

use wasm_bindgen::prelude::wasm_bindgen;

#[wasm_bindgen(typescript_custom_section)]
const TYPESCRIPT_HOST_API: &str = r#"
export interface WalletHttpHost {
  executeHttp(request: unknown): Promise<unknown>;
  cancelHttp(callId: unknown): Promise<void>;
}

export interface WalletPlatformHost {
  readProtectedSecret(request: unknown): Promise<Uint8Array | number[]>;
  storeProtectedSecret(request: unknown): Promise<void>;
  deleteProtectedSecret(secretRef: unknown): Promise<void>;
  loadJournal(key: unknown): Promise<unknown | undefined>;
  compareExchangeJournal(mutation: unknown): Promise<unknown>;
}
"#;
