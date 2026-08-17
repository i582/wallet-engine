//! Deprecated runtime used only by the legacy create-wallet entry point.
//!
//! New client-driven operations must not use this module. It will be removed
//! together with `wallet_engine_lifecycle_create_wallet` as C bindings move
//! away from the library-owned runtime.

use std::{io, sync::OnceLock};

use tokio::runtime::{Builder, Runtime};

use crate::WalletEngineAbiStatus;

static RUNTIME: OnceLock<io::Result<Runtime>> = OnceLock::new();

// This private module exposes the runtime to its sibling ABI modules.
#[allow(clippy::redundant_pub_crate)]
#[deprecated(note = "legacy async ABI only; remove with the library-owned runtime")]
pub(super) fn runtime() -> Result<&'static Runtime, WalletEngineAbiStatus> {
    RUNTIME
        .get_or_init(|| {
            Builder::new_multi_thread()
                .thread_name("wallet-engine-c")
                .build()
        })
        .as_ref()
        .map_err(|_| WalletEngineAbiStatus::Panic)
}
