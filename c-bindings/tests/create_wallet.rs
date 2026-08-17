#![allow(unsafe_code)]
#![allow(clippy::expect_used)]

use std::{
    ffi::c_void,
    sync::{Mutex, MutexGuard},
    thread::ThreadId,
};

use wallet_engine_c::{
    WALLET_ENGINE_NETWORK_TESTNET, WALLET_ENGINE_PLATFORM_HOST_CALLBACKS_SIZE,
    WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_UNAVAILABLE, WalletEngineAbiStatus,
    WalletEngineCreateWalletRequest, WalletEngineCreatedWalletView,
    WalletEngineImportWalletRequest, WalletEngineLifecycle, WalletEnginePlatformHostCallbacks,
    WalletEngineProtectedSecretHostErrorView, WalletEngineProtectedSecretStoreResultFn,
    WalletEngineProtectedSecretStoreView, WalletEngineStringView, WalletEngineStringViewSlice,
    WalletEngineWalletDescriptorView, WalletEngineWalletLifecycleErrorCode,
    WalletEngineWalletLifecycleErrorView, wallet_engine_lifecycle_create_wallet,
    wallet_engine_lifecycle_free, wallet_engine_lifecycle_import_wallet,
    wallet_engine_lifecycle_new,
};

const RECOVERY_PHRASE: &str = "section garden tomato dinner season dice renew length useful spin trade intact use universe what post spike keen mandate behind concert egg doll rug";
const TESTNET_ADDRESS: &str = "0QA_6fh0aRAkD7n1MNfAUx8TvyCUw2iTQfzVM-0isMze2anN";

#[derive(Debug, PartialEq, Eq)]
enum LifecycleResult {
    Created {
        record_id: String,
        secret_ref: String,
        phrase: String,
    },
    Imported {
        record_id: String,
        address: String,
        secret_ref: String,
    },
    Error {
        code: WalletEngineWalletLifecycleErrorCode,
        diagnostic: String,
    },
}

#[derive(Default)]
struct Observation {
    store_threads: Vec<ThreadId>,
    result_threads: Vec<ThreadId>,
    stored_secret_refs: Vec<String>,
    stored_secrets: Vec<String>,
    results: Vec<LifecycleResult>,
}

#[derive(Default)]
struct TestContext {
    observation: Mutex<Observation>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

unsafe fn test_context<'a>(context: *mut c_void) -> &'a TestContext {
    // SAFETY: Every callback table in this test uses a live `TestContext`.
    unsafe { &*context.cast::<TestContext>() }
}

const unsafe extern "C" fn retain_context(_context: *mut c_void) {}

const unsafe extern "C" fn release_context(_context: *mut c_void) {}

unsafe fn record_store(context: *mut c_void, request: *const WalletEngineProtectedSecretStoreView) {
    assert!(!request.is_null());
    // SAFETY: The library supplies a callback-scoped readable request.
    let request = unsafe { request.read() };
    // SAFETY: Nested request views remain live during this callback.
    let secret_ref = unsafe { request.secret_ref.value.try_to_string() }
        .expect("secret reference should be valid");
    // SAFETY: Nested request views remain live during this callback.
    let secret = unsafe { request.bytes.try_to_vec() }.expect("secret bytes should be valid");
    let secret = String::from_utf8(secret).expect("mnemonic should be UTF-8");
    assert!(request.require_user_presence);

    // SAFETY: The callback table supplies a live test context.
    let context = unsafe { test_context(context) };
    let mut observation = lock(&context.observation);
    observation.store_threads.push(std::thread::current().id());
    observation.stored_secret_refs.push(secret_ref);
    observation.stored_secrets.push(secret);
}

unsafe extern "C" fn store_success(
    context: *mut c_void,
    request: *const WalletEngineProtectedSecretStoreView,
    result_context: *mut c_void,
    result: WalletEngineProtectedSecretStoreResultFn,
) {
    // SAFETY: The library supplies callback-scoped arguments.
    unsafe { record_store(context, request) };
    let result = result.expect("result callback should be present");
    // SAFETY: The result context and callback are valid for this host call.
    let status = unsafe { result(result_context, std::ptr::null()) };
    assert_eq!(status, WalletEngineAbiStatus::Ok);
}

unsafe extern "C" fn store_error(
    context: *mut c_void,
    request: *const WalletEngineProtectedSecretStoreView,
    result_context: *mut c_void,
    result: WalletEngineProtectedSecretStoreResultFn,
) {
    // SAFETY: The library supplies callback-scoped arguments.
    unsafe { record_store(context, request) };
    let error = WalletEngineProtectedSecretHostErrorView {
        kind: WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_UNAVAILABLE,
        diagnostic: WalletEngineStringView::from("keychain unavailable"),
    };
    let result = result.expect("result callback should be present");
    // SAFETY: The result context and error view remain live for this call.
    let status = unsafe { result(result_context, &error) };
    assert_eq!(status, WalletEngineAbiStatus::Ok);
}

const unsafe extern "C" fn store_without_result(
    _context: *mut c_void,
    _request: *const WalletEngineProtectedSecretStoreView,
    _result_context: *mut c_void,
    _result: WalletEngineProtectedSecretStoreResultFn,
) {
}

unsafe extern "C" fn store_duplicate_result(
    _context: *mut c_void,
    _request: *const WalletEngineProtectedSecretStoreView,
    result_context: *mut c_void,
    result: WalletEngineProtectedSecretStoreResultFn,
) {
    let result = result.expect("result callback should be present");
    // SAFETY: The result callback is live for this host call.
    let first = unsafe { result(result_context, std::ptr::null()) };
    assert_eq!(first, WalletEngineAbiStatus::Ok);
    // SAFETY: This deliberate duplicate verifies rejection.
    let duplicate = unsafe { result(result_context, std::ptr::null()) };
    assert_eq!(duplicate, WalletEngineAbiStatus::InvalidArgument);
}

unsafe extern "C" fn store_invalid_error(
    _context: *mut c_void,
    _request: *const WalletEngineProtectedSecretStoreView,
    result_context: *mut c_void,
    result: WalletEngineProtectedSecretStoreResultFn,
) {
    let invalid_utf8 = [0xff];
    let error = WalletEngineProtectedSecretHostErrorView {
        kind: WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_UNAVAILABLE,
        diagnostic: WalletEngineStringView {
            data: invalid_utf8.as_ptr().cast(),
            len: invalid_utf8.len(),
        },
    };
    let result = result.expect("result callback should be present");
    // SAFETY: The invalid bytes are readable and are rejected as UTF-8.
    let status = unsafe { result(result_context, &error) };
    assert_eq!(status, WalletEngineAbiStatus::InvalidUtf8);
}

fn host_callbacks(
    context: &TestContext,
    store: unsafe extern "C" fn(
        *mut c_void,
        *const WalletEngineProtectedSecretStoreView,
        *mut c_void,
        WalletEngineProtectedSecretStoreResultFn,
    ),
) -> WalletEnginePlatformHostCallbacks {
    WalletEnginePlatformHostCallbacks {
        struct_size: WALLET_ENGINE_PLATFORM_HOST_CALLBACKS_SIZE,
        context: std::ptr::from_ref(context).cast_mut().cast(),
        retain: Some(retain_context),
        release: Some(release_context),
        store_protected_secret: Some(store),
    }
}

unsafe fn new_lifecycle(
    callbacks: &WalletEnginePlatformHostCallbacks,
) -> *mut WalletEngineLifecycle {
    let mut lifecycle = std::ptr::null_mut();
    // SAFETY: The callback table and output pointer remain live for this call.
    let status = unsafe { wallet_engine_lifecycle_new(callbacks, &mut lifecycle) };
    assert_eq!(status, WalletEngineAbiStatus::Ok);
    assert!(!lifecycle.is_null());
    lifecycle
}

unsafe extern "C" fn create_result(
    context: *mut c_void,
    abi_status: WalletEngineAbiStatus,
    wallet: *const WalletEngineCreatedWalletView,
    error: *const WalletEngineWalletLifecycleErrorView,
) {
    assert_eq!(abi_status, WalletEngineAbiStatus::Ok);
    // SAFETY: The result callback receives a live test context.
    let context = unsafe { test_context(context) };
    let result = if !wallet.is_null() && error.is_null() {
        // SAFETY: The wallet view and nested values live for this callback.
        let wallet = unsafe { wallet.read() };
        LifecycleResult::Created {
            // SAFETY: Nested views live for this callback.
            record_id: unsafe { wallet.descriptor.record_id.try_to_string() }
                .expect("record ID should be valid"),
            // SAFETY: Nested views live for this callback.
            secret_ref: unsafe { wallet.descriptor.secret_ref.value.try_to_string() }
                .expect("secret reference should be valid"),
            // SAFETY: Nested views live for this callback.
            phrase: unsafe { wallet.recovery_phrase.phrase.try_to_string() }
                .expect("phrase should be valid"),
        }
    } else {
        assert!(wallet.is_null());
        assert!(!error.is_null());
        // SAFETY: The error view and diagnostic live for this callback.
        let error = unsafe { error.read() };
        LifecycleResult::Error {
            code: error.code,
            // SAFETY: The diagnostic view lives for this callback.
            diagnostic: unsafe { error.diagnostic.try_to_string() }
                .expect("diagnostic should be valid"),
        }
    };

    let mut observation = lock(&context.observation);
    observation.result_threads.push(std::thread::current().id());
    observation.results.push(result);
}

unsafe extern "C" fn import_result(
    context: *mut c_void,
    abi_status: WalletEngineAbiStatus,
    descriptor: *const WalletEngineWalletDescriptorView,
    error: *const WalletEngineWalletLifecycleErrorView,
) {
    assert_eq!(abi_status, WalletEngineAbiStatus::Ok);
    // SAFETY: The result callback receives a live test context.
    let context = unsafe { test_context(context) };
    let result = if !descriptor.is_null() && error.is_null() {
        // SAFETY: The descriptor and nested views live for this callback.
        let descriptor = unsafe { descriptor.read() };
        LifecycleResult::Imported {
            // SAFETY: Nested views live for this callback.
            record_id: unsafe { descriptor.record_id.try_to_string() }
                .expect("record ID should be valid"),
            // SAFETY: Nested views live for this callback.
            address: unsafe { descriptor.address.try_to_string() }
                .expect("address should be valid"),
            // SAFETY: Nested views live for this callback.
            secret_ref: unsafe { descriptor.secret_ref.value.try_to_string() }
                .expect("secret reference should be valid"),
        }
    } else {
        assert!(descriptor.is_null());
        assert!(!error.is_null());
        // SAFETY: The error view and diagnostic live for this callback.
        let error = unsafe { error.read() };
        LifecycleResult::Error {
            code: error.code,
            // SAFETY: The diagnostic view lives for this callback.
            diagnostic: unsafe { error.diagnostic.try_to_string() }
                .expect("diagnostic should be valid"),
        }
    };

    let mut observation = lock(&context.observation);
    observation.result_threads.push(std::thread::current().id());
    observation.results.push(result);
}

unsafe fn create_wallet(
    lifecycle: *const WalletEngineLifecycle,
    context: &TestContext,
    record_id: &str,
) -> WalletEngineAbiStatus {
    let request = WalletEngineCreateWalletRequest {
        record_id: WalletEngineStringView::from(record_id),
        network: WALLET_ENGINE_NETWORK_TESTNET,
    };
    // SAFETY: The handle, request views, and callback context are live.
    unsafe {
        wallet_engine_lifecycle_create_wallet(
            lifecycle,
            &request,
            std::ptr::from_ref(context).cast_mut().cast(),
            Some(create_result),
        )
    }
}

#[test]
fn create_wallet_is_fully_synchronous_on_the_calling_thread() {
    let context = TestContext::default();
    let callbacks = host_callbacks(&context, store_success);
    // SAFETY: The callback context outlives the lifecycle.
    let lifecycle = unsafe { new_lifecycle(&callbacks) };
    let caller = std::thread::current().id();

    // SAFETY: The lifecycle and callback context are live for this call.
    let status = unsafe { create_wallet(lifecycle, &context, "wallet-1") };
    assert_eq!(status, WalletEngineAbiStatus::Ok);

    let observation = lock(&context.observation);
    assert_eq!(observation.store_threads, [caller]);
    assert_eq!(observation.result_threads, [caller]);
    assert_eq!(observation.stored_secret_refs, ["wallet:wallet-1:mnemonic"]);
    assert_eq!(observation.stored_secrets.len(), 1);
    assert_eq!(
        observation.stored_secrets[0]
            .split_ascii_whitespace()
            .count(),
        24
    );
    assert!(matches!(
        observation.results.as_slice(),
        [LifecycleResult::Created {
            record_id,
            secret_ref,
            phrase,
        }] if record_id == "wallet-1"
            && secret_ref == "wallet:wallet-1:mnemonic"
            && phrase.split_ascii_whitespace().count() == 24
    ));
    drop(observation);

    // SAFETY: No calls use the lifecycle concurrently with this free.
    unsafe { wallet_engine_lifecycle_free(lifecycle) };
}

#[test]
fn import_wallet_is_fully_synchronous() {
    let context = TestContext::default();
    let callbacks = host_callbacks(&context, store_success);
    // SAFETY: The callback context outlives the lifecycle.
    let lifecycle = unsafe { new_lifecycle(&callbacks) };
    let words = RECOVERY_PHRASE.split_ascii_whitespace().collect::<Vec<_>>();
    let word_views = words
        .iter()
        .copied()
        .map(WalletEngineStringView::from)
        .collect::<Vec<_>>();
    let request = WalletEngineImportWalletRequest {
        record_id: WalletEngineStringView::from("imported-wallet"),
        network: WALLET_ENGINE_NETWORK_TESTNET,
        recovery_words: WalletEngineStringViewSlice::from(word_views.as_slice()),
    };

    // SAFETY: The handle, request views, and callback context are live.
    let status = unsafe {
        wallet_engine_lifecycle_import_wallet(
            lifecycle,
            &request,
            std::ptr::from_ref(&context).cast_mut().cast(),
            Some(import_result),
        )
    };
    assert_eq!(status, WalletEngineAbiStatus::Ok);

    let observation = lock(&context.observation);
    assert_eq!(observation.stored_secrets, [RECOVERY_PHRASE]);
    assert_eq!(
        observation.results,
        [LifecycleResult::Imported {
            record_id: "imported-wallet".to_owned(),
            address: TESTNET_ADDRESS.to_owned(),
            secret_ref: "wallet:imported-wallet:mnemonic".to_owned(),
        }]
    );
    drop(observation);

    // SAFETY: No calls use the lifecycle concurrently with this free.
    unsafe { wallet_engine_lifecycle_free(lifecycle) };
}

#[test]
fn domain_and_host_errors_are_delivered_synchronously() {
    let invalid_context = TestContext::default();
    let callbacks = host_callbacks(&invalid_context, store_success);
    // SAFETY: The callback context outlives the lifecycle.
    let lifecycle = unsafe { new_lifecycle(&callbacks) };
    // SAFETY: The lifecycle and callback context are live.
    let status = unsafe { create_wallet(lifecycle, &invalid_context, "") };
    assert_eq!(status, WalletEngineAbiStatus::Ok);
    let observation = lock(&invalid_context.observation);
    assert!(observation.store_threads.is_empty());
    assert_eq!(
        observation.results,
        [LifecycleResult::Error {
            code: WalletEngineWalletLifecycleErrorCode::InvalidRecordId,
            diagnostic: String::new(),
        }]
    );
    drop(observation);
    // SAFETY: No calls use the lifecycle concurrently with this free.
    unsafe { wallet_engine_lifecycle_free(lifecycle) };

    let host_context = TestContext::default();
    let callbacks = host_callbacks(&host_context, store_error);
    // SAFETY: The callback context outlives the lifecycle.
    let lifecycle = unsafe { new_lifecycle(&callbacks) };
    // SAFETY: The lifecycle and callback context are live.
    let status = unsafe { create_wallet(lifecycle, &host_context, "host-error") };
    assert_eq!(status, WalletEngineAbiStatus::Ok);
    assert_eq!(
        lock(&host_context.observation).results,
        [LifecycleResult::Error {
            code: WalletEngineWalletLifecycleErrorCode::ProtectedSecretHost,
            diagnostic: "keychain unavailable".to_owned(),
        }]
    );
    // SAFETY: No calls use the lifecycle concurrently with this free.
    unsafe { wallet_engine_lifecycle_free(lifecycle) };
}

#[test]
fn synchronous_host_contract_violations_are_boundary_errors() {
    for (store, expected) in [
        (
            store_without_result as unsafe extern "C" fn(_, _, _, _),
            WalletEngineAbiStatus::InvalidArgument,
        ),
        (
            store_duplicate_result as unsafe extern "C" fn(_, _, _, _),
            WalletEngineAbiStatus::InvalidArgument,
        ),
        (
            store_invalid_error as unsafe extern "C" fn(_, _, _, _),
            WalletEngineAbiStatus::InvalidUtf8,
        ),
    ] {
        let context = TestContext::default();
        let callbacks = host_callbacks(&context, store);
        // SAFETY: The callback context outlives the lifecycle.
        let lifecycle = unsafe { new_lifecycle(&callbacks) };
        // SAFETY: The lifecycle and callback context are live.
        let status = unsafe { create_wallet(lifecycle, &context, "boundary-error") };
        assert_eq!(status, expected);
        assert!(lock(&context.observation).results.is_empty());
        // SAFETY: No calls use the lifecycle concurrently with this free.
        unsafe { wallet_engine_lifecycle_free(lifecycle) };
    }
}

#[test]
fn separate_calls_run_concurrently_only_on_client_threads() {
    let context = Box::new(TestContext::default());
    let callbacks = host_callbacks(&context, store_success);
    // SAFETY: The boxed callback context outlives the lifecycle and threads.
    let lifecycle = unsafe { new_lifecycle(&callbacks) };
    let lifecycle_address = lifecycle as usize;
    let context_address = std::ptr::from_ref(context.as_ref()) as usize;

    let threads = ["thread-wallet-1", "thread-wallet-2"].map(|record_id| {
        std::thread::spawn(move || {
            let lifecycle = lifecycle_address as *const WalletEngineLifecycle;
            let context = context_address as *const TestContext;
            // SAFETY: Main keeps both allocations live until every thread joins.
            let status = unsafe { create_wallet(lifecycle, &*context, record_id) };
            assert_eq!(status, WalletEngineAbiStatus::Ok);
        })
    });
    for thread in threads {
        thread.join().expect("client worker should finish");
    }

    let observation = lock(&context.observation);
    assert_eq!(observation.store_threads.len(), 2);
    assert_eq!(observation.result_threads.len(), 2);
    assert_ne!(observation.store_threads[0], observation.store_threads[1]);
    for thread in &observation.store_threads {
        assert!(observation.result_threads.contains(thread));
    }
    drop(observation);

    // SAFETY: Both client workers finished before lifecycle destruction.
    unsafe { wallet_engine_lifecycle_free(lifecycle) };
}

#[test]
fn lifecycle_calls_validate_boundary_arguments() {
    let context = TestContext::default();
    let callbacks = host_callbacks(&context, store_success);
    // SAFETY: The callback context outlives the lifecycle.
    let lifecycle = unsafe { new_lifecycle(&callbacks) };
    let request = WalletEngineCreateWalletRequest {
        record_id: WalletEngineStringView::from("wallet-1"),
        network: WALLET_ENGINE_NETWORK_TESTNET,
    };

    // SAFETY: Each invalid pointer is rejected before dereference; other values
    // remain valid for the call.
    let status = unsafe {
        wallet_engine_lifecycle_create_wallet(
            std::ptr::null(),
            &request,
            std::ptr::null_mut(),
            Some(create_result),
        )
    };
    assert_eq!(status, WalletEngineAbiStatus::InvalidArgument);
    // SAFETY: Null request is rejected before dereference.
    let status = unsafe {
        wallet_engine_lifecycle_create_wallet(
            lifecycle,
            std::ptr::null(),
            std::ptr::null_mut(),
            Some(create_result),
        )
    };
    assert_eq!(status, WalletEngineAbiStatus::InvalidArgument);
    // SAFETY: Missing callback is rejected before the null context is used.
    let status = unsafe {
        wallet_engine_lifecycle_create_wallet(lifecycle, &request, std::ptr::null_mut(), None)
    };
    assert_eq!(status, WalletEngineAbiStatus::InvalidArgument);

    // SAFETY: No calls use the lifecycle concurrently with this free.
    unsafe { wallet_engine_lifecycle_free(lifecycle) };
}
