//! Opaque C handle for the wallet lifecycle service.

use std::{
    ffi::c_void,
    future::Future,
    panic::{AssertUnwindSafe, catch_unwind},
    pin::Pin,
    sync::{Arc, Mutex, TryLockError},
    task::{Context, Poll, Waker},
};

use wallet_engine::{
    CreatedWallet, WalletDescriptor, WalletLifecycle as CoreWalletLifecycle, WalletLifecycleError,
};

use crate::{
    WalletEngineAbiStatus, WalletEngineCreateWalletRequest, WalletEngineCreatedWalletView,
    WalletEngineImportWalletRequest, WalletEnginePlatformHostAdapter,
    WalletEnginePlatformHostCallbacks, WalletEngineWalletDescriptorView,
    WalletEngineWalletLifecycleErrorView, with_created_wallet_view,
};

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

type WalletOperationOutcome<T> = Result<T, WalletLifecycleError>;
type WalletOperationFuture<T> =
    Pin<Box<dyn Future<Output = WalletOperationOutcome<T>> + Send + 'static>>;

struct ClientDrivenOperation<T> {
    future: Mutex<Option<WalletOperationFuture<T>>>,
}

enum ClientDrivenPoll<T> {
    Pending,
    Ready(WalletOperationOutcome<T>),
}

impl<T> ClientDrivenOperation<T> {
    fn new(future: WalletOperationFuture<T>) -> Self {
        Self {
            future: Mutex::new(Some(future)),
        }
    }

    fn poll_once(&self) -> Result<ClientDrivenPoll<T>, WalletEngineAbiStatus> {
        let mut future = match self.future.try_lock() {
            Ok(future) => future,
            Err(TryLockError::WouldBlock) => return Err(WalletEngineAbiStatus::OperationBusy),
            Err(TryLockError::Poisoned(_)) => return Err(WalletEngineAbiStatus::Panic),
        };
        let Some(active) = future.as_mut() else {
            return Err(WalletEngineAbiStatus::InvalidArgument);
        };

        let poll = catch_unwind(AssertUnwindSafe(|| {
            let mut context = Context::from_waker(Waker::noop());
            active.as_mut().poll(&mut context)
        }));
        let result = match poll {
            Ok(Poll::Pending) => Ok(ClientDrivenPoll::Pending),
            Ok(Poll::Ready(outcome)) => {
                *future = None;
                Ok(ClientDrivenPoll::Ready(outcome))
            }
            Err(_) => {
                *future = None;
                Err(WalletEngineAbiStatus::Panic)
            }
        };
        drop(future);
        result
    }
}

/// Opaque client-driven wallet-creation operation.
///
/// Create it with [`wallet_engine_lifecycle_create_wallet_start`], advance it
/// with [`wallet_engine_create_wallet_operation_poll`], and release it with
/// [`wallet_engine_create_wallet_operation_free`]. The handle has no thread
/// affinity, but the same operation must not be freed concurrently with any
/// other call that uses it.
pub struct WalletEngineCreateWalletOperation {
    inner: ClientDrivenOperation<CreatedWallet>,
}

/// Receives an imported wallet descriptor synchronously from an explicit poll.
///
/// This callback is never retained. It runs only when
/// [`wallet_engine_import_wallet_operation_poll`] returns `READY`, on the
/// thread that called that poll function. `abi_status` is `OK`; boundary
/// failures are returned directly by the poll function.
///
/// The descriptor, error, and all nested views remain valid only until the
/// callback returns.
pub type WalletEngineImportWalletResultFn = Option<
    unsafe extern "C" fn(
        context: *mut c_void,
        abi_status: WalletEngineAbiStatus,
        descriptor: *const WalletEngineWalletDescriptorView,
        error: *const WalletEngineWalletLifecycleErrorView,
    ),
>;

/// Opaque client-driven wallet-import operation.
///
/// Create it with [`wallet_engine_lifecycle_import_wallet_start`], advance it
/// with [`wallet_engine_import_wallet_operation_poll`], and release it with
/// [`wallet_engine_import_wallet_operation_free`]. The handle has no thread
/// affinity, but `free` requires external synchronization with every other use
/// of the same raw handle.
pub struct WalletEngineImportWalletOperation {
    inner: ClientDrivenOperation<WalletDescriptor>,
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

fn deliver_create_wallet_outcome(
    outcome: WalletOperationOutcome<CreatedWallet>,
    completion: CreateWalletCompletion,
) {
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

#[derive(Clone, Copy)]
struct ImportWalletCompletion {
    context: *mut c_void,
    callback: unsafe extern "C" fn(
        *mut c_void,
        WalletEngineAbiStatus,
        *const WalletEngineWalletDescriptorView,
        *const WalletEngineWalletLifecycleErrorView,
    ),
}

impl ImportWalletCompletion {
    unsafe fn call(
        self,
        abi_status: WalletEngineAbiStatus,
        descriptor: *const WalletEngineWalletDescriptorView,
        error: *const WalletEngineWalletLifecycleErrorView,
    ) {
        // SAFETY: The C caller guarantees the callback accepts its context and
        // callback-scoped result views.
        unsafe { (self.callback)(self.context, abi_status, descriptor, error) };
    }
}

fn deliver_import_wallet_outcome(
    outcome: WalletOperationOutcome<WalletDescriptor>,
    completion: ImportWalletCompletion,
) {
    match outcome {
        Ok(descriptor) => {
            let view = WalletEngineWalletDescriptorView::from(&descriptor);
            // SAFETY: `view` and all nested views borrow `descriptor`, which
            // remains live for this callback invocation.
            unsafe { completion.call(WalletEngineAbiStatus::Ok, &view, std::ptr::null()) };
        }
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
/// [`wallet_engine_lifecycle_free`]. The handle has no thread affinity. A live
/// lifecycle may be shared by client threads that concurrently create distinct
/// operations. `free` requires exclusive ownership and external
/// synchronization with every other use of the raw handle.
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
/// The client must externally synchronize this function with every other use
/// of the same raw lifecycle handle.
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
/// On success, the client owns `out_operation` until the matching free. The
/// same live lifecycle may be used concurrently to create distinct operations
/// from multiple client-owned threads.
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
    let future: WalletOperationFuture<CreatedWallet> =
        Box::pin(async move { lifecycle.create_wallet(request).await });
    let operation = Box::new(WalletEngineCreateWalletOperation {
        inner: ClientDrivenOperation::new(future),
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
/// waiting. Different operation handles may be polled concurrently. Polling
/// uses no thread-local runtime state and performs work only on the calling
/// thread.
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
    let outcome = match operation.inner.poll_once() {
        Ok(ClientDrivenPoll::Pending) => return WalletEngineAbiStatus::Ok,
        Ok(ClientDrivenPoll::Ready(outcome)) => outcome,
        Err(status) => return status,
    };

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
/// Dropping a pending operation cancels and destroys its state. The client
/// must externally synchronize this function with all other uses of the same
/// raw handle.
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

/// Creates a client-driven wallet-import operation without polling it.
///
/// The record ID and every recovery word are copied, and the lifecycle service
/// is retained, before this function returns. No platform or result callback is
/// invoked. On success, the client owns `out_operation` until the matching
/// free. A live lifecycle may create distinct operations concurrently.
///
/// # Safety
///
/// `lifecycle` must point to a live lifecycle handle for this call. `request`
/// and all nested views must be readable for this call. `out_operation` must
/// point to writable storage for one operation pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wallet_engine_lifecycle_import_wallet_start(
    lifecycle: *const WalletEngineLifecycle,
    request: *const WalletEngineImportWalletRequest,
    out_operation: *mut *mut WalletEngineImportWalletOperation,
) -> WalletEngineAbiStatus {
    catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: The caller upholds the handle, request, and output-pointer
        // contracts documented by this function.
        unsafe { lifecycle_import_wallet_start(lifecycle, request, out_operation) }
    }))
    .unwrap_or(WalletEngineAbiStatus::Panic)
}

unsafe fn lifecycle_import_wallet_start(
    lifecycle: *const WalletEngineLifecycle,
    request: *const WalletEngineImportWalletRequest,
    out_operation: *mut *mut WalletEngineImportWalletOperation,
) -> WalletEngineAbiStatus {
    if out_operation.is_null() {
        return WalletEngineAbiStatus::InvalidArgument;
    }

    // SAFETY: The caller guarantees that `out_operation` is writable.
    unsafe { out_operation.write(std::ptr::null_mut()) };
    if lifecycle.is_null() || request.is_null() {
        return WalletEngineAbiStatus::InvalidArgument;
    }

    // SAFETY: The caller guarantees a readable request, word-view array, and
    // nested string views for this call. Conversion copies all request data.
    let request = match unsafe { request.read().try_to_core() } {
        Ok(request) => request,
        Err(status) => return status,
    };
    // SAFETY: The caller keeps the lifecycle live for this call. The future
    // owns this Arc independently of the raw lifecycle handle afterward.
    let lifecycle = Arc::clone(&unsafe { &*lifecycle }.inner);
    let future: WalletOperationFuture<WalletDescriptor> =
        Box::pin(async move { lifecycle.import_wallet(request).await });
    let operation = Box::new(WalletEngineImportWalletOperation {
        inner: ClientDrivenOperation::new(future),
    });

    // SAFETY: The output pointer is writable and receives unique Box ownership
    // until `wallet_engine_import_wallet_operation_free` is called.
    unsafe { out_operation.write(Box::into_raw(operation)) };
    WalletEngineAbiStatus::Ok
}

/// Polls a wallet-import operation once on the calling thread.
///
/// `PENDING` never invokes `result`. `READY` invokes it exactly once before
/// this function returns. The callback is never retained. Concurrent polls of
/// the same operation return `OPERATION_BUSY`; distinct operation handles may
/// be polled concurrently. No work continues after this method returns.
///
/// # Safety
///
/// `operation` must point to a live operation handle and must not be freed for
/// this call. `out_state` must point to writable storage. `result_context` must
/// be valid for a synchronous invocation of `result` during this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wallet_engine_import_wallet_operation_poll(
    operation: *mut WalletEngineImportWalletOperation,
    result_context: *mut c_void,
    result: WalletEngineImportWalletResultFn,
    out_state: *mut WalletEngineOperationPollState,
) -> WalletEngineAbiStatus {
    catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: The caller upholds the operation, callback, and output
        // contracts documented by this function.
        unsafe { import_wallet_operation_poll(operation, result_context, result, out_state) }
    }))
    .unwrap_or(WalletEngineAbiStatus::Panic)
}

unsafe fn import_wallet_operation_poll(
    operation: *mut WalletEngineImportWalletOperation,
    result_context: *mut c_void,
    result: WalletEngineImportWalletResultFn,
    out_state: *mut WalletEngineOperationPollState,
) -> WalletEngineAbiStatus {
    if out_state.is_null() {
        return WalletEngineAbiStatus::InvalidArgument;
    }
    // SAFETY: The caller guarantees writable output storage. Pending is the
    // conservative state for validation and polling failures.
    unsafe { out_state.write(WalletEngineOperationPollState::Pending) };

    let Some(result) = result else {
        return WalletEngineAbiStatus::InvalidArgument;
    };
    if operation.is_null() {
        return WalletEngineAbiStatus::InvalidArgument;
    }

    // SAFETY: The caller guarantees a live operation for this call.
    let operation = unsafe { &*operation };
    let outcome = match operation.inner.poll_once() {
        Ok(ClientDrivenPoll::Pending) => return WalletEngineAbiStatus::Ok,
        Ok(ClientDrivenPoll::Ready(outcome)) => outcome,
        Err(status) => return status,
    };

    // SAFETY: The caller guarantees writable output storage for this call.
    unsafe { out_state.write(WalletEngineOperationPollState::Ready) };
    let completion = ImportWalletCompletion {
        context: result_context,
        callback: result,
    };
    deliver_import_wallet_outcome(outcome, completion);
    WalletEngineAbiStatus::Ok
}

/// Releases a client-driven wallet-import operation. Passing null is a no-op.
///
/// Dropping a pending operation cancels its state and destroys the copied
/// recovery words. The client must externally synchronize this function with
/// every other use of the same raw handle.
///
/// # Safety
///
/// `operation` must be null or a live pointer returned by
/// [`wallet_engine_lifecycle_import_wallet_start`] that has not been freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wallet_engine_import_wallet_operation_free(
    operation: *mut WalletEngineImportWalletOperation,
) {
    drop(catch_unwind(AssertUnwindSafe(|| {
        if operation.is_null() {
            return;
        }

        // SAFETY: The caller transfers back the unique Box ownership obtained
        // from `wallet_engine_lifecycle_import_wallet_start`.
        drop(unsafe { Box::from_raw(operation) });
    })));
}
