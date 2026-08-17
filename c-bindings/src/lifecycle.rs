//! Synchronous C handle for wallet lifecycle operations.

use std::{
    ffi::c_void,
    panic::{AssertUnwindSafe, catch_unwind},
};

use wallet_engine::{
    CreatedWallet, WalletDescriptor, WalletLifecycleError,
    c_abi::{self, WalletLifecycleCallError},
};

use crate::{
    StoreProtectedSecretError, WalletEngineAbiStatus, WalletEngineCreateWalletRequest,
    WalletEngineCreatedWalletView, WalletEngineImportWalletRequest,
    WalletEnginePlatformHostAdapter, WalletEnginePlatformHostCallbacks,
    WalletEngineWalletDescriptorView, WalletEngineWalletLifecycleErrorView,
    with_created_wallet_view,
};

/// Receives a wallet-creation result during the synchronous API call.
///
/// `abi_status` is `OK`; boundary failures are returned directly by
/// [`wallet_engine_lifecycle_create_wallet`] without invoking this callback.
/// On success `wallet` is non-null and `error` is null. On a domain failure
/// `wallet` is null and `error` is non-null. All borrowed views remain valid
/// only until this callback returns.
pub type WalletEngineCreateWalletResultFn = Option<
    unsafe extern "C" fn(
        context: *mut c_void,
        abi_status: WalletEngineAbiStatus,
        wallet: *const WalletEngineCreatedWalletView,
        error: *const WalletEngineWalletLifecycleErrorView,
    ),
>;

/// Receives a wallet-import result during the synchronous API call.
///
/// `abi_status` is `OK`; boundary failures are returned directly by
/// [`wallet_engine_lifecycle_import_wallet`] without invoking this callback.
/// On success `descriptor` is non-null and `error` is null. On a domain failure
/// `descriptor` is null and `error` is non-null. All borrowed views remain
/// valid only until this callback returns.
pub type WalletEngineImportWalletResultFn = Option<
    unsafe extern "C" fn(
        context: *mut c_void,
        abi_status: WalletEngineAbiStatus,
        descriptor: *const WalletEngineWalletDescriptorView,
        error: *const WalletEngineWalletLifecycleErrorView,
    ),
>;

#[derive(Clone, Copy)]
struct CreateWalletResult {
    context: *mut c_void,
    callback: unsafe extern "C" fn(
        *mut c_void,
        WalletEngineAbiStatus,
        *const WalletEngineCreatedWalletView,
        *const WalletEngineWalletLifecycleErrorView,
    ),
}

impl CreateWalletResult {
    unsafe fn call(
        self,
        wallet: *const WalletEngineCreatedWalletView,
        error: *const WalletEngineWalletLifecycleErrorView,
    ) {
        // SAFETY: The C caller guarantees that the callback accepts its context
        // and callback-scoped result views.
        unsafe { (self.callback)(self.context, WalletEngineAbiStatus::Ok, wallet, error) };
    }
}

fn deliver_create_wallet_outcome(
    outcome: Result<CreatedWallet, WalletLifecycleError>,
    result: CreateWalletResult,
) {
    match outcome {
        Ok(wallet) => with_created_wallet_view(&wallet, |view| {
            // SAFETY: `view` and every nested view remain live for this call.
            unsafe { result.call(&view, std::ptr::null()) };
        }),
        Err(error) => {
            let view = WalletEngineWalletLifecycleErrorView::from(&error);
            // SAFETY: `view` and its diagnostic remain live for this call.
            unsafe { result.call(std::ptr::null(), &view) };
        }
    }
}

#[derive(Clone, Copy)]
struct ImportWalletResult {
    context: *mut c_void,
    callback: unsafe extern "C" fn(
        *mut c_void,
        WalletEngineAbiStatus,
        *const WalletEngineWalletDescriptorView,
        *const WalletEngineWalletLifecycleErrorView,
    ),
}

impl ImportWalletResult {
    unsafe fn call(
        self,
        descriptor: *const WalletEngineWalletDescriptorView,
        error: *const WalletEngineWalletLifecycleErrorView,
    ) {
        // SAFETY: The C caller guarantees that the callback accepts its context
        // and callback-scoped result views.
        unsafe { (self.callback)(self.context, WalletEngineAbiStatus::Ok, descriptor, error) };
    }
}

fn deliver_import_wallet_outcome(
    outcome: Result<WalletDescriptor, WalletLifecycleError>,
    result: ImportWalletResult,
) {
    match outcome {
        Ok(descriptor) => {
            let view = WalletEngineWalletDescriptorView::from(&descriptor);
            // SAFETY: `view` and every nested view borrow the live descriptor.
            unsafe { result.call(&view, std::ptr::null()) };
        }
        Err(error) => {
            let view = WalletEngineWalletLifecycleErrorView::from(&error);
            // SAFETY: `view` and its diagnostic remain live for this call.
            unsafe { result.call(std::ptr::null(), &view) };
        }
    }
}

/// Opaque wallet lifecycle handle owned by the C consumer.
///
/// Every lifecycle operation is synchronous and runs entirely on the calling
/// client-owned thread. A live handle may be used concurrently from multiple
/// client threads when the supplied host callbacks support those calls.
/// `free` requires external synchronization with every use of the raw handle.
pub struct WalletEngineLifecycle {
    host: WalletEnginePlatformHostAdapter,
}

/// Creates a lifecycle backed by consumer-provided synchronous callbacks.
///
/// On success, writes a newly allocated handle to `out_lifecycle`. On failure,
/// writes null when `out_lifecycle` itself is valid. The callback context is
/// retained only after every argument and required callback is validated.
///
/// # Safety
///
/// `out_lifecycle` must point to writable storage for one lifecycle pointer.
/// `host` must satisfy [`WalletEnginePlatformHostCallbacks`]'s safety,
/// threading, and lifetime contract.
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
    // SAFETY: The caller guarantees writable output storage.
    unsafe { out_lifecycle.write(std::ptr::null_mut()) };

    // SAFETY: The caller upholds the callback-table contract.
    let host = match unsafe { WalletEnginePlatformHostAdapter::try_from_callbacks(host) } {
        Ok(host) => host,
        Err(status) => return status,
    };
    let lifecycle = Box::new(WalletEngineLifecycle { host });

    // SAFETY: Box ownership transfers to the C consumer until `free`.
    unsafe { out_lifecycle.write(Box::into_raw(lifecycle)) };
    WalletEngineAbiStatus::Ok
}

/// Releases a lifecycle handle. Passing null is a no-op.
///
/// # Safety
///
/// `lifecycle` must be null or a live pointer returned by
/// [`wallet_engine_lifecycle_new`] that has not already been freed. The caller
/// must externally synchronize this call with all other uses of the handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wallet_engine_lifecycle_free(lifecycle: *mut WalletEngineLifecycle) {
    drop(catch_unwind(AssertUnwindSafe(|| {
        if lifecycle.is_null() {
            return;
        }
        // SAFETY: The caller transfers back the unique Box ownership obtained
        // from `wallet_engine_lifecycle_new`.
        drop(unsafe { Box::from_raw(lifecycle) });
    })));
}

/// Creates and stores a wallet synchronously on the calling client thread.
///
/// The host storage callback and `result` are invoked before this function
/// returns and are never retained. The client owns all scheduling: to perform
/// this operation asynchronously, call it from a client-owned worker thread.
/// Distinct calls may execute concurrently on different client threads.
///
/// # Safety
///
/// `lifecycle` must point to a live lifecycle handle for this call. `request`
/// and its nested views must be readable for this call. `result_context` must
/// be valid for one synchronous invocation of `result`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wallet_engine_lifecycle_create_wallet(
    lifecycle: *const WalletEngineLifecycle,
    request: *const WalletEngineCreateWalletRequest,
    result_context: *mut c_void,
    result: WalletEngineCreateWalletResultFn,
) -> WalletEngineAbiStatus {
    catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: The caller upholds the handle, request, and callback contracts.
        unsafe { lifecycle_create_wallet(lifecycle, request, result_context, result) }
    }))
    .unwrap_or(WalletEngineAbiStatus::Panic)
}

unsafe fn lifecycle_create_wallet(
    lifecycle: *const WalletEngineLifecycle,
    request: *const WalletEngineCreateWalletRequest,
    result_context: *mut c_void,
    result: WalletEngineCreateWalletResultFn,
) -> WalletEngineAbiStatus {
    let Some(result) = result else {
        return WalletEngineAbiStatus::InvalidArgument;
    };
    if lifecycle.is_null() || request.is_null() {
        return WalletEngineAbiStatus::InvalidArgument;
    }

    // SAFETY: The caller guarantees a readable request and nested views.
    let request = match unsafe { request.read().try_to_core() } {
        Ok(request) => request,
        Err(status) => return status,
    };
    // SAFETY: The caller guarantees a live lifecycle for this call.
    let host = &unsafe { &*lifecycle }.host;
    let outcome = c_abi::create_wallet(request, |request| host.store_protected_secret(&request));
    let outcome = match outcome {
        Ok(wallet) => Ok(wallet),
        Err(WalletLifecycleCallError::Wallet(error)) => Err(error),
        Err(WalletLifecycleCallError::Store(StoreProtectedSecretError::Host(error))) => {
            Err(error.into())
        }
        Err(WalletLifecycleCallError::Store(StoreProtectedSecretError::Abi(status))) => {
            return status;
        }
    };

    deliver_create_wallet_outcome(
        outcome,
        CreateWalletResult {
            context: result_context,
            callback: result,
        },
    );
    WalletEngineAbiStatus::Ok
}

/// Imports and stores a wallet synchronously on the calling client thread.
///
/// The host storage callback and `result` are invoked before this function
/// returns and are never retained. The client owns all scheduling: to perform
/// this operation asynchronously, call it from a client-owned worker thread.
/// Distinct calls may execute concurrently on different client threads.
///
/// # Safety
///
/// `lifecycle` must point to a live lifecycle handle for this call. `request`
/// and every nested view must be readable for this call. `result_context` must
/// be valid for one synchronous invocation of `result`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wallet_engine_lifecycle_import_wallet(
    lifecycle: *const WalletEngineLifecycle,
    request: *const WalletEngineImportWalletRequest,
    result_context: *mut c_void,
    result: WalletEngineImportWalletResultFn,
) -> WalletEngineAbiStatus {
    catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: The caller upholds the handle, request, and callback contracts.
        unsafe { lifecycle_import_wallet(lifecycle, request, result_context, result) }
    }))
    .unwrap_or(WalletEngineAbiStatus::Panic)
}

unsafe fn lifecycle_import_wallet(
    lifecycle: *const WalletEngineLifecycle,
    request: *const WalletEngineImportWalletRequest,
    result_context: *mut c_void,
    result: WalletEngineImportWalletResultFn,
) -> WalletEngineAbiStatus {
    let Some(result) = result else {
        return WalletEngineAbiStatus::InvalidArgument;
    };
    if lifecycle.is_null() || request.is_null() {
        return WalletEngineAbiStatus::InvalidArgument;
    }

    // SAFETY: The caller guarantees a readable request and nested views.
    let request = match unsafe { request.read().try_to_core() } {
        Ok(request) => request,
        Err(status) => return status,
    };
    // SAFETY: The caller guarantees a live lifecycle for this call.
    let host = &unsafe { &*lifecycle }.host;
    let outcome = c_abi::import_wallet(request, |request| host.store_protected_secret(&request));
    let outcome = match outcome {
        Ok(descriptor) => Ok(descriptor),
        Err(WalletLifecycleCallError::Wallet(error)) => Err(error),
        Err(WalletLifecycleCallError::Store(StoreProtectedSecretError::Host(error))) => {
            Err(error.into())
        }
        Err(WalletLifecycleCallError::Store(StoreProtectedSecretError::Abi(status))) => {
            return status;
        }
    };

    deliver_import_wallet_outcome(
        outcome,
        ImportWalletResult {
            context: result_context,
            callback: result,
        },
    );
    WalletEngineAbiStatus::Ok
}
