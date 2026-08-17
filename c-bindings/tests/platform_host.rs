#![allow(unsafe_code)]
#![allow(clippy::expect_used)]

use std::{
    ffi::c_void,
    future::Future,
    mem::size_of,
    pin::pin,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicPtr, AtomicUsize, Ordering},
    },
    task::{Context, Poll, Wake, Waker},
};

use wallet_engine::{
    ProtectedSecretHostError, ProtectedSecretHostErrorKind, ProtectedSecretRef,
    ProtectedSecretStore, WalletPlatformHost,
};
use wallet_engine_c::{
    WALLET_ENGINE_PLATFORM_HOST_CALLBACKS_SIZE,
    WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_UNAVAILABLE, WalletEngineAbiStatus,
    WalletEnginePlatformHostAdapter, WalletEnginePlatformHostCallbacks,
    WalletEngineProtectedSecretHostErrorView, WalletEngineProtectedSecretStoreCompletion,
    WalletEngineProtectedSecretStoreView, WalletEngineStoreProtectedSecretFn,
    WalletEngineStringView, wallet_engine_protected_secret_store_completion_complete,
    wallet_engine_protected_secret_store_completion_free,
};

#[derive(Debug, PartialEq, Eq)]
struct StoredRequest {
    secret_ref: String,
    bytes: Vec<u8>,
    require_user_presence: bool,
}

#[derive(Default)]
struct TestContext {
    retains: AtomicUsize,
    releases: AtomicUsize,
    stores: AtomicUsize,
    captured_completion: AtomicPtr<WalletEngineProtectedSecretStoreCompletion>,
    requests: Mutex<Vec<StoredRequest>>,
    completion_statuses: Mutex<Vec<WalletEngineAbiStatus>>,
}

#[derive(Default)]
struct WakeCounter {
    calls: AtomicUsize,
}

impl Wake for WakeCounter {
    fn wake(self: Arc<Self>) {
        self.calls.fetch_add(1, Ordering::Relaxed);
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.calls.fetch_add(1, Ordering::Relaxed);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn poll_once<F: Future>(future: F) -> Option<F::Output> {
    let mut future = pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    match future.as_mut().poll(&mut context) {
        Poll::Ready(output) => Some(output),
        Poll::Pending => None,
    }
}

fn run_to_completion<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => std::thread::yield_now(),
        }
    }
}

unsafe fn test_context<'a>(context: *mut c_void) -> &'a TestContext {
    // SAFETY: Every test callback table uses a live `TestContext` pointer.
    unsafe { &*context.cast::<TestContext>() }
}

unsafe extern "C" fn retain_context(context: *mut c_void) {
    // SAFETY: The callback table supplies a live `TestContext` pointer.
    let context = unsafe { test_context(context) };
    context.retains.fetch_add(1, Ordering::Relaxed);
}

unsafe extern "C" fn release_context(context: *mut c_void) {
    // SAFETY: The callback table supplies a live `TestContext` pointer.
    let context = unsafe { test_context(context) };
    context.releases.fetch_add(1, Ordering::Relaxed);
}

unsafe fn record_request(
    context: *mut c_void,
    request: *const WalletEngineProtectedSecretStoreView,
) {
    // SAFETY: The adapter passes a non-null request valid for this callback.
    let request = unsafe { *request };
    // SAFETY: Nested request views remain valid for this callback.
    let secret_ref = unsafe { request.secret_ref.value.try_to_string() };
    // SAFETY: Nested request views remain valid for this callback.
    let bytes = unsafe { request.bytes.try_to_vec() };
    let (Ok(secret_ref), Ok(bytes)) = (secret_ref, bytes) else {
        panic!("adapter supplied malformed protected-secret views");
    };
    let stored = StoredRequest {
        secret_ref,
        bytes,
        require_user_presence: request.require_user_presence,
    };
    // SAFETY: The callback table supplies a live `TestContext` pointer.
    let context = unsafe { test_context(context) };
    context.stores.fetch_add(1, Ordering::Relaxed);
    lock(&context.requests).push(stored);
}

unsafe extern "C" fn store_success(
    context: *mut c_void,
    completion: *mut WalletEngineProtectedSecretStoreCompletion,
    request: *const WalletEngineProtectedSecretStoreView,
) {
    // SAFETY: The adapter supplies a callback-scoped request.
    unsafe { record_request(context, request) };
    // SAFETY: Null denotes successful completion and does not get dereferenced.
    let status = unsafe {
        wallet_engine_protected_secret_store_completion_complete(completion, std::ptr::null())
    };
    // SAFETY: The host owns this live completion handle and no other call uses
    // it now.
    unsafe { wallet_engine_protected_secret_store_completion_free(completion) };
    // SAFETY: The callback table supplies a live `TestContext` pointer.
    lock(&unsafe { test_context(context) }.completion_statuses).push(status);
}

unsafe extern "C" fn store_async_success(
    context: *mut c_void,
    completion: *mut WalletEngineProtectedSecretStoreCompletion,
    request: *const WalletEngineProtectedSecretStoreView,
) {
    // SAFETY: The adapter supplies a callback-scoped request.
    unsafe { record_request(context, request) };
    let completion_address = completion as usize;
    std::thread::spawn(move || {
        let completion = completion_address as *mut WalletEngineProtectedSecretStoreCompletion;
        // SAFETY: Null denotes successful completion and does not get
        // dereferenced. The host owns the completion handle while the future
        // is pending.
        let status = unsafe {
            wallet_engine_protected_secret_store_completion_complete(completion, std::ptr::null())
        };
        assert_eq!(status, WalletEngineAbiStatus::Ok);
        // SAFETY: This thread owns the handle and has finished using it.
        unsafe { wallet_engine_protected_secret_store_completion_free(completion) };
    });
}

unsafe extern "C" fn store_error(
    context: *mut c_void,
    completion: *mut WalletEngineProtectedSecretStoreCompletion,
    request: *const WalletEngineProtectedSecretStoreView,
) {
    // SAFETY: The adapter supplies a callback-scoped request.
    unsafe { record_request(context, request) };
    let error = WalletEngineProtectedSecretHostErrorView {
        kind: WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_UNAVAILABLE,
        diagnostic: WalletEngineStringView::from("keychain unavailable"),
    };
    // SAFETY: `error` and its literal-backed diagnostic are readable for this
    // call.
    let status =
        unsafe { wallet_engine_protected_secret_store_completion_complete(completion, &error) };
    // SAFETY: The host owns this live completion handle and no other call uses
    // it now.
    unsafe { wallet_engine_protected_secret_store_completion_free(completion) };
    // SAFETY: The callback table supplies a live `TestContext` pointer.
    lock(&unsafe { test_context(context) }.completion_statuses).push(status);
}

unsafe extern "C" fn store_invalid_then_success(
    context: *mut c_void,
    completion: *mut WalletEngineProtectedSecretStoreCompletion,
    request: *const WalletEngineProtectedSecretStoreView,
) {
    // SAFETY: The adapter supplies a callback-scoped request.
    unsafe { record_request(context, request) };
    let invalid_error = WalletEngineProtectedSecretHostErrorView {
        kind: u32::MAX,
        diagnostic: WalletEngineStringView::empty(),
    };
    // SAFETY: `invalid_error` is readable and its empty view is not
    // dereferenced.
    let invalid = unsafe {
        wallet_engine_protected_secret_store_completion_complete(completion, &invalid_error)
    };
    // SAFETY: Null denotes success. The invalid attempt above did not complete
    // the handle.
    let success = unsafe {
        wallet_engine_protected_secret_store_completion_complete(completion, std::ptr::null())
    };
    // SAFETY: The successful attempt completed the handle, so this call is
    // expected to reject the duplicate without dereferencing the null error.
    let duplicate = unsafe {
        wallet_engine_protected_secret_store_completion_complete(completion, std::ptr::null())
    };
    // SAFETY: The host owns this live completion handle and no other call uses
    // it now.
    unsafe { wallet_engine_protected_secret_store_completion_free(completion) };
    // SAFETY: The callback table supplies a live `TestContext` pointer.
    lock(&unsafe { test_context(context) }.completion_statuses)
        .extend([invalid, success, duplicate]);
}

unsafe extern "C" fn store_without_completion(
    context: *mut c_void,
    completion: *mut WalletEngineProtectedSecretStoreCompletion,
    request: *const WalletEngineProtectedSecretStoreView,
) {
    // SAFETY: The adapter supplies a callback-scoped request.
    unsafe { record_request(context, request) };
    // SAFETY: The callback table supplies a live `TestContext` pointer.
    unsafe { test_context(context) }
        .captured_completion
        .store(completion, Ordering::Relaxed);
}

unsafe extern "C" fn cancel_store(
    context: *mut c_void,
    completion: *mut WalletEngineProtectedSecretStoreCompletion,
    request: *const WalletEngineProtectedSecretStoreView,
) {
    // SAFETY: The adapter supplies a callback-scoped request.
    unsafe { record_request(context, request) };
    // SAFETY: The host owns this live completion handle and no other call uses
    // it now. Freeing it without completion reports cancellation.
    unsafe { wallet_engine_protected_secret_store_completion_free(completion) };
}

fn callback_table(
    context: &TestContext,
    store: WalletEngineStoreProtectedSecretFn,
) -> WalletEnginePlatformHostCallbacks {
    WalletEnginePlatformHostCallbacks {
        struct_size: WALLET_ENGINE_PLATFORM_HOST_CALLBACKS_SIZE,
        context: std::ptr::from_ref(context).cast_mut().cast(),
        retain: Some(retain_context),
        release: Some(release_context),
        store_protected_secret: store,
    }
}

unsafe fn adapter(
    callbacks: &WalletEnginePlatformHostCallbacks,
) -> WalletEnginePlatformHostAdapter {
    // SAFETY: The table and its context remain live for the returned adapter's
    // use in each test.
    match unsafe { WalletEnginePlatformHostAdapter::try_from_callbacks(callbacks) } {
        Ok(adapter) => adapter,
        Err(status) => panic!("valid callback table was rejected: {status:?}"),
    }
}

fn store_request() -> ProtectedSecretStore {
    ProtectedSecretStore {
        secret_ref: ProtectedSecretRef {
            value: "wallet:wallet-1:mnemonic".to_owned(),
        },
        bytes: b"secret bytes".to_vec(),
        require_user_presence: true,
    }
}

#[test]
fn callback_table_is_validated_before_context_is_retained() {
    let context = TestContext::default();
    let callbacks = callback_table(&context, Some(store_success));

    assert_eq!(
        WALLET_ENGINE_PLATFORM_HOST_CALLBACKS_SIZE,
        size_of::<WalletEnginePlatformHostCallbacks>()
    );

    // SAFETY: A null callback-table pointer is explicitly supported as an
    // invalid argument.
    let null = unsafe { WalletEnginePlatformHostAdapter::try_from_callbacks(std::ptr::null()) };
    assert!(matches!(null, Err(WalletEngineAbiStatus::InvalidArgument)));

    let truncated = WalletEnginePlatformHostCallbacks {
        struct_size: WALLET_ENGINE_PLATFORM_HOST_CALLBACKS_SIZE - 1,
        ..callbacks
    };
    // SAFETY: `callbacks` is readable, but deliberately advertises a truncated
    // table.
    let truncated = unsafe { WalletEnginePlatformHostAdapter::try_from_callbacks(&truncated) };
    assert!(matches!(
        truncated,
        Err(WalletEngineAbiStatus::InvalidArgument)
    ));

    for incomplete in [
        WalletEnginePlatformHostCallbacks {
            retain: None,
            ..callbacks
        },
        WalletEnginePlatformHostCallbacks {
            release: None,
            ..callbacks
        },
        WalletEnginePlatformHostCallbacks {
            store_protected_secret: None,
            ..callbacks
        },
    ] {
        // SAFETY: Each complete readable table deliberately omits one required
        // function.
        let result = unsafe { WalletEnginePlatformHostAdapter::try_from_callbacks(&incomplete) };
        assert!(matches!(
            result,
            Err(WalletEngineAbiStatus::InvalidArgument)
        ));
    }
    assert_eq!(context.retains.load(Ordering::Relaxed), 0);
    assert_eq!(context.releases.load(Ordering::Relaxed), 0);
}

#[test]
fn adapter_retains_context_once_across_clones() {
    let context = TestContext::default();
    let callbacks = callback_table(&context, Some(store_success));
    // SAFETY: The callback table and context remain live through this test.
    let host = unsafe { adapter(&callbacks) };
    let clone = host.clone();

    assert_eq!(context.retains.load(Ordering::Relaxed), 1);
    drop(host);
    assert_eq!(context.releases.load(Ordering::Relaxed), 0);
    drop(clone);
    assert_eq!(context.releases.load(Ordering::Relaxed), 1);
}

#[test]
fn store_request_can_complete_synchronously() {
    let context = TestContext::default();
    let callbacks = callback_table(&context, Some(store_success));
    // SAFETY: The callback table and context remain live through this test.
    let host = unsafe { adapter(&callbacks) };

    assert_eq!(
        run_to_completion(host.store_protected_secret(store_request())),
        Ok(())
    );
    assert_eq!(context.stores.load(Ordering::Relaxed), 1);
    assert_eq!(
        lock(&context.requests).as_slice(),
        [StoredRequest {
            secret_ref: "wallet:wallet-1:mnemonic".to_owned(),
            bytes: b"secret bytes".to_vec(),
            require_user_presence: true,
        }]
    );
    assert_eq!(
        lock(&context.completion_statuses).as_slice(),
        [WalletEngineAbiStatus::Ok]
    );
}

#[test]
fn store_request_can_complete_from_another_thread() {
    let context = TestContext::default();
    let callbacks = callback_table(&context, Some(store_async_success));
    // SAFETY: The callback table and context remain live through this test.
    let host = unsafe { adapter(&callbacks) };

    assert_eq!(
        run_to_completion(host.store_protected_secret(store_request())),
        Ok(())
    );
    assert_eq!(context.stores.load(Ordering::Relaxed), 1);
}

#[test]
fn store_error_is_copied_into_the_core_type() {
    let context = TestContext::default();
    let callbacks = callback_table(&context, Some(store_error));
    // SAFETY: The callback table and context remain live through this test.
    let host = unsafe { adapter(&callbacks) };

    assert_eq!(
        run_to_completion(host.store_protected_secret(store_request())),
        Err(ProtectedSecretHostError::Failed {
            kind: ProtectedSecretHostErrorKind::Unavailable,
            diagnostic: "keychain unavailable".to_owned(),
        })
    );
    assert_eq!(
        lock(&context.completion_statuses).as_slice(),
        [WalletEngineAbiStatus::Ok]
    );
}

#[test]
fn invalid_and_duplicate_completions_are_rejected() {
    let context = TestContext::default();
    let callbacks = callback_table(&context, Some(store_invalid_then_success));
    // SAFETY: The callback table and context remain live through this test.
    let host = unsafe { adapter(&callbacks) };

    assert_eq!(
        run_to_completion(host.store_protected_secret(store_request())),
        Ok(())
    );
    assert_eq!(
        lock(&context.completion_statuses).as_slice(),
        [
            WalletEngineAbiStatus::InvalidArgument,
            WalletEngineAbiStatus::Ok,
            WalletEngineAbiStatus::InvalidArgument,
        ]
    );

    // SAFETY: Null is rejected without being dereferenced.
    let null = unsafe {
        wallet_engine_protected_secret_store_completion_complete(
            std::ptr::null_mut(),
            std::ptr::null(),
        )
    };
    assert_eq!(null, WalletEngineAbiStatus::InvalidArgument);
}

#[test]
fn completion_remains_safe_after_dropping_the_host_future() {
    let context = TestContext::default();
    let callbacks = callback_table(&context, Some(store_without_completion));
    // SAFETY: The callback table and context remain live through this test.
    let host = unsafe { adapter(&callbacks) };

    let result = poll_once(host.store_protected_secret(store_request()));
    assert_eq!(result, None);
    let completion = context.captured_completion.load(Ordering::Relaxed);
    assert!(!completion.is_null());
    // SAFETY: The callback transferred this live handle to the test. The
    // receiver was dropped with the future, so completion is safely rejected.
    let status = unsafe {
        wallet_engine_protected_secret_store_completion_complete(completion, std::ptr::null())
    };
    assert_eq!(status, WalletEngineAbiStatus::InvalidArgument);
    // SAFETY: The test owns the handle and has finished using it.
    unsafe { wallet_engine_protected_secret_store_completion_free(completion) };
}

#[test]
fn completion_records_result_without_waking_the_operation() {
    let context = TestContext::default();
    let callbacks = callback_table(&context, Some(store_without_completion));
    // SAFETY: The callback table and context remain live through this test.
    let host = unsafe { adapter(&callbacks) };
    let mut future = pin!(host.store_protected_secret(store_request()));
    let wake_counter = Arc::new(WakeCounter::default());
    let waker = Waker::from(Arc::clone(&wake_counter));
    let mut task_context = Context::from_waker(&waker);

    assert_eq!(future.as_mut().poll(&mut task_context), Poll::Pending);
    let completion = context.captured_completion.load(Ordering::Acquire);
    assert!(!completion.is_null());
    // SAFETY: The callback transferred this live handle to the test.
    let status = unsafe {
        wallet_engine_protected_secret_store_completion_complete(completion, std::ptr::null())
    };
    assert_eq!(status, WalletEngineAbiStatus::Ok);
    assert_eq!(wake_counter.calls.load(Ordering::Relaxed), 0);
    assert_eq!(future.as_mut().poll(&mut task_context), Poll::Ready(Ok(())));
    // SAFETY: Completion returned and this test uniquely owns the handle.
    unsafe { wallet_engine_protected_secret_store_completion_free(completion) };
}

#[test]
fn freeing_without_completion_reports_cancellation() {
    let context = TestContext::default();
    let callbacks = callback_table(&context, Some(cancel_store));
    // SAFETY: The callback table and context remain live through this test.
    let host = unsafe { adapter(&callbacks) };

    assert_eq!(
        run_to_completion(host.store_protected_secret(store_request())),
        Err(ProtectedSecretHostError::Failed {
            kind: ProtectedSecretHostErrorKind::Cancelled,
            diagnostic: "protected-secret store completion was released without a result"
                .to_owned(),
        })
    );
}
