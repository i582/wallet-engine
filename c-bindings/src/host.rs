//! Synchronous C callback adapter for platform-owned services.

use std::{
    ffi::c_void,
    mem::size_of,
    panic::{AssertUnwindSafe, catch_unwind},
};

use wallet_engine::{ProtectedSecretHostError, ProtectedSecretStore};

use crate::{
    StoreProtectedSecretError, WalletEngineAbiStatus, WalletEngineProtectedSecretHostErrorView,
    WalletEngineProtectedSecretStoreView,
};

/// Retains a consumer-owned callback context.
pub type WalletEngineContextRetainFn = Option<unsafe extern "C" fn(context: *mut c_void)>;

/// Releases a consumer-owned callback context.
pub type WalletEngineContextReleaseFn = Option<unsafe extern "C" fn(context: *mut c_void)>;

/// Reports the synchronous result of one protected-secret store callback.
///
/// Pass null for `error` on success. A non-null error and its diagnostic are
/// copied before this function returns. The result callback is valid only for
/// the current [`WalletEngineStoreProtectedSecretFn`] invocation and must be
/// called exactly once from that invocation's thread.
pub type WalletEngineProtectedSecretStoreResultFn = Option<
    unsafe extern "C" fn(
        context: *mut c_void,
        error: *const WalletEngineProtectedSecretHostErrorView,
    ) -> WalletEngineAbiStatus,
>;

/// Stores protected secret bytes synchronously on the calling client thread.
///
/// The request and all nested views remain valid only until this callback
/// returns. The callback must invoke `result` exactly once before returning and
/// must not retain `request`, `result_context`, or `result`.
pub type WalletEngineStoreProtectedSecretFn = Option<
    unsafe extern "C" fn(
        context: *mut c_void,
        request: *const WalletEngineProtectedSecretStoreView,
        result_context: *mut c_void,
        result: WalletEngineProtectedSecretStoreResultFn,
    ),
>;

/// Versionable callbacks supplied by the C platform host.
///
/// Each callback runs synchronously on the client-owned thread that called a
/// Wallet Engine API. The library creates no threads, queues, event loops, or
/// asynchronous continuations. The callbacks and `context` must support calls
/// from every client thread on which the lifecycle handle is used. Set
/// `struct_size` to `sizeof(WalletEnginePlatformHostCallbacks)`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct WalletEnginePlatformHostCallbacks {
    /// Size in bytes of this callback table.
    pub struct_size: usize,
    /// Opaque context forwarded to every callback.
    pub context: *mut c_void,
    /// Required function that retains `context`.
    pub retain: WalletEngineContextRetainFn,
    /// Required function that releases `context`.
    pub release: WalletEngineContextReleaseFn,
    /// Required synchronous protected-secret store function.
    pub store_protected_secret: WalletEngineStoreProtectedSecretFn,
}

/// Size of the platform-host callback table implemented by this ABI version.
pub const WALLET_ENGINE_PLATFORM_HOST_CALLBACKS_SIZE: usize =
    size_of::<WalletEnginePlatformHostCallbacks>();

#[derive(Debug)]
enum CapturedStoreResult {
    Success,
    Host(ProtectedSecretHostError),
    Abi(WalletEngineAbiStatus),
}

struct StoreResultCapture {
    result: Option<CapturedStoreResult>,
    duplicate: bool,
}

unsafe extern "C" fn capture_store_result(
    context: *mut c_void,
    error: *const WalletEngineProtectedSecretHostErrorView,
) -> WalletEngineAbiStatus {
    if context.is_null() {
        return WalletEngineAbiStatus::InvalidArgument;
    }

    // SAFETY: `context` points to the stack capture owned by the active
    // synchronous store invocation. The callback contract forbids retaining
    // the pointer or invoking it from another thread.
    let capture = unsafe { &mut *context.cast::<StoreResultCapture>() };
    let result = catch_unwind(AssertUnwindSafe(|| {
        if capture.result.is_some() {
            capture.duplicate = true;
            return WalletEngineAbiStatus::InvalidArgument;
        }

        if error.is_null() {
            capture.result = Some(CapturedStoreResult::Success);
            return WalletEngineAbiStatus::Ok;
        }

        // SAFETY: The host callback guarantees that `error` and its diagnostic
        // remain readable for this synchronous nested result call.
        let error = unsafe { error.read() };
        // SAFETY: The same callback contract covers the nested diagnostic view.
        match unsafe { error.try_to_core() } {
            Ok(error) => {
                capture.result = Some(CapturedStoreResult::Host(error));
                WalletEngineAbiStatus::Ok
            }
            Err(status) => {
                capture.result = Some(CapturedStoreResult::Abi(status));
                status
            }
        }
    }));

    match result {
        Ok(status) => status,
        Err(_) => {
            capture.result = Some(CapturedStoreResult::Abi(WalletEngineAbiStatus::Panic));
            WalletEngineAbiStatus::Panic
        }
    }
}

/// Adapter that owns the consumer callback table and retained context.
pub struct WalletEnginePlatformHostAdapter {
    callbacks: WalletEnginePlatformHostCallbacks,
}

// SAFETY: Construction requires callbacks and context that support calls from
// arbitrary client-owned threads. The callback table is immutable afterward.
unsafe impl Send for WalletEnginePlatformHostAdapter {}
// SAFETY: See the `Send` implementation.
unsafe impl Sync for WalletEnginePlatformHostAdapter {}

impl WalletEnginePlatformHostAdapter {
    /// Validates a C callback table and retains its context.
    ///
    /// # Errors
    ///
    /// Returns [`WalletEngineAbiStatus::InvalidArgument`] for a null, truncated,
    /// or incomplete callback table.
    ///
    /// # Safety
    ///
    /// `callbacks` must point to `struct_size` readable bytes. All supplied
    /// functions and the context must satisfy
    /// [`WalletEnginePlatformHostCallbacks`]'s threading and lifetime contract.
    pub unsafe fn try_from_callbacks(
        callbacks: *const WalletEnginePlatformHostCallbacks,
    ) -> Result<Self, WalletEngineAbiStatus> {
        if callbacks.is_null() {
            return Err(WalletEngineAbiStatus::InvalidArgument);
        }

        // SAFETY: The caller guarantees at least a readable `struct_size` field.
        let struct_size = unsafe { std::ptr::addr_of!((*callbacks).struct_size).read() };
        if struct_size < WALLET_ENGINE_PLATFORM_HOST_CALLBACKS_SIZE {
            return Err(WalletEngineAbiStatus::InvalidArgument);
        }

        // SAFETY: The size check and caller contract guarantee that the current
        // callback-table prefix is readable.
        let callbacks = unsafe { callbacks.read() };
        let (Some(retain), Some(_release), Some(_store)) = (
            callbacks.retain,
            callbacks.release,
            callbacks.store_protected_secret,
        ) else {
            return Err(WalletEngineAbiStatus::InvalidArgument);
        };

        // SAFETY: The callback contract requires `retain` to accept `context`
        // and keep it alive until the matching release.
        unsafe { retain(callbacks.context) };
        Ok(Self { callbacks })
    }

    pub(crate) fn store_protected_secret(
        &self,
        request: &ProtectedSecretStore,
    ) -> Result<(), StoreProtectedSecretError> {
        let Some(store) = self.callbacks.store_protected_secret else {
            return Err(StoreProtectedSecretError::Abi(
                WalletEngineAbiStatus::InvalidArgument,
            ));
        };
        let mut capture = StoreResultCapture {
            result: None,
            duplicate: false,
        };
        let request = WalletEngineProtectedSecretStoreView::from(request);

        // SAFETY: The callback was validated during construction. All borrowed
        // values and the capture remain live until this synchronous callback
        // returns, and the host contract forbids retaining them.
        unsafe {
            store(
                self.callbacks.context,
                &request,
                std::ptr::from_mut(&mut capture).cast(),
                Some(capture_store_result),
            );
        }

        if capture.duplicate {
            return Err(StoreProtectedSecretError::Abi(
                WalletEngineAbiStatus::InvalidArgument,
            ));
        }
        match capture.result {
            Some(CapturedStoreResult::Success) => Ok(()),
            Some(CapturedStoreResult::Host(error)) => Err(StoreProtectedSecretError::Host(error)),
            Some(CapturedStoreResult::Abi(status)) => Err(StoreProtectedSecretError::Abi(status)),
            None => Err(StoreProtectedSecretError::Abi(
                WalletEngineAbiStatus::InvalidArgument,
            )),
        }
    }
}

impl Drop for WalletEnginePlatformHostAdapter {
    fn drop(&mut self) {
        if let Some(release) = self.callbacks.release {
            // SAFETY: Construction retained the context once and the callback
            // contract requires the paired release to accept it.
            unsafe { release(self.callbacks.context) };
        }
    }
}
