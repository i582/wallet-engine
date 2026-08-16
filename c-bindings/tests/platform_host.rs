#![allow(unsafe_code)]
#![allow(clippy::expect_used)]

use std::{
    ffi::c_void,
    mem::size_of,
    sync::{
        Mutex, MutexGuard,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
};

use futures::{FutureExt, executor::block_on};
use wallet_engine::{
    ProtectedSecretHostError, ProtectedSecretHostErrorKind, ProtectedSecretRef,
    ProtectedSecretStore, WalletPlatformHost,
};
use wallet_engine_c::{
    WALLET_ENGINE_PLATFORM_HOST_CALLBACKS_SIZE,
    WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_UNAVAILABLE, WalletEngineAbiStatus,
    WalletEngineCompletionId, WalletEnginePlatformHostAdapter, WalletEnginePlatformHostCallbacks,
    WalletEngineProtectedSecretHostErrorView, WalletEngineProtectedSecretStoreView,
    WalletEngineStoreProtectedSecretFn, WalletEngineStringView,
    wallet_engine_store_protected_secret_complete,
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
    captured_completion_id: AtomicU64,
    requests: Mutex<Vec<StoredRequest>>,
    completion_statuses: Mutex<Vec<WalletEngineAbiStatus>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
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
    completion_id: WalletEngineCompletionId,
    request: *const WalletEngineProtectedSecretStoreView,
) {
    // SAFETY: The adapter supplies a callback-scoped request.
    unsafe { record_request(context, request) };
    // SAFETY: Null denotes successful completion and does not get dereferenced.
    let status =
        unsafe { wallet_engine_store_protected_secret_complete(completion_id, std::ptr::null()) };
    // SAFETY: The callback table supplies a live `TestContext` pointer.
    lock(&unsafe { test_context(context) }.completion_statuses).push(status);
}

unsafe extern "C" fn store_async_success(
    context: *mut c_void,
    completion_id: WalletEngineCompletionId,
    request: *const WalletEngineProtectedSecretStoreView,
) {
    // SAFETY: The adapter supplies a callback-scoped request.
    unsafe { record_request(context, request) };
    std::thread::spawn(move || {
        // SAFETY: Null denotes successful completion and does not get
        // dereferenced. The completion ID remains registered while the host
        // future is pending.
        let status = unsafe {
            wallet_engine_store_protected_secret_complete(completion_id, std::ptr::null())
        };
        assert_eq!(status, WalletEngineAbiStatus::Ok);
    });
}

unsafe extern "C" fn store_error(
    context: *mut c_void,
    completion_id: WalletEngineCompletionId,
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
    let status = unsafe { wallet_engine_store_protected_secret_complete(completion_id, &error) };
    // SAFETY: The callback table supplies a live `TestContext` pointer.
    lock(&unsafe { test_context(context) }.completion_statuses).push(status);
}

unsafe extern "C" fn store_invalid_then_success(
    context: *mut c_void,
    completion_id: WalletEngineCompletionId,
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
    let invalid =
        unsafe { wallet_engine_store_protected_secret_complete(completion_id, &invalid_error) };
    // SAFETY: Null denotes success. The invalid attempt above did not consume
    // the completion ID.
    let success =
        unsafe { wallet_engine_store_protected_secret_complete(completion_id, std::ptr::null()) };
    // SAFETY: The successful attempt consumed the ID, so this call is expected
    // to reject it without dereferencing the null error.
    let duplicate =
        unsafe { wallet_engine_store_protected_secret_complete(completion_id, std::ptr::null()) };
    // SAFETY: The callback table supplies a live `TestContext` pointer.
    lock(&unsafe { test_context(context) }.completion_statuses)
        .extend([invalid, success, duplicate]);
}

unsafe extern "C" fn store_without_completion(
    context: *mut c_void,
    completion_id: WalletEngineCompletionId,
    request: *const WalletEngineProtectedSecretStoreView,
) {
    // SAFETY: The adapter supplies a callback-scoped request.
    unsafe { record_request(context, request) };
    // SAFETY: The callback table supplies a live `TestContext` pointer.
    unsafe { test_context(context) }
        .captured_completion_id
        .store(completion_id, Ordering::Relaxed);
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
        block_on(host.store_protected_secret(store_request())),
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
        block_on(host.store_protected_secret(store_request())),
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
        block_on(host.store_protected_secret(store_request())),
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
        block_on(host.store_protected_secret(store_request())),
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

    // SAFETY: Null is not dereferenced. The ID was never registered.
    let unknown =
        unsafe { wallet_engine_store_protected_secret_complete(u64::MAX, std::ptr::null()) };
    assert_eq!(unknown, WalletEngineAbiStatus::InvalidArgument);
}

#[test]
fn dropping_the_host_future_unregisters_its_completion() {
    let context = TestContext::default();
    let callbacks = callback_table(&context, Some(store_without_completion));
    // SAFETY: The callback table and context remain live through this test.
    let host = unsafe { adapter(&callbacks) };

    let result = host.store_protected_secret(store_request()).now_or_never();
    assert_eq!(result, None);
    let completion_id = context.captured_completion_id.load(Ordering::Relaxed);
    assert_ne!(completion_id, 0);
    // SAFETY: Null is not dereferenced. Dropping the future removed this ID.
    let status =
        unsafe { wallet_engine_store_protected_secret_complete(completion_id, std::ptr::null()) };
    assert_eq!(status, WalletEngineAbiStatus::InvalidArgument);
}
