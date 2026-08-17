//! Opaque C handle for the wallet lifecycle service.

use std::{
    ffi::c_void,
    future::Future,
    panic::{AssertUnwindSafe, catch_unwind},
    pin::Pin,
    sync::{Arc, Mutex, TryLockError},
    task::{Context, Poll},
};

use futures::{FutureExt, task::noop_waker_ref};
use wallet_engine::{CreatedWallet, WalletLifecycle as CoreWalletLifecycle, WalletLifecycleError};

#[allow(deprecated, reason = "used only by the deprecated asynchronous C ABI")]
use crate::runtime::runtime;
use crate::{
    WalletEngineAbiStatus, WalletEngineCreateWalletRequest, WalletEngineCreatedWalletView,
    WalletEnginePlatformHostAdapter, WalletEnginePlatformHostCallbacks,
    WalletEngineWalletLifecycleErrorView, with_created_wallet_view,
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
#[deprecated(
    note = "use WalletEngineCreateWalletResultFn with wallet_engine_create_wallet_operation_poll"
)]
pub type WalletEngineCreateWalletCompletionFn = Option<
    unsafe extern "C" fn(
        context: *mut c_void,
        abi_status: WalletEngineAbiStatus,
        wallet: *const WalletEngineCreatedWalletView,
        error: *const WalletEngineWalletLifecycleErrorView,
    ),
>;

/// Receives a wallet-creation result synchronously from an explicit poll.
///
/// This callback is never retained. It is invoked only when
/// [`wallet_engine_create_wallet_operation_poll`] returns a `READY` state and
/// always runs on the thread that called that poll function. `abi_status` is
/// `OK`; boundary failures are returned directly by the poll function.
///
/// All result views and their nested pointers remain valid only until the
/// callback returns.
pub type WalletEngineCreateWalletResultFn = Option<
    unsafe extern "C" fn(
        context: *mut c_void,
        abi_status: WalletEngineAbiStatus,
        wallet: *const WalletEngineCreatedWalletView,
        error: *const WalletEngineWalletLifecycleErrorView,
    ),
>;

/// State produced by one client-driven operation poll.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalletEngineOperationPollState {
    /// The operation has not finished; the client decides when to poll again.
    Pending = 0,
    /// The operation finished and delivered its result during this poll call.
    Ready = 1,
}

type CreateWalletOutcome = Result<CreatedWallet, WalletLifecycleError>;
type CreateWalletFuture = Pin<Box<dyn Future<Output = CreateWalletOutcome> + Send + 'static>>;

struct CreateWalletOperationState {
    future: Option<CreateWalletFuture>,
}

/// Opaque client-driven wallet-creation operation.
///
/// Create it with [`wallet_engine_lifecycle_create_wallet_start`], advance it
/// with [`wallet_engine_create_wallet_operation_poll`], and release it with
/// [`wallet_engine_create_wallet_operation_free`]. The handle has no thread
/// affinity, but the same operation must not be freed concurrently with any
/// other call that uses it.
pub struct WalletEngineCreateWalletOperation {
    state: Mutex<CreateWalletOperationState>,
}

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

fn deliver_create_wallet_outcome(outcome: CreateWalletOutcome, completion: CreateWalletCompletion) {
    match outcome {
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

/// Creates a client-driven wallet-creation operation without polling it.
///
/// The request is copied and the lifecycle service is retained before this
/// function returns. No platform or result callback is invoked by this call.
/// On success, the client owns `out_operation` until the matching free.
///
/// # Safety
///
/// `lifecycle` must point to a live lifecycle handle for this call. `request`
/// and its nested views must be readable for this call. `out_operation` must
/// point to writable storage for one operation pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wallet_engine_lifecycle_create_wallet_start(
    lifecycle: *const WalletEngineLifecycle,
    request: *const WalletEngineCreateWalletRequest,
    out_operation: *mut *mut WalletEngineCreateWalletOperation,
) -> WalletEngineAbiStatus {
    catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: The caller upholds the handle, request, and output-pointer
        // contracts documented by this function.
        unsafe { lifecycle_create_wallet_start(lifecycle, request, out_operation) }
    }))
    .unwrap_or(WalletEngineAbiStatus::Panic)
}

unsafe fn lifecycle_create_wallet_start(
    lifecycle: *const WalletEngineLifecycle,
    request: *const WalletEngineCreateWalletRequest,
    out_operation: *mut *mut WalletEngineCreateWalletOperation,
) -> WalletEngineAbiStatus {
    if out_operation.is_null() {
        return WalletEngineAbiStatus::InvalidArgument;
    }

    // SAFETY: The caller guarantees that `out_operation` is writable.
    unsafe { out_operation.write(std::ptr::null_mut()) };
    if lifecycle.is_null() || request.is_null() {
        return WalletEngineAbiStatus::InvalidArgument;
    }

    // SAFETY: The caller guarantees a readable request and nested record-ID
    // view for this call. Conversion copies all request data.
    let request = match unsafe { request.read().try_to_core() } {
        Ok(request) => request,
        Err(status) => return status,
    };
    // SAFETY: The caller guarantees the lifecycle handle remains live for this
    // call. The future owns this Arc independently of the C handle afterward.
    let lifecycle = Arc::clone(&unsafe { &*lifecycle }.inner);
    let future: CreateWalletFuture =
        Box::pin(async move { lifecycle.create_wallet(request).await });
    let operation = Box::new(WalletEngineCreateWalletOperation {
        state: Mutex::new(CreateWalletOperationState {
            future: Some(future),
        }),
    });

    // SAFETY: The output pointer is writable and receives unique Box ownership
    // until `wallet_engine_create_wallet_operation_free` is called.
    unsafe { out_operation.write(Box::into_raw(operation)) };
    WalletEngineAbiStatus::Ok
}

/// Polls a wallet-creation operation once on the calling thread.
///
/// A `PENDING` result never invokes `result`. A `READY` result invokes `result`
/// exactly once before this function returns; the callback is not retained.
/// The client is solely responsible for deciding where and when to poll again.
/// Concurrent polls of the same operation return `OPERATION_BUSY` rather than
/// waiting. Different operation handles may be polled concurrently.
///
/// # Safety
///
/// `operation` must point to a live operation handle and must not be freed for
/// this call. `out_state` must point to writable storage. `result_context` must
/// be valid for a synchronous invocation of `result` during this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wallet_engine_create_wallet_operation_poll(
    operation: *mut WalletEngineCreateWalletOperation,
    result_context: *mut c_void,
    result: WalletEngineCreateWalletResultFn,
    out_state: *mut WalletEngineOperationPollState,
) -> WalletEngineAbiStatus {
    catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: The caller upholds the operation, callback, and output
        // contracts documented by this function.
        unsafe { create_wallet_operation_poll(operation, result_context, result, out_state) }
    }))
    .unwrap_or(WalletEngineAbiStatus::Panic)
}

unsafe fn create_wallet_operation_poll(
    operation: *mut WalletEngineCreateWalletOperation,
    result_context: *mut c_void,
    result: WalletEngineCreateWalletResultFn,
    out_state: *mut WalletEngineOperationPollState,
) -> WalletEngineAbiStatus {
    if out_state.is_null() {
        return WalletEngineAbiStatus::InvalidArgument;
    }
    // SAFETY: The caller guarantees that `out_state` is writable. Pending is
    // the conservative value when later validation or polling fails.
    unsafe { out_state.write(WalletEngineOperationPollState::Pending) };

    let Some(result) = result else {
        return WalletEngineAbiStatus::InvalidArgument;
    };
    if operation.is_null() {
        return WalletEngineAbiStatus::InvalidArgument;
    }

    // SAFETY: The caller guarantees a live operation for this call.
    let operation = unsafe { &*operation };
    let mut state = match operation.state.try_lock() {
        Ok(state) => state,
        Err(TryLockError::WouldBlock) => return WalletEngineAbiStatus::OperationBusy,
        Err(TryLockError::Poisoned(_)) => return WalletEngineAbiStatus::Panic,
    };
    let Some(future) = state.future.as_mut() else {
        return WalletEngineAbiStatus::InvalidArgument;
    };

    let poll = catch_unwind(AssertUnwindSafe(|| {
        let mut context = Context::from_waker(noop_waker_ref());
        future.as_mut().poll(&mut context)
    }));
    let outcome = match poll {
        Ok(Poll::Pending) => return WalletEngineAbiStatus::Ok,
        Ok(Poll::Ready(outcome)) => outcome,
        Err(_) => {
            state.future = None;
            return WalletEngineAbiStatus::Panic;
        }
    };

    state.future = None;
    drop(state);
    // SAFETY: The caller guarantees that `out_state` remains writable for the
    // duration of this call.
    unsafe { out_state.write(WalletEngineOperationPollState::Ready) };
    let completion = CreateWalletCompletion {
        context: result_context,
        callback: result,
    };
    deliver_create_wallet_outcome(outcome, completion);
    WalletEngineAbiStatus::Ok
}

/// Releases a client-driven wallet-creation operation. Passing null is a no-op.
///
/// Dropping a pending operation cancels its Rust future. The client must
/// externally synchronize this function with all other uses of the same raw
/// handle.
///
/// # Safety
///
/// `operation` must be null or a live pointer returned by
/// [`wallet_engine_lifecycle_create_wallet_start`] that has not been freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wallet_engine_create_wallet_operation_free(
    operation: *mut WalletEngineCreateWalletOperation,
) {
    drop(catch_unwind(AssertUnwindSafe(|| {
        if operation.is_null() {
            return;
        }

        // SAFETY: The caller transfers back the unique Box ownership obtained
        // from `wallet_engine_lifecycle_create_wallet_start`.
        drop(unsafe { Box::from_raw(operation) });
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
#[deprecated(
    note = "use wallet_engine_lifecycle_create_wallet_start, wallet_engine_create_wallet_operation_poll, and wallet_engine_create_wallet_operation_free"
)]
#[unsafe(no_mangle)]
#[allow(deprecated, reason = "implements the deprecated asynchronous C ABI")]
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

#[allow(deprecated, reason = "implements the deprecated asynchronous C ABI")]
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
    let outcome = lifecycle.create_wallet(request).await;
    deliver_create_wallet_outcome(outcome, completion);
}
