//! C callback adapter for platform-owned services.

use std::{
    ffi::c_void,
    future::Future,
    mem::size_of,
    panic::{AssertUnwindSafe, catch_unwind},
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};

use wallet_engine::{
    JournalCompareExchange, JournalCompareExchangeResult, JournalHostError, JournalHostErrorKind,
    JournalKey, JournalRecord, ProtectedSecretHostError, ProtectedSecretHostErrorKind,
    ProtectedSecretRead, ProtectedSecretRef, ProtectedSecretStore, WalletPlatformHost,
};

use crate::{
    WalletEngineAbiStatus, WalletEngineProtectedSecretHostErrorView,
    WalletEngineProtectedSecretStoreView,
};

/// Retains a consumer-owned callback context.
pub type WalletEngineContextRetainFn = Option<unsafe extern "C" fn(context: *mut c_void)>;

/// Releases a consumer-owned callback context.
pub type WalletEngineContextReleaseFn = Option<unsafe extern "C" fn(context: *mut c_void)>;

/// Requests storage of protected secret bytes.
///
/// The request and all nested views remain valid only until this function
/// returns. Ownership of `completion` transfers to the host. The host must
/// eventually release it with
/// [`wallet_engine_protected_secret_store_completion_free`], optionally after
/// completing it with
/// [`wallet_engine_protected_secret_store_completion_complete`].
pub type WalletEngineStoreProtectedSecretFn = Option<
    unsafe extern "C" fn(
        context: *mut c_void,
        completion: *mut WalletEngineProtectedSecretStoreCompletion,
        request: *const WalletEngineProtectedSecretStoreView,
    ),
>;

/// Versionable callbacks supplied by the C platform host.
///
/// The callbacks and `context` must be safe to use from arbitrary client-owned
/// threads that call Wallet Engine APIs. The library creates no callback
/// threads. Set `struct_size` to `sizeof(WalletEnginePlatformHostCallbacks)`.
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
    /// Required function that stores protected secret bytes.
    pub store_protected_secret: WalletEngineStoreProtectedSecretFn,
}

/// Size of the platform-host callback table implemented by this ABI version.
pub const WALLET_ENGINE_PLATFORM_HOST_CALLBACKS_SIZE: usize =
    size_of::<WalletEnginePlatformHostCallbacks>();

type ProtectedSecretStoreResult = Result<(), ProtectedSecretHostError>;

struct ProtectedSecretStoreCompletionState {
    inner: Mutex<ProtectedSecretStoreCompletionStateInner>,
}

struct ProtectedSecretStoreCompletionStateInner {
    receiver_alive: bool,
    completed: bool,
    result: Option<ProtectedSecretStoreResult>,
}

impl ProtectedSecretStoreCompletionState {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(ProtectedSecretStoreCompletionStateInner {
                receiver_alive: true,
                completed: false,
                result: None,
            }),
        })
    }

    fn complete(&self, result: ProtectedSecretStoreResult) -> WalletEngineAbiStatus {
        let mut inner = match self.inner.lock() {
            Ok(inner) => inner,
            Err(poisoned) => poisoned.into_inner(),
        };
        if inner.completed || !inner.receiver_alive {
            return WalletEngineAbiStatus::InvalidArgument;
        }

        inner.completed = true;
        inner.result = Some(result);
        WalletEngineAbiStatus::Ok
    }

    fn cancel_if_pending(&self) {
        let mut inner = match self.inner.lock() {
            Ok(inner) => inner,
            Err(poisoned) => poisoned.into_inner(),
        };
        if inner.completed || !inner.receiver_alive {
            return;
        }

        inner.completed = true;
        inner.result = Some(cancelled_protected_secret_store());
    }
}

struct ProtectedSecretStoreReceiver {
    state: Arc<ProtectedSecretStoreCompletionState>,
}

impl Future for ProtectedSecretStoreReceiver {
    type Output = ProtectedSecretStoreResult;

    fn poll(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        let mut inner = match self.state.inner.lock() {
            Ok(inner) => inner,
            Err(poisoned) => poisoned.into_inner(),
        };
        inner.result.take().map_or(Poll::Pending, Poll::Ready)
    }
}

impl Drop for ProtectedSecretStoreReceiver {
    fn drop(&mut self) {
        let mut inner = match self.state.inner.lock() {
            Ok(inner) => inner,
            Err(poisoned) => poisoned.into_inner(),
        };
        inner.receiver_alive = false;
        drop(inner.result.take());
    }
}

/// Owned completion for one protected-secret store request.
///
/// The handle has no thread affinity. `complete` may be called from any
/// client-owned thread, but `free` must be externally synchronized with every
/// other use of the same raw handle.
pub struct WalletEngineProtectedSecretStoreCompletion {
    state: Arc<ProtectedSecretStoreCompletionState>,
}

fn cancelled_protected_secret_store() -> ProtectedSecretStoreResult {
    Err(ProtectedSecretHostError::Failed {
        kind: ProtectedSecretHostErrorKind::Cancelled,
        diagnostic: "protected-secret store completion was released without a result".to_owned(),
    })
}

/// Completes a protected-secret store request previously issued to the host.
///
/// Pass a null `error` on success. A non-null error and its diagnostic are
/// copied before this function returns. Only the first valid completion is
/// accepted. This function does not release the handle; the host must still
/// call [`wallet_engine_protected_secret_store_completion_free`]. Completion
/// only records the result and never schedules or polls the owning operation.
///
/// # Safety
///
/// `completion` must point to a live completion handle received by the host
/// callback and must not be freed for this call.
///
/// When `error` is non-null, it must point to a readable protected-secret host
/// error view whose diagnostic satisfies the view's safety contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wallet_engine_protected_secret_store_completion_complete(
    completion: *mut WalletEngineProtectedSecretStoreCompletion,
    error: *const WalletEngineProtectedSecretHostErrorView,
) -> WalletEngineAbiStatus {
    catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: The caller upholds the completion-handle, error-pointer, and
        // nested-view contracts.
        unsafe { complete_protected_secret_store(completion, error) }
    }))
    .unwrap_or(WalletEngineAbiStatus::Panic)
}

unsafe fn complete_protected_secret_store(
    completion: *mut WalletEngineProtectedSecretStoreCompletion,
    error: *const WalletEngineProtectedSecretHostErrorView,
) -> WalletEngineAbiStatus {
    if completion.is_null() {
        return WalletEngineAbiStatus::InvalidArgument;
    }

    let result = if error.is_null() {
        Ok(())
    } else {
        // SAFETY: The caller guarantees that `error` points to a readable value.
        let error = unsafe { *error };
        // SAFETY: The caller guarantees the nested diagnostic view is readable.
        match unsafe { error.try_to_core() } {
            Ok(error) => Err(error),
            Err(status) => return status,
        }
    };

    // SAFETY: The caller guarantees a live completion handle for this call.
    let completion = unsafe { &*completion };
    completion.state.complete(result)
}

/// Releases a protected-secret store completion. Passing null is a no-op.
///
/// Releasing a completion before a successful `complete` reports cancellation
/// to the owning operation. It does not poll or schedule that operation. This
/// function must be externally synchronized with all other uses of the same
/// raw handle.
///
/// # Safety
///
/// `completion` must be null or a live completion handle received by the host
/// callback that has not already been freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wallet_engine_protected_secret_store_completion_free(
    completion: *mut WalletEngineProtectedSecretStoreCompletion,
) {
    drop(catch_unwind(AssertUnwindSafe(|| {
        if completion.is_null() {
            return;
        }

        // SAFETY: The caller transfers back the unique Box ownership received
        // by the host callback.
        let completion = unsafe { Box::from_raw(completion) };
        completion.state.cancel_if_pending();
    })));
}

struct PlatformHostInner {
    callbacks: WalletEnginePlatformHostCallbacks,
}

// SAFETY: Construction requires callbacks that support arbitrary worker
// threads. The opaque context remains alive through the paired retain/release
// callbacks, and Rust never dereferences it.
unsafe impl Send for PlatformHostInner {}
// SAFETY: See the `Send` implementation. All access to the callback table is
// immutable after construction.
unsafe impl Sync for PlatformHostInner {}

impl Drop for PlatformHostInner {
    fn drop(&mut self) {
        if let Some(release) = self.callbacks.release {
            // SAFETY: The constructor retained this context once, and the
            // callback contract requires the paired release to accept it.
            unsafe { release(self.callbacks.context) };
        }
    }
}

/// Rust adapter that forwards [`WalletPlatformHost`] requests to C callbacks.
#[derive(Clone)]
pub struct WalletEnginePlatformHostAdapter {
    inner: Arc<PlatformHostInner>,
}

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

        // SAFETY: The size check and caller contract guarantee the complete
        // current callback-table prefix is readable.
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
        Ok(Self {
            inner: Arc::new(PlatformHostInner { callbacks }),
        })
    }
}

fn unsupported_protected_secret(operation: &str) -> ProtectedSecretHostError {
    ProtectedSecretHostError::Failed {
        kind: ProtectedSecretHostErrorKind::Other,
        diagnostic: format!("C ABI platform callback is unavailable: {operation}"),
    }
}

fn unsupported_journal(operation: &str) -> JournalHostError {
    JournalHostError::Failed {
        kind: JournalHostErrorKind::Other,
        diagnostic: format!("C ABI platform callback is unavailable: {operation}"),
    }
}

#[async_trait::async_trait]
impl WalletPlatformHost for WalletEnginePlatformHostAdapter {
    async fn read_protected_secret(
        &self,
        _request: ProtectedSecretRead,
    ) -> Result<Vec<u8>, ProtectedSecretHostError> {
        Err(unsupported_protected_secret("read_protected_secret"))
    }

    async fn store_protected_secret(
        &self,
        request: ProtectedSecretStore,
    ) -> Result<(), ProtectedSecretHostError> {
        let Some(store) = self.inner.callbacks.store_protected_secret else {
            return Err(unsupported_protected_secret("store_protected_secret"));
        };
        let state = ProtectedSecretStoreCompletionState::new();
        let receiver = ProtectedSecretStoreReceiver {
            state: Arc::clone(&state),
        };
        let completion = Box::new(WalletEngineProtectedSecretStoreCompletion { state });
        let completion = Box::into_raw(completion);
        {
            let request = WalletEngineProtectedSecretStoreView::from(&request);

            // SAFETY: The callback was validated during construction. Its
            // context remains retained by `self.inner`, and `request` lives
            // until the call returns. Ownership of `completion` transfers to
            // the callback.
            unsafe { store(self.inner.callbacks.context, completion, &request) };
        }

        receiver.await
    }

    async fn delete_protected_secret(
        &self,
        _secret_ref: ProtectedSecretRef,
    ) -> Result<(), ProtectedSecretHostError> {
        Err(unsupported_protected_secret("delete_protected_secret"))
    }

    async fn load_journal(
        &self,
        _key: JournalKey,
    ) -> Result<Option<JournalRecord>, JournalHostError> {
        Err(unsupported_journal("load_journal"))
    }

    async fn compare_exchange_journal(
        &self,
        _mutation: JournalCompareExchange,
    ) -> Result<JournalCompareExchangeResult, JournalHostError> {
        Err(unsupported_journal("compare_exchange_journal"))
    }
}
