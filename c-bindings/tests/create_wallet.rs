#![allow(unsafe_code)]

use std::{
    ffi::c_void,
    sync::{
        Mutex, MutexGuard,
        atomic::{AtomicUsize, Ordering},
        mpsc::{Receiver, Sender, channel},
    },
    time::{Duration, Instant},
};

use wallet_engine_c::{
    WALLET_ENGINE_NETWORK_MAINNET, WALLET_ENGINE_NETWORK_TESTNET,
    WALLET_ENGINE_PLATFORM_HOST_CALLBACKS_SIZE,
    WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_AUTHENTICATION_FAILED,
    WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_CANCELLED,
    WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_NOT_FOUND,
    WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_OTHER,
    WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_POLICY_VIOLATION,
    WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_UNAVAILABLE, WalletEngineAbiStatus,
    WalletEngineCompletionId, WalletEngineCreateWalletRequest, WalletEngineCreatedWalletView,
    WalletEngineLifecycle, WalletEngineNetwork, WalletEnginePlatformHostCallbacks,
    WalletEngineProtectedSecretHostErrorView, WalletEngineProtectedSecretStoreView,
    WalletEngineStoreProtectedSecretFn, WalletEngineStringView,
    WalletEngineWalletLifecycleErrorCode, WalletEngineWalletLifecycleErrorView,
    wallet_engine_lifecycle_create_wallet, wallet_engine_lifecycle_free,
    wallet_engine_lifecycle_new, wallet_engine_store_protected_secret_complete,
};

#[derive(Debug, PartialEq, Eq)]
struct StoredSecret {
    secret_ref: String,
    bytes_len: usize,
    require_user_presence: bool,
}

#[derive(Debug, PartialEq, Eq)]
struct CreatedWallet {
    record_id: String,
    address: String,
    network: WalletEngineNetwork,
    secret_ref: String,
    phrase: String,
}

#[derive(Debug, PartialEq, Eq)]
struct LifecycleError {
    code: WalletEngineWalletLifecycleErrorCode,
    protected_secret_host_error_kind: Option<u32>,
    diagnostic: String,
}

#[derive(Debug, PartialEq, Eq)]
struct CompletionResult {
    abi_status: WalletEngineAbiStatus,
    wallet: Option<CreatedWallet>,
    error: Option<LifecycleError>,
    valid_pointer_shape: bool,
}

struct TestContext {
    retains: AtomicUsize,
    releases: AtomicUsize,
    stores: AtomicUsize,
    completion_calls: AtomicUsize,
    host_error_kind: u32,
    stored_secret: Mutex<Option<StoredSecret>>,
    completion_sender: Mutex<Option<Sender<CompletionResult>>>,
}

impl TestContext {
    const fn new(completion_sender: Sender<CompletionResult>) -> Self {
        Self::with_host_error_kind(
            completion_sender,
            WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_UNAVAILABLE,
        )
    }

    const fn with_host_error_kind(
        completion_sender: Sender<CompletionResult>,
        host_error_kind: u32,
    ) -> Self {
        Self {
            retains: AtomicUsize::new(0),
            releases: AtomicUsize::new(0),
            stores: AtomicUsize::new(0),
            completion_calls: AtomicUsize::new(0),
            host_error_kind,
            stored_secret: Mutex::new(None),
            completion_sender: Mutex::new(Some(completion_sender)),
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
    // SAFETY: Test callback tables and completions use a live `TestContext`.
    unsafe { &*context.cast::<TestContext>() }
}

unsafe extern "C" fn retain_context(context: *mut c_void) {
    // SAFETY: The callback table supplies a live `TestContext` pointer.
    unsafe { test_context(context) }
        .retains
        .fetch_add(1, Ordering::Release);
}

unsafe extern "C" fn release_context(context: *mut c_void) {
    // SAFETY: The callback table supplies a live `TestContext` pointer.
    unsafe { test_context(context) }
        .releases
        .fetch_add(1, Ordering::Release);
}

unsafe fn record_store_request(
    context: *mut c_void,
    request: *const WalletEngineProtectedSecretStoreView,
) {
    // SAFETY: The host adapter supplies a callback-scoped request.
    let request = unsafe { *request };
    // SAFETY: Nested views remain live for the callback.
    let secret_ref = unsafe { request.secret_ref.value.try_to_string() };
    let stored = secret_ref.ok().map(|secret_ref| StoredSecret {
        secret_ref,
        bytes_len: request.bytes.len,
        require_user_presence: request.require_user_presence,
    });
    // SAFETY: The callback table supplies a live `TestContext` pointer.
    let context = unsafe { test_context(context) };
    context.stores.fetch_add(1, Ordering::Relaxed);
    *lock(&context.stored_secret) = stored;
}

unsafe extern "C" fn store_success(
    context: *mut c_void,
    completion_id: WalletEngineCompletionId,
    request: *const WalletEngineProtectedSecretStoreView,
) {
    // SAFETY: The adapter supplies a callback-scoped request.
    unsafe { record_store_request(context, request) };
    // SAFETY: Null denotes successful completion and is not dereferenced.
    let _ =
        unsafe { wallet_engine_store_protected_secret_complete(completion_id, std::ptr::null()) };
}

unsafe extern "C" fn store_async_success(
    context: *mut c_void,
    completion_id: WalletEngineCompletionId,
    request: *const WalletEngineProtectedSecretStoreView,
) {
    // SAFETY: The adapter supplies a callback-scoped request.
    unsafe { record_store_request(context, request) };
    drop(std::thread::spawn(move || {
        // SAFETY: Null denotes successful completion and is not dereferenced.
        let _ = unsafe {
            wallet_engine_store_protected_secret_complete(completion_id, std::ptr::null())
        };
    }));
}

unsafe extern "C" fn store_error(
    context: *mut c_void,
    completion_id: WalletEngineCompletionId,
    request: *const WalletEngineProtectedSecretStoreView,
) {
    // SAFETY: The adapter supplies a callback-scoped request.
    unsafe { record_store_request(context, request) };
    // SAFETY: The callback table supplies a live `TestContext` pointer.
    let kind = unsafe { test_context(context) }.host_error_kind;
    let error = WalletEngineProtectedSecretHostErrorView {
        kind,
        diagnostic: WalletEngineStringView::from("protected storage failure"),
    };
    // SAFETY: `error` and its diagnostic remain live for this call.
    let _ = unsafe { wallet_engine_store_protected_secret_complete(completion_id, &error) };
}

unsafe fn copy_wallet(wallet: *const WalletEngineCreatedWalletView) -> Option<CreatedWallet> {
    // SAFETY: The completion callback receives a readable wallet view.
    let wallet = unsafe { *wallet };
    // SAFETY: All nested strings remain live for the callback.
    let record_id = unsafe { wallet.descriptor.record_id.try_to_string().ok()? };
    // SAFETY: All nested strings remain live for the callback.
    let address = unsafe { wallet.descriptor.address.try_to_string().ok()? };
    // SAFETY: All nested strings remain live for the callback.
    let secret_ref = unsafe { wallet.descriptor.secret_ref.value.try_to_string().ok()? };
    // SAFETY: The callback contract keeps the phrase view live.
    let phrase = unsafe { wallet.recovery_phrase.phrase.try_to_string().ok()? };

    Some(CreatedWallet {
        record_id,
        address,
        network: wallet.descriptor.network,
        secret_ref,
        phrase,
    })
}

unsafe fn copy_error(error: *const WalletEngineWalletLifecycleErrorView) -> Option<LifecycleError> {
    // SAFETY: The completion callback receives a readable error view.
    let error = unsafe { *error };
    // SAFETY: The diagnostic remains live for the callback.
    let diagnostic = unsafe { error.diagnostic.try_to_string().ok()? };
    Some(LifecycleError {
        code: error.code,
        protected_secret_host_error_kind: error
            .has_protected_secret_host_error_kind
            .then_some(error.protected_secret_host_error_kind),
        diagnostic,
    })
}

unsafe extern "C" fn create_wallet_complete(
    context: *mut c_void,
    abi_status: WalletEngineAbiStatus,
    wallet: *const WalletEngineCreatedWalletView,
    error: *const WalletEngineWalletLifecycleErrorView,
) {
    let (wallet_copy, error_copy, valid_pointer_shape) =
        if abi_status == WalletEngineAbiStatus::Ok && !wallet.is_null() && error.is_null() {
            // SAFETY: The non-null wallet view remains live for this callback.
            let wallet = unsafe { copy_wallet(wallet) };
            let valid = wallet.is_some();
            (wallet, None, valid)
        } else if abi_status == WalletEngineAbiStatus::Ok && wallet.is_null() && !error.is_null() {
            // SAFETY: The non-null error view remains live for this callback.
            let error = unsafe { copy_error(error) };
            let valid = error.is_some();
            (None, error, valid)
        } else {
            (
                None,
                None,
                abi_status == WalletEngineAbiStatus::Panic && wallet.is_null() && error.is_null(),
            )
        };
    let result = CompletionResult {
        abi_status,
        wallet: wallet_copy,
        error: error_copy,
        valid_pointer_shape,
    };

    // SAFETY: The completion context is a live `TestContext`.
    let context = unsafe { test_context(context) };
    context.completion_calls.fetch_add(1, Ordering::Relaxed);
    let sender = lock(&context.completion_sender).take();
    if let Some(sender) = sender {
        let _ = sender.send(result);
    }
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

unsafe fn lifecycle(callbacks: &WalletEnginePlatformHostCallbacks) -> *mut WalletEngineLifecycle {
    let mut lifecycle = std::ptr::null_mut();
    // SAFETY: The callback table and output pointer remain live for this call.
    let status = unsafe { wallet_engine_lifecycle_new(callbacks, &mut lifecycle) };
    assert_eq!(status, WalletEngineAbiStatus::Ok);
    assert!(!lifecycle.is_null());
    lifecycle
}

fn wait_for_release(context: &TestContext) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while context.releases.load(Ordering::Acquire) != 1 {
        assert!(Instant::now() < deadline, "host context was not released");
        std::thread::yield_now();
    }
}

fn receive(receiver: &Receiver<CompletionResult>) -> CompletionResult {
    match receiver.recv_timeout(Duration::from_secs(5)) {
        Ok(result) => result,
        Err(error) => panic!("wallet completion was not received: {error}"),
    }
}

#[test]
fn create_wallet_rejects_invalid_boundary_arguments_before_starting() {
    let (sender, receiver) = channel();
    let context = TestContext::new(sender);
    let callbacks = callback_table(&context, Some(store_success));
    // SAFETY: The callback table remains live until the handle is freed.
    let lifecycle = unsafe { lifecycle(&callbacks) };
    let valid_request = WalletEngineCreateWalletRequest {
        record_id: WalletEngineStringView::from("wallet-1"),
        network: WALLET_ENGINE_NETWORK_TESTNET,
    };

    // SAFETY: A null handle is accepted as an invalid argument and is not
    // dereferenced.
    let status = unsafe {
        wallet_engine_lifecycle_create_wallet(
            std::ptr::null(),
            &valid_request,
            std::ptr::from_ref(&context).cast_mut().cast(),
            Some(create_wallet_complete),
        )
    };
    assert_eq!(status, WalletEngineAbiStatus::InvalidArgument);
    // SAFETY: A null request is accepted as an invalid argument and is not
    // dereferenced.
    let status = unsafe {
        wallet_engine_lifecycle_create_wallet(
            lifecycle,
            std::ptr::null(),
            std::ptr::from_ref(&context).cast_mut().cast(),
            Some(create_wallet_complete),
        )
    };
    assert_eq!(status, WalletEngineAbiStatus::InvalidArgument);
    // SAFETY: The handle and request are valid; the missing callback is
    // rejected before starting work.
    let status = unsafe {
        wallet_engine_lifecycle_create_wallet(
            lifecycle,
            &valid_request,
            std::ptr::from_ref(&context).cast_mut().cast(),
            None,
        )
    };
    assert_eq!(status, WalletEngineAbiStatus::InvalidArgument);

    let invalid_utf8 = [0xff];
    let invalid_utf8_request = WalletEngineCreateWalletRequest {
        record_id: WalletEngineStringView {
            data: invalid_utf8.as_ptr().cast(),
            len: invalid_utf8.len(),
        },
        network: WALLET_ENGINE_NETWORK_TESTNET,
    };
    // SAFETY: The invalid UTF-8 byte remains readable for this call.
    let status = unsafe {
        wallet_engine_lifecycle_create_wallet(
            lifecycle,
            &invalid_utf8_request,
            std::ptr::from_ref(&context).cast_mut().cast(),
            Some(create_wallet_complete),
        )
    };
    assert_eq!(status, WalletEngineAbiStatus::InvalidUtf8);

    let unknown_network_request = WalletEngineCreateWalletRequest {
        record_id: WalletEngineStringView::from("wallet-1"),
        network: 2,
    };
    // SAFETY: The handle, request, and completion context remain live.
    let status = unsafe {
        wallet_engine_lifecycle_create_wallet(
            lifecycle,
            &unknown_network_request,
            std::ptr::from_ref(&context).cast_mut().cast(),
            Some(create_wallet_complete),
        )
    };
    assert_eq!(status, WalletEngineAbiStatus::InvalidArgument);

    assert!(receiver.try_recv().is_err());
    assert_eq!(context.stores.load(Ordering::Relaxed), 0);
    assert_eq!(context.completion_calls.load(Ordering::Relaxed), 0);
    // SAFETY: This is the live handle returned above.
    unsafe { wallet_engine_lifecycle_free(lifecycle) };
    wait_for_release(&context);
}

#[test]
fn invalid_record_id_is_reported_as_a_domain_error() {
    let (sender, receiver) = channel();
    let context = TestContext::new(sender);
    let callbacks = callback_table(&context, Some(store_success));
    // SAFETY: The callback table remains live through operation completion.
    let lifecycle = unsafe { lifecycle(&callbacks) };
    let request = WalletEngineCreateWalletRequest {
        record_id: WalletEngineStringView::from(""),
        network: WALLET_ENGINE_NETWORK_MAINNET,
    };

    // SAFETY: All pointers remain live for the start call and completion.
    let status = unsafe {
        wallet_engine_lifecycle_create_wallet(
            lifecycle,
            &request,
            std::ptr::from_ref(&context).cast_mut().cast(),
            Some(create_wallet_complete),
        )
    };
    assert_eq!(status, WalletEngineAbiStatus::Ok);
    // SAFETY: The operation owns an internal Arc after the successful start.
    unsafe { wallet_engine_lifecycle_free(lifecycle) };

    assert_eq!(
        receive(&receiver),
        CompletionResult {
            abi_status: WalletEngineAbiStatus::Ok,
            wallet: None,
            error: Some(LifecycleError {
                code: WalletEngineWalletLifecycleErrorCode::InvalidRecordId,
                protected_secret_host_error_kind: None,
                diagnostic: String::new(),
            }),
            valid_pointer_shape: true,
        }
    );
    assert_eq!(context.stores.load(Ordering::Relaxed), 0);
    wait_for_release(&context);
    assert_eq!(context.completion_calls.load(Ordering::Relaxed), 1);
}

fn run_success_case(store: WalletEngineStoreProtectedSecretFn, network: WalletEngineNetwork) {
    let (sender, receiver) = channel();
    let context = TestContext::new(sender);
    let callbacks = callback_table(&context, store);
    // SAFETY: The callback table remains live through operation completion.
    let lifecycle = unsafe { lifecycle(&callbacks) };
    let request = WalletEngineCreateWalletRequest {
        record_id: WalletEngineStringView::from("wallet-1"),
        network,
    };

    // SAFETY: All pointers remain live for the start call and completion.
    let status = unsafe {
        wallet_engine_lifecycle_create_wallet(
            lifecycle,
            &request,
            std::ptr::from_ref(&context).cast_mut().cast(),
            Some(create_wallet_complete),
        )
    };
    assert_eq!(status, WalletEngineAbiStatus::Ok);
    assert_eq!(context.retains.load(Ordering::Acquire), 1);
    // SAFETY: The operation owns an internal Arc after the successful start.
    unsafe { wallet_engine_lifecycle_free(lifecycle) };

    let result = receive(&receiver);
    assert_eq!(result.abi_status, WalletEngineAbiStatus::Ok);
    assert!(result.valid_pointer_shape);
    assert_eq!(result.error, None);
    let Some(wallet) = result.wallet else {
        panic!("successful completion did not contain a wallet");
    };
    assert_eq!(wallet.record_id, "wallet-1");
    assert!(!wallet.address.is_empty());
    assert_eq!(wallet.network, network);
    assert_eq!(wallet.secret_ref, "wallet:wallet-1:mnemonic");
    assert_eq!(wallet.phrase.split_ascii_whitespace().count(), 24);

    assert_eq!(context.stores.load(Ordering::Relaxed), 1);
    let stored_guard = lock(&context.stored_secret);
    let Some(stored) = stored_guard.as_ref() else {
        panic!("storage callback did not copy the request");
    };
    assert_eq!(stored.secret_ref, wallet.secret_ref);
    assert_ne!(stored.bytes_len, 0);
    assert!(stored.require_user_presence);
    drop(stored_guard);
    wait_for_release(&context);
    assert_eq!(context.completion_calls.load(Ordering::Relaxed), 1);
}

#[test]
fn created_wallet_is_delivered_after_synchronous_host_completion() {
    run_success_case(Some(store_success), WALLET_ENGINE_NETWORK_MAINNET);
}

#[test]
fn created_wallet_is_delivered_after_asynchronous_host_completion() {
    run_success_case(Some(store_async_success), WALLET_ENGINE_NETWORK_TESTNET);
}

#[test]
fn protected_storage_failure_is_reported_as_a_domain_error() {
    for host_error_kind in [
        WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_NOT_FOUND,
        WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_AUTHENTICATION_FAILED,
        WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_CANCELLED,
        WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_UNAVAILABLE,
        WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_POLICY_VIOLATION,
        WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_OTHER,
    ] {
        let (sender, receiver) = channel();
        let context = TestContext::with_host_error_kind(sender, host_error_kind);
        let callbacks = callback_table(&context, Some(store_error));
        // SAFETY: The callback table remains live through operation completion.
        let lifecycle = unsafe { lifecycle(&callbacks) };
        let request = WalletEngineCreateWalletRequest {
            record_id: WalletEngineStringView::from("wallet-1"),
            network: WALLET_ENGINE_NETWORK_TESTNET,
        };

        // SAFETY: All pointers remain live for the start call and completion.
        let status = unsafe {
            wallet_engine_lifecycle_create_wallet(
                lifecycle,
                &request,
                std::ptr::from_ref(&context).cast_mut().cast(),
                Some(create_wallet_complete),
            )
        };
        assert_eq!(status, WalletEngineAbiStatus::Ok);
        // SAFETY: The operation owns an internal Arc after the successful start.
        unsafe { wallet_engine_lifecycle_free(lifecycle) };

        assert_eq!(
            receive(&receiver),
            CompletionResult {
                abi_status: WalletEngineAbiStatus::Ok,
                wallet: None,
                error: Some(LifecycleError {
                    code: WalletEngineWalletLifecycleErrorCode::ProtectedSecretHost,
                    protected_secret_host_error_kind: Some(host_error_kind),
                    diagnostic: "protected storage failure".to_owned(),
                }),
                valid_pointer_shape: true,
            }
        );
        wait_for_release(&context);
        assert_eq!(context.completion_calls.load(Ordering::Relaxed), 1);
    }
}
