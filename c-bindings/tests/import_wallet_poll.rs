#![allow(unsafe_code)]
#![allow(clippy::expect_used)]

use std::{
    ffi::c_void,
    sync::{
        Mutex, MutexGuard,
        atomic::{AtomicPtr, AtomicUsize, Ordering},
    },
    thread::ThreadId,
};

use wallet_engine_c::{
    WALLET_ENGINE_NETWORK_TESTNET, WALLET_ENGINE_PLATFORM_HOST_CALLBACKS_SIZE,
    WalletEngineAbiStatus, WalletEngineImportWalletOperation, WalletEngineImportWalletRequest,
    WalletEngineLifecycle, WalletEngineOperationPollState, WalletEnginePlatformHostCallbacks,
    WalletEngineProtectedSecretStoreCompletion, WalletEngineProtectedSecretStoreView,
    WalletEngineStringView, WalletEngineStringViewSlice, WalletEngineWalletDescriptorView,
    WalletEngineWalletLifecycleErrorCode, WalletEngineWalletLifecycleErrorView,
    wallet_engine_import_wallet_operation_free, wallet_engine_import_wallet_operation_poll,
    wallet_engine_lifecycle_free, wallet_engine_lifecycle_import_wallet_start,
    wallet_engine_lifecycle_new, wallet_engine_protected_secret_store_completion_complete,
    wallet_engine_protected_secret_store_completion_free,
};

const RECOVERY_PHRASE: &str = "section garden tomato dinner season dice renew length useful spin trade intact use universe what post spike keen mandate behind concert egg doll rug";
const TESTNET_ADDRESS: &str = "0QA_6fh0aRAkD7n1MNfAUx8TvyCUw2iTQfzVM-0isMze2anN";

#[derive(Debug, PartialEq, Eq)]
enum ImportResult {
    Descriptor {
        record_id: String,
        address: String,
        secret_ref: String,
    },
    Error(WalletEngineWalletLifecycleErrorCode),
}

#[derive(Default)]
struct Observation {
    host_threads: Vec<ThreadId>,
    result_threads: Vec<ThreadId>,
    result: Option<ImportResult>,
    stored_secret_ref: Option<String>,
}

struct TestContext {
    retains: AtomicUsize,
    releases: AtomicUsize,
    stores: AtomicUsize,
    pending_completion: AtomicPtr<WalletEngineProtectedSecretStoreCompletion>,
    observation: Mutex<Observation>,
}

impl TestContext {
    fn new() -> Self {
        Self {
            retains: AtomicUsize::new(0),
            releases: AtomicUsize::new(0),
            stores: AtomicUsize::new(0),
            pending_completion: AtomicPtr::new(std::ptr::null_mut()),
            observation: Mutex::new(Observation::default()),
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

unsafe fn test_context<'a>(context: *mut c_void) -> &'a TestContext {
    // SAFETY: Every callback in this test uses a live `TestContext` pointer.
    unsafe { &*context.cast::<TestContext>() }
}

unsafe extern "C" fn retain_context(context: *mut c_void) {
    // SAFETY: The callback table supplies a live `TestContext` pointer.
    unsafe { test_context(context) }
        .retains
        .fetch_add(1, Ordering::Relaxed);
}

unsafe extern "C" fn release_context(context: *mut c_void) {
    // SAFETY: The callback table supplies a live `TestContext` pointer.
    unsafe { test_context(context) }
        .releases
        .fetch_add(1, Ordering::Release);
}

unsafe fn record_store(
    context: *mut c_void,
    completion: *mut WalletEngineProtectedSecretStoreCompletion,
    request: *const WalletEngineProtectedSecretStoreView,
) {
    // SAFETY: The host callback receives a readable callback-scoped request.
    let request = unsafe { *request };
    // SAFETY: The nested reference remains live for this callback.
    let secret_ref = unsafe { request.secret_ref.value.try_to_string() }
        .expect("adapter supplied an invalid secret reference");
    assert!(!request.bytes.data.is_null());
    assert_ne!(request.bytes.len, 0);
    assert!(request.require_user_presence);

    // SAFETY: The callback table supplies a live `TestContext` pointer.
    let context = unsafe { test_context(context) };
    context.stores.fetch_add(1, Ordering::Relaxed);
    context
        .pending_completion
        .store(completion, Ordering::Release);
    let mut observation = lock(&context.observation);
    observation.host_threads.push(std::thread::current().id());
    observation.stored_secret_ref = Some(secret_ref);
}

unsafe extern "C" fn store_synchronously(
    context: *mut c_void,
    completion: *mut WalletEngineProtectedSecretStoreCompletion,
    request: *const WalletEngineProtectedSecretStoreView,
) {
    // SAFETY: The adapter supplies callback-scoped arguments.
    unsafe { record_store(context, completion, request) };
    // SAFETY: The callback owns the live completion handle.
    let status = unsafe {
        wallet_engine_protected_secret_store_completion_complete(completion, std::ptr::null())
    };
    assert_eq!(status, WalletEngineAbiStatus::Ok);
    // SAFETY: Completion has returned and no other thread uses this handle.
    unsafe { wallet_engine_protected_secret_store_completion_free(completion) };
}

unsafe extern "C" fn store_later(
    context: *mut c_void,
    completion: *mut WalletEngineProtectedSecretStoreCompletion,
    request: *const WalletEngineProtectedSecretStoreView,
) {
    // SAFETY: The adapter supplies callback-scoped arguments. The test takes
    // ownership of `completion` through the context.
    unsafe { record_store(context, completion, request) };
}

unsafe extern "C" fn import_result(
    context: *mut c_void,
    abi_status: WalletEngineAbiStatus,
    descriptor: *const WalletEngineWalletDescriptorView,
    error: *const WalletEngineWalletLifecycleErrorView,
) {
    assert_eq!(abi_status, WalletEngineAbiStatus::Ok);
    let result = if !descriptor.is_null() && error.is_null() {
        // SAFETY: The callback receives a readable callback-scoped descriptor.
        let descriptor = unsafe { *descriptor };
        // SAFETY: Nested descriptor views remain live for this callback.
        let record_id = unsafe { descriptor.record_id.try_to_string() }
            .expect("descriptor record ID is invalid");
        // SAFETY: Nested descriptor views remain live for this callback.
        let address =
            unsafe { descriptor.address.try_to_string() }.expect("descriptor address is invalid");
        // SAFETY: Nested descriptor views remain live for this callback.
        let secret_ref = unsafe { descriptor.secret_ref.value.try_to_string() }
            .expect("descriptor secret reference is invalid");
        ImportResult::Descriptor {
            record_id,
            address,
            secret_ref,
        }
    } else if descriptor.is_null() && !error.is_null() {
        // SAFETY: The callback receives a readable callback-scoped error.
        ImportResult::Error(unsafe { *error }.code)
    } else {
        panic!("invalid import result pointer shape");
    };

    // SAFETY: The result callback receives a live `TestContext` pointer.
    let context = unsafe { test_context(context) };
    let mut observation = lock(&context.observation);
    observation.result_threads.push(std::thread::current().id());
    observation.result = Some(result);
}

fn callbacks(
    context: &TestContext,
    store: unsafe extern "C" fn(
        *mut c_void,
        *mut WalletEngineProtectedSecretStoreCompletion,
        *const WalletEngineProtectedSecretStoreView,
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

unsafe fn lifecycle(callbacks: &WalletEnginePlatformHostCallbacks) -> *mut WalletEngineLifecycle {
    let mut lifecycle = std::ptr::null_mut();
    // SAFETY: The callback table and output pointer remain live for this call.
    let status = unsafe { wallet_engine_lifecycle_new(callbacks, &mut lifecycle) };
    assert_eq!(status, WalletEngineAbiStatus::Ok);
    assert!(!lifecycle.is_null());
    lifecycle
}

fn recovery_words() -> Vec<String> {
    RECOVERY_PHRASE
        .split_ascii_whitespace()
        .map(str::to_owned)
        .collect()
}

unsafe fn start_operation(
    lifecycle: *const WalletEngineLifecycle,
    record_id: &str,
    words: &[String],
) -> *mut WalletEngineImportWalletOperation {
    let word_views: Vec<_> = words
        .iter()
        .map(|word| WalletEngineStringView::from(word.as_str()))
        .collect();
    let request = WalletEngineImportWalletRequest {
        record_id: WalletEngineStringView::from(record_id),
        network: WALLET_ENGINE_NETWORK_TESTNET,
        recovery_words: WalletEngineStringViewSlice::from(word_views.as_slice()),
    };
    let mut operation = std::ptr::null_mut();
    // SAFETY: All request views and output storage remain live for this call.
    let status =
        unsafe { wallet_engine_lifecycle_import_wallet_start(lifecycle, &request, &mut operation) };
    assert_eq!(status, WalletEngineAbiStatus::Ok);
    assert!(!operation.is_null());
    operation
}

unsafe fn poll_operation(
    operation: *mut WalletEngineImportWalletOperation,
    context: &TestContext,
) -> (WalletEngineAbiStatus, WalletEngineOperationPollState) {
    let mut state = WalletEngineOperationPollState::Pending;
    // SAFETY: The operation and callback context remain live, and `state` is
    // writable for this synchronous poll.
    let status = unsafe {
        wallet_engine_import_wallet_operation_poll(
            operation,
            std::ptr::from_ref(context).cast_mut().cast(),
            Some(import_result),
            &mut state,
        )
    };
    (status, state)
}

#[test]
fn import_api_rejects_invalid_arguments_without_advancing() {
    let context = TestContext::new();
    let callbacks = callbacks(&context, store_synchronously);
    // SAFETY: The callback table remains live through operation destruction.
    let lifecycle = unsafe { lifecycle(&callbacks) };
    let words = recovery_words();
    let word_views: Vec<_> = words
        .iter()
        .map(|word| WalletEngineStringView::from(word.as_str()))
        .collect();
    let request = WalletEngineImportWalletRequest {
        record_id: WalletEngineStringView::from("validated-import"),
        network: WALLET_ENGINE_NETWORK_TESTNET,
        recovery_words: WalletEngineStringViewSlice::from(word_views.as_slice()),
    };

    // SAFETY: A null output pointer is rejected without being dereferenced.
    let status = unsafe {
        wallet_engine_lifecycle_import_wallet_start(lifecycle, &request, std::ptr::null_mut())
    };
    assert_eq!(status, WalletEngineAbiStatus::InvalidArgument);

    let mut operation = std::ptr::dangling_mut::<WalletEngineImportWalletOperation>();
    // SAFETY: A null request is rejected and the writable output is cleared.
    let status = unsafe {
        wallet_engine_lifecycle_import_wallet_start(lifecycle, std::ptr::null(), &mut operation)
    };
    assert_eq!(status, WalletEngineAbiStatus::InvalidArgument);
    assert!(operation.is_null());

    // SAFETY: All request views and output storage remain live for this call.
    let status =
        unsafe { wallet_engine_lifecycle_import_wallet_start(lifecycle, &request, &mut operation) };
    assert_eq!(status, WalletEngineAbiStatus::Ok);
    assert!(!operation.is_null());

    let mut state = WalletEngineOperationPollState::Ready;
    // SAFETY: A null callback is rejected before advancing the live operation.
    let status = unsafe {
        wallet_engine_import_wallet_operation_poll(
            operation,
            std::ptr::null_mut(),
            None,
            &mut state,
        )
    };
    assert_eq!(status, WalletEngineAbiStatus::InvalidArgument);
    assert_eq!(state, WalletEngineOperationPollState::Pending);
    assert_eq!(context.stores.load(Ordering::Relaxed), 0);

    // SAFETY: The rejected poll did not advance this live operation.
    let poll = unsafe { poll_operation(operation, &context) };
    assert_eq!(poll.0, WalletEngineAbiStatus::Ok);
    assert_eq!(poll.1, WalletEngineOperationPollState::Ready);
    // SAFETY: Polling is complete and this test uniquely owns both handles.
    unsafe {
        wallet_engine_import_wallet_operation_free(operation);
        wallet_engine_lifecycle_free(lifecycle);
    }
    assert_eq!(context.releases.load(Ordering::Acquire), 1);
}

#[test]
fn start_copies_recovery_words_without_polling() {
    let context = TestContext::new();
    let callbacks = callbacks(&context, store_synchronously);
    // SAFETY: The callback table remains live through operation destruction.
    let lifecycle = unsafe { lifecycle(&callbacks) };
    let mut words = vec!["not".to_owned(), "a".to_owned(), "mnemonic".to_owned()];
    // SAFETY: The lifecycle and word strings remain live for the start call.
    let operation = unsafe { start_operation(lifecycle, "copied-import", &words) };
    words.clear();

    assert_eq!(context.stores.load(Ordering::Relaxed), 0);
    assert!(lock(&context.observation).result.is_none());
    // SAFETY: The future retained the core lifecycle.
    unsafe { wallet_engine_lifecycle_free(lifecycle) };
    // SAFETY: The operation owns copied request values and remains live.
    let poll = unsafe { poll_operation(operation, &context) };
    assert_eq!(
        poll,
        (
            WalletEngineAbiStatus::Ok,
            WalletEngineOperationPollState::Ready,
        )
    );
    assert_eq!(
        lock(&context.observation).result.as_ref(),
        Some(&ImportResult::Error(
            WalletEngineWalletLifecycleErrorCode::InvalidRecoveryPhrase
        ))
    );
    // SAFETY: Polling is complete and this test uniquely owns the handle.
    unsafe { wallet_engine_import_wallet_operation_free(operation) };
    assert_eq!(context.releases.load(Ordering::Acquire), 1);
}

#[test]
fn synchronous_import_runs_host_and_result_callbacks_on_the_poll_thread() {
    let context = TestContext::new();
    let callbacks = callbacks(&context, store_synchronously);
    // SAFETY: The callback table remains live through operation completion.
    let lifecycle = unsafe { lifecycle(&callbacks) };
    let words = recovery_words();
    // SAFETY: The lifecycle and input views remain live for the start call.
    let operation = unsafe { start_operation(lifecycle, "imported-wallet", &words) };
    // SAFETY: The operation future retained the core lifecycle.
    unsafe { wallet_engine_lifecycle_free(lifecycle) };

    let operation_address = operation.addr();
    let context_address = std::ptr::from_ref(&context).addr();
    let poll_thread = std::thread::spawn(move || {
        let operation = operation_address as *mut WalletEngineImportWalletOperation;
        // SAFETY: The parent keeps the context live until this thread joins.
        let context = unsafe { &*(context_address as *const TestContext) };
        // SAFETY: This thread exclusively polls the live operation.
        let result = unsafe { poll_operation(operation, context) };
        (std::thread::current().id(), result)
    });
    let (thread_id, result) = poll_thread.join().expect("import poll thread panicked");
    assert_eq!(
        result,
        (
            WalletEngineAbiStatus::Ok,
            WalletEngineOperationPollState::Ready,
        )
    );

    let observation = lock(&context.observation);
    assert_eq!(observation.host_threads.as_slice(), [thread_id]);
    assert_eq!(observation.result_threads.as_slice(), [thread_id]);
    assert_eq!(
        observation.result.as_ref(),
        Some(&ImportResult::Descriptor {
            record_id: "imported-wallet".to_owned(),
            address: TESTNET_ADDRESS.to_owned(),
            secret_ref: "wallet:imported-wallet:mnemonic".to_owned(),
        })
    );
    assert_eq!(
        observation.stored_secret_ref.as_deref(),
        Some("wallet:imported-wallet:mnemonic")
    );
    drop(observation);

    // SAFETY: The poll thread joined and this test uniquely owns the handle.
    unsafe { wallet_engine_import_wallet_operation_free(operation) };
    assert_eq!(context.releases.load(Ordering::Acquire), 1);
}

#[test]
fn asynchronous_completion_requires_an_explicit_client_poll() {
    let context = TestContext::new();
    let callbacks = callbacks(&context, store_later);
    // SAFETY: The callback table remains live through operation completion.
    let lifecycle = unsafe { lifecycle(&callbacks) };
    let words = recovery_words();
    // SAFETY: The lifecycle and input views remain live for the start call.
    let operation = unsafe { start_operation(lifecycle, "async-import", &words) };
    // SAFETY: The operation future retained the core lifecycle.
    unsafe { wallet_engine_lifecycle_free(lifecycle) };

    // SAFETY: This thread exclusively polls the live operation.
    let poll = unsafe { poll_operation(operation, &context) };
    assert_eq!(poll.1, WalletEngineOperationPollState::Pending);
    assert!(lock(&context.observation).result.is_none());
    let completion = context.pending_completion.load(Ordering::Acquire);
    assert!(!completion.is_null());
    let completion_address = completion.addr();
    let completion_thread = std::thread::spawn(move || {
        let completion = completion_address as *mut WalletEngineProtectedSecretStoreCompletion;
        // SAFETY: This client-owned thread owns the live completion handle.
        let status = unsafe {
            wallet_engine_protected_secret_store_completion_complete(completion, std::ptr::null())
        };
        // SAFETY: Completion returned and no other thread uses the handle.
        unsafe { wallet_engine_protected_secret_store_completion_free(completion) };
        status
    });
    assert_eq!(
        completion_thread
            .join()
            .expect("completion thread panicked"),
        WalletEngineAbiStatus::Ok
    );
    assert!(lock(&context.observation).result.is_none());

    // SAFETY: The client explicitly resumes the live operation.
    let poll = unsafe { poll_operation(operation, &context) };
    assert_eq!(poll.1, WalletEngineOperationPollState::Ready);
    assert!(matches!(
        lock(&context.observation).result.as_ref(),
        Some(&ImportResult::Descriptor { .. })
    ));
    // SAFETY: Polling is complete and this test uniquely owns the handle.
    unsafe { wallet_engine_import_wallet_operation_free(operation) };
    assert_eq!(context.releases.load(Ordering::Acquire), 1);
}

#[test]
fn freeing_pending_import_cancels_it_without_resuming() {
    let context = TestContext::new();
    let callbacks = callbacks(&context, store_later);
    // SAFETY: The callback table remains live through operation destruction.
    let lifecycle = unsafe { lifecycle(&callbacks) };
    let words = recovery_words();
    // SAFETY: The lifecycle and input views remain live for the start call.
    let operation = unsafe { start_operation(lifecycle, "cancel-import", &words) };
    // SAFETY: The operation future retained the core lifecycle.
    unsafe { wallet_engine_lifecycle_free(lifecycle) };
    // SAFETY: This thread exclusively polls the live operation.
    let poll = unsafe { poll_operation(operation, &context) };
    assert_eq!(poll.1, WalletEngineOperationPollState::Pending);
    let completion = context.pending_completion.load(Ordering::Acquire);
    assert!(!completion.is_null());

    // SAFETY: No poll is active and this test uniquely owns the operation.
    unsafe { wallet_engine_import_wallet_operation_free(operation) };
    // SAFETY: The host still owns this completion after operation destruction.
    let status = unsafe {
        wallet_engine_protected_secret_store_completion_complete(completion, std::ptr::null())
    };
    assert_eq!(status, WalletEngineAbiStatus::InvalidArgument);
    // SAFETY: Completion has returned and no other thread uses the handle.
    unsafe { wallet_engine_protected_secret_store_completion_free(completion) };
    assert!(lock(&context.observation).result.is_none());
    assert_eq!(context.releases.load(Ordering::Acquire), 1);
}
