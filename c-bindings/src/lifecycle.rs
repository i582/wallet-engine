//! Opaque C handle for the wallet lifecycle service.

use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::Arc,
};

use wallet_engine::WalletLifecycle as CoreWalletLifecycle;

use crate::{
    WalletEngineAbiStatus, WalletEnginePlatformHostAdapter, WalletEnginePlatformHostCallbacks,
};

/// Opaque wallet lifecycle handle owned by the C consumer.
///
/// Create it with [`wallet_engine_lifecycle_new`] and release it with
/// [`wallet_engine_lifecycle_free`].
pub struct WalletEngineLifecycle {
    inner: Arc<CoreWalletLifecycle>,
}

/// Creates a wallet lifecycle backed by consumer-provided platform callbacks.
///
/// On success, writes a newly allocated handle to `out_lifecycle`. On failure,
/// writes null when `out_lifecycle` itself is valid. The callback context is
/// retained only after all arguments and required callbacks are validated.
///
/// # Safety
///
/// `out_lifecycle` must point to writable storage for one lifecycle pointer.
/// `host` must satisfy [`WalletEnginePlatformHostCallbacks`]'s safety and
/// lifetime contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wallet_engine_lifecycle_new(
    host: *const WalletEnginePlatformHostCallbacks,
    out_lifecycle: *mut *mut WalletEngineLifecycle,
) -> WalletEngineAbiStatus {
    catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: The caller upholds the host and output-pointer contracts.
        unsafe { lifecycle_new(host, out_lifecycle) }
    }))
    .unwrap_or(WalletEngineAbiStatus::Panic)
}

unsafe fn lifecycle_new(
    host: *const WalletEnginePlatformHostCallbacks,
    out_lifecycle: *mut *mut WalletEngineLifecycle,
) -> WalletEngineAbiStatus {
    if out_lifecycle.is_null() {
        return WalletEngineAbiStatus::InvalidArgument;
    }

    // SAFETY: The caller guarantees that `out_lifecycle` is writable.
    unsafe { out_lifecycle.write(std::ptr::null_mut()) };

    // SAFETY: The caller upholds the platform-host callback contract.
    let platform_host = match unsafe { WalletEnginePlatformHostAdapter::try_from_callbacks(host) } {
        Ok(platform_host) => platform_host,
        Err(status) => return status,
    };
    let inner = CoreWalletLifecycle::new(Arc::new(platform_host));
    let lifecycle = Box::new(WalletEngineLifecycle { inner });

    // SAFETY: The caller guarantees that `out_lifecycle` is writable. Box
    // ownership is transferred to the C consumer until the matching free.
    unsafe { out_lifecycle.write(Box::into_raw(lifecycle)) };
    WalletEngineAbiStatus::Ok
}

/// Releases a lifecycle handle. Passing null is a no-op.
///
/// # Safety
///
/// `lifecycle` must be null or a live pointer returned by
/// [`wallet_engine_lifecycle_new`] that has not already been freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wallet_engine_lifecycle_free(lifecycle: *mut WalletEngineLifecycle) {
    drop(catch_unwind(AssertUnwindSafe(|| {
        if lifecycle.is_null() {
            return;
        }

        // SAFETY: The caller transfers back the unique Box ownership obtained
        // from `wallet_engine_lifecycle_new`.
        let lifecycle = unsafe { Box::from_raw(lifecycle) };
        let WalletEngineLifecycle { inner } = *lifecycle;
        drop(inner);
    })));
}
