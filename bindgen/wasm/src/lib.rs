mod address;
mod error;
mod host;
mod lifecycle;
mod mnemonic;
mod serde;
mod ton_transfer_link;
mod wallet;

pub use address::{convert_ton_address, is_valid_ton_address, parse_ton_address};
pub use host::{WalletHttpHost, WalletPlatformHost, WalletStatuslessHost};
pub use lifecycle::WalletLifecycle;
pub use mnemonic::mnemonic_wordlist;
pub use ton_transfer_link::parse_ton_transfer_link;
pub use wallet::WalletClient;

use wasm_bindgen::prelude::wasm_bindgen;

#[wasm_bindgen(typescript_custom_section)]
const TYPESCRIPT_HOST_API: &str = r#"
export interface WalletHttpHost {
  executeHttp(request: unknown): Promise<unknown>;
  cancelHttp(callId: unknown): Promise<void>;
}

export interface WalletStatuslessHost {
  executeStatusless(request: unknown): Promise<Uint8Array | number[]>;
  cancelStatusless(callId: unknown): Promise<void>;
}

export interface WalletPlatformHost {
  readProtectedSecret(request: unknown): Promise<Uint8Array | number[]>;
  storeProtectedSecret(request: unknown): Promise<void>;
  deleteProtectedSecret(secretRef: unknown): Promise<void>;
  loadJournal(key: unknown): Promise<unknown | undefined>;
  compareExchangeJournal(mutation: unknown): Promise<unknown>;
}
"#;
