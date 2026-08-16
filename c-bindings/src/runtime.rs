//! Shared asynchronous runtime for C ABI operations.

use std::{io, sync::OnceLock};

use tokio::runtime::{Builder, Runtime};

use crate::WalletEngineAbiStatus;

static RUNTIME: OnceLock<io::Result<Runtime>> = OnceLock::new();

// This private module exposes the runtime to its sibling ABI modules.
#[allow(clippy::redundant_pub_crate)]
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
