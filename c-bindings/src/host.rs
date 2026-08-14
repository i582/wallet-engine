//! C callback adapter for platform-owned services.

use std::{
    collections::{HashMap, hash_map::Entry},
    ffi::c_void,
    mem::size_of,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Mutex, MutexGuard, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};

use futures::channel::oneshot;
use wallet_engine::{
    JournalCompareExchange, JournalCompareExchangeResult, JournalHostError, JournalHostErrorKind,
    JournalKey, JournalRecord, ProtectedSecretHostError, ProtectedSecretHostErrorKind,
    ProtectedSecretRead, ProtectedSecretRef, ProtectedSecretStore, WalletPlatformHost,
};

use crate::{
    WalletEngineAbiStatus, WalletEngineProtectedSecretHostErrorView,
    WalletEngineProtectedSecretStoreView,
};

/// Identifies one pending asynchronous host completion.
pub type WalletEngineCompletionId = u64;

/// Retains a consumer-owned callback context.
pub type WalletEngineContextRetainFn = Option<unsafe extern "C" fn(context: *mut c_void)>;

/// Releases a consumer-owned callback context.
pub type WalletEngineContextReleaseFn = Option<unsafe extern "C" fn(context: *mut c_void)>;

/// Requests storage of protected secret bytes.
///
/// The request and all nested views remain valid only until this function
/// returns. The host must copy data needed by asynchronous storage, then call
/// `wallet_engine_store_protected_secret_complete` exactly once.
pub type WalletEngineStoreProtectedSecretFn = Option<
    unsafe extern "C" fn(
        context: *mut c_void,
        completion_id: WalletEngineCompletionId,
        request: *const WalletEngineProtectedSecretStoreView,
    ),
>;

/// Versionable callbacks supplied by the C platform host.
///
/// The callbacks and `context` must be safe to use from arbitrary worker
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
type ProtectedSecretStoreSender = oneshot::Sender<ProtectedSecretStoreResult>;

static NEXT_COMPLETION_ID: AtomicU64 = AtomicU64::new(1);
static PROTECTED_SECRET_STORE_COMPLETIONS: OnceLock<
    Mutex<HashMap<WalletEngineCompletionId, ProtectedSecretStoreSender>>,
> = OnceLock::new();

fn completions() -> &'static Mutex<HashMap<WalletEngineCompletionId, ProtectedSecretStoreSender>> {
    PROTECTED_SECRET_STORE_COMPLETIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn lock_completions()
-> MutexGuard<'static, HashMap<WalletEngineCompletionId, ProtectedSecretStoreSender>> {
    match completions().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn register_completion(sender: ProtectedSecretStoreSender) -> WalletEngineCompletionId {
    let mut completions = lock_completions();
    loop {
        let id = NEXT_COMPLETION_ID.fetch_add(1, Ordering::Relaxed);
        if id == 0 {
            continue;
        }

        if let Entry::Vacant(entry) = completions.entry(id) {
            entry.insert(sender);
            return id;
        }
    }
}

fn remove_completion(id: WalletEngineCompletionId) -> Option<ProtectedSecretStoreSender> {
    lock_completions().remove(&id)
}

struct PendingCompletion {
    id: WalletEngineCompletionId,
}

impl Drop for PendingCompletion {
    fn drop(&mut self) {
        remove_completion(self.id);
    }
}

/// Completes a protected-secret store request previously issued to C.
///
/// Pass a null `error` on success. A non-null error and its diagnostic are
/// copied before this function returns.
///
/// # Safety
///
/// When `error` is non-null, it must point to a readable protected-secret host
/// error view whose diagnostic satisfies the view's safety contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wallet_engine_store_protected_secret_complete(
    completion_id: WalletEngineCompletionId,
    error: *const WalletEngineProtectedSecretHostErrorView,
) -> WalletEngineAbiStatus {
    catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: The caller upholds the error pointer and nested-view contract.
        unsafe { complete_protected_secret_store(completion_id, error) }
    }))
    .unwrap_or(WalletEngineAbiStatus::Panic)
}

unsafe fn complete_protected_secret_store(
    completion_id: WalletEngineCompletionId,
    error: *const WalletEngineProtectedSecretHostErrorView,
) -> WalletEngineAbiStatus {
    if completion_id == 0 {
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

    let Some(sender) = remove_completion(completion_id) else {
        return WalletEngineAbiStatus::InvalidArgument;
    };

    if sender.send(result).is_ok() {
        WalletEngineAbiStatus::Ok
    } else {
        WalletEngineAbiStatus::InvalidArgument
    }
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
        let (sender, receiver) = oneshot::channel();
        let completion_id = register_completion(sender);
        let _pending = PendingCompletion { id: completion_id };
        {
            let request = WalletEngineProtectedSecretStoreView::from(&request);

            // SAFETY: The callback was validated during construction. Its
            // context remains retained by `self.inner`, and `request` lives
            // until the call returns.
            unsafe { store(self.inner.callbacks.context, completion_id, &request) };
        }

        receiver.await.unwrap_or_else(|_| {
            Err(unsupported_protected_secret(
                "store_protected_secret completion",
            ))
        })
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
