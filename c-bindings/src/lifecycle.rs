//! Opaque C handle for the wallet lifecycle service.

use std::{
    ffi::c_void,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::Arc,
};

use futures::FutureExt;
use wallet_engine::WalletLifecycle as CoreWalletLifecycle;

use crate::{
    WalletEngineAbiStatus, WalletEngineCreateWalletRequest, WalletEngineCreatedWalletView,
    WalletEnginePlatformHostAdapter, WalletEnginePlatformHostCallbacks,
    WalletEngineWalletLifecycleErrorView, runtime::runtime, with_created_wallet_view,
};

/// Receives the result of an asynchronous wallet-creation operation.
///
/// `abi_status` is `OK` for both a successful wallet and a domain failure. On
/// success, `wallet` is non-null and `error` is null. For a domain failure,
/// `wallet` is null and `error` is non-null. A boundary panic is reported with
/// `PANIC` and both result pointers null.
///
/// All result views and their nested pointers remain valid only until the
/// callback returns. The callback can run on an arbitrary worker thread.
pub type WalletEngineCreateWalletCompletionFn = Option<
    unsafe extern "C" fn(
        context: *mut c_void,
        abi_status: WalletEngineAbiStatus,
        wallet: *const WalletEngineCreatedWalletView,
        error: *const WalletEngineWalletLifecycleErrorView,
    ),
>;

#[derive(Clone, Copy)]
struct CreateWalletCompletion {
    context: *mut c_void,
    callback: unsafe extern "C" fn(
        *mut c_void,
        WalletEngineAbiStatus,
        *const WalletEngineCreatedWalletView,
        *const WalletEngineWalletLifecycleErrorView,
    ),
}

// SAFETY: The start-function contract requires the callback and its opaque
// context to be safe to use from an arbitrary worker thread. Rust forwards the
// pointer but never dereferences it.
unsafe impl Send for CreateWalletCompletion {}

impl CreateWalletCompletion {
    unsafe fn call(
        self,
        abi_status: WalletEngineAbiStatus,
        wallet: *const WalletEngineCreatedWalletView,
        error: *const WalletEngineWalletLifecycleErrorView,
    ) {
        // SAFETY: The C caller guarantees the callback accepts its context and
        // callback-scoped result views.
        unsafe { (self.callback)(self.context, abi_status, wallet, error) };
    }
}

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

/// Starts creation of a wallet on the shared asynchronous runtime.
///
/// Returns an ABI status describing argument validation and task startup. An
/// `OK` return guarantees that `completion` will be called exactly once. Domain
/// failures are delivered asynchronously with an `OK` ABI status and a
/// non-null error view.
///
/// The request is copied before this function returns. The lifecycle service
/// is retained for the operation, so the caller may immediately free its
/// lifecycle handle after an `OK` return.
///
/// # Safety
///
/// `lifecycle` must point to a live lifecycle handle for this call. `request`
/// and its record-ID view must be readable for this call. `completion` and
/// `completion_context` must remain safe to invoke from an arbitrary worker
/// thread until the callback returns. The context may be null if the callback
/// accepts null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wallet_engine_lifecycle_create_wallet(
    lifecycle: *const WalletEngineLifecycle,
    request: *const WalletEngineCreateWalletRequest,
    completion_context: *mut c_void,
    completion: WalletEngineCreateWalletCompletionFn,
) -> WalletEngineAbiStatus {
    catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: The caller upholds the handle, request, and callback
        // contracts documented by this function.
        unsafe { lifecycle_create_wallet(lifecycle, request, completion_context, completion) }
    }))
    .unwrap_or(WalletEngineAbiStatus::Panic)
}

unsafe fn lifecycle_create_wallet(
    lifecycle: *const WalletEngineLifecycle,
    request: *const WalletEngineCreateWalletRequest,
    completion_context: *mut c_void,
    completion: WalletEngineCreateWalletCompletionFn,
) -> WalletEngineAbiStatus {
    let Some(completion) = completion else {
        return WalletEngineAbiStatus::InvalidArgument;
    };
    if lifecycle.is_null() || request.is_null() {
        return WalletEngineAbiStatus::InvalidArgument;
    }

    // SAFETY: The caller guarantees a readable request and nested record-ID
    // view for this call. Conversion copies all request data.
    let request = match unsafe { request.read().try_to_core() } {
        Ok(request) => request,
        Err(status) => return status,
    };
    // SAFETY: The caller guarantees the handle remains live for this call.
    let lifecycle = Arc::clone(&unsafe { &*lifecycle }.inner);
    let completion = CreateWalletCompletion {
        context: completion_context,
        callback: completion,
    };

    let runtime = match runtime() {
        Ok(runtime) => runtime,
        Err(status) => return status,
    };
    drop(runtime.spawn(async move {
        let outcome = AssertUnwindSafe(run_create_wallet(lifecycle, request, completion))
            .catch_unwind()
            .await;
        if outcome.is_err() {
            // SAFETY: Null result pointers represent a caught boundary panic
            // and are valid for this callback invocation.
            unsafe {
                completion.call(
                    WalletEngineAbiStatus::Panic,
                    std::ptr::null(),
                    std::ptr::null(),
                )
            };
        }
    }));

    WalletEngineAbiStatus::Ok
}

async fn run_create_wallet(
    lifecycle: Arc<CoreWalletLifecycle>,
    request: wallet_engine::CreateWalletRequest,
    completion: CreateWalletCompletion,
) {
    match lifecycle.create_wallet(request).await {
        Ok(wallet) => with_created_wallet_view(&wallet, |view| {
            // SAFETY: `view` and all nested views remain live for this callback
            // invocation.
            unsafe { completion.call(WalletEngineAbiStatus::Ok, &view, std::ptr::null()) };
        }),
        Err(error) => {
            let view = WalletEngineWalletLifecycleErrorView::from(&error);
            // SAFETY: `view` and its diagnostic remain live for this callback
            // invocation.
            unsafe { completion.call(WalletEngineAbiStatus::Ok, std::ptr::null(), &view) };
        }
    }
}
