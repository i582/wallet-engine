#![allow(unsafe_code)]
#![allow(clippy::expect_used)]
#![allow(
    clippy::undocumented_unsafe_blocks,
    reason = "the integration test documents FFI lifetime invariants around grouped assertions"
)]

use std::{
    ffi::c_void,
    sync::{
        Arc, Barrier, Mutex, MutexGuard,
        atomic::{AtomicPtr, AtomicUsize, Ordering},
    },
    thread::ThreadId,
};

use wallet_engine_c::{
    WALLET_ENGINE_NETWORK_TESTNET, WALLET_ENGINE_PLATFORM_HOST_CALLBACKS_SIZE,
    WalletEngineAbiStatus, WalletEngineCreateWalletOperation, WalletEngineCreateWalletRequest,
    WalletEngineCreatedWalletView, WalletEngineLifecycle, WalletEngineOperationPollState,
    WalletEnginePlatformHostCallbacks, WalletEngineProtectedSecretStoreCompletion,
    WalletEngineProtectedSecretStoreView, WalletEngineStringView,
    WalletEngineWalletLifecycleErrorView, wallet_engine_create_wallet_operation_free,
    wallet_engine_create_wallet_operation_poll, wallet_engine_lifecycle_create_wallet_start,
    wallet_engine_lifecycle_free, wallet_engine_lifecycle_new,
    wallet_engine_protected_secret_store_completion_complete,
    wallet_engine_protected_secret_store_completion_free,
};

#[derive(Default)]
struct PollObservation {
    host_threads: Vec<ThreadId>,
    result_threads: Vec<ThreadId>,
    valid_results: Vec<bool>,
}

struct TestContext {
    retains: AtomicUsize,
    releases: AtomicUsize,
    stores: AtomicUsize,
    results: AtomicUsize,
    pending_completion: AtomicPtr<WalletEngineProtectedSecretStoreCompletion>,
    observation: Mutex<PollObservation>,
    store_entered: Option<Barrier>,
    release_store: Option<Barrier>,
}

impl TestContext {
    fn regular() -> Self {
        Self {
            retains: AtomicUsize::new(0),
            releases: AtomicUsize::new(0),
            stores: AtomicUsize::new(0),
            results: AtomicUsize::new(0),
            pending_completion: AtomicPtr::new(std::ptr::null_mut()),
            observation: Mutex::new(PollObservation::default()),
            store_entered: None,
            release_store: None,
        }
    }

    fn blocking_store() -> Self {
        Self {
            store_entered: Some(Barrier::new(2)),
            release_store: Some(Barrier::new(2)),
            ..Self::regular()
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
    // SAFETY: Every callback table and result callback in this test uses a
    // live `TestContext` pointer.
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
        .fetch_add(1, Ordering::Relaxed);
}

unsafe fn record_store(
    context: *mut c_void,
    completion: *mut WalletEngineProtectedSecretStoreCompletion,
) {
    // SAFETY: The callback table supplies a live `TestContext` pointer.
    let context = unsafe { test_context(context) };
    context.stores.fetch_add(1, Ordering::Relaxed);
    context
        .pending_completion
        .store(completion, Ordering::Relaxed);
    lock(&context.observation)
        .host_threads
        .push(std::thread::current().id());
}

unsafe extern "C" fn store_synchronously(
    context: *mut c_void,
    completion: *mut WalletEngineProtectedSecretStoreCompletion,
    _request: *const WalletEngineProtectedSecretStoreView,
) {
    // SAFETY: The callback table supplies a live context and completion handle.
    unsafe { record_store(context, completion) };
    // SAFETY: Null denotes a successful completion and is not dereferenced.
    let _status = unsafe {
        wallet_engine_protected_secret_store_completion_complete(completion, std::ptr::null())
    };
    // SAFETY: The callback owns this handle and has finished using it.
    unsafe { wallet_engine_protected_secret_store_completion_free(completion) };
}

unsafe extern "C" fn store_later(
    context: *mut c_void,
    completion: *mut WalletEngineProtectedSecretStoreCompletion,
    _request: *const WalletEngineProtectedSecretStoreView,
) {
    // SAFETY: The callback table supplies a live context and completion handle.
    unsafe { record_store(context, completion) };
}

unsafe extern "C" fn store_while_blocking_poll(
    context: *mut c_void,
    completion: *mut WalletEngineProtectedSecretStoreCompletion,
    _request: *const WalletEngineProtectedSecretStoreView,
) {
    // SAFETY: The callback table supplies a live context and completion handle.
    unsafe { record_store(context, completion) };
    // SAFETY: The callback table supplies a live `TestContext` pointer.
    let context = unsafe { test_context(context) };
    if let Some(entered) = &context.store_entered {
        entered.wait();
    }
    if let Some(release) = &context.release_store {
        release.wait();
    }
    // SAFETY: Null denotes a successful completion and is not dereferenced.
    let _status = unsafe {
        wallet_engine_protected_secret_store_completion_complete(completion, std::ptr::null())
    };
    // SAFETY: The callback owns this handle and has finished using it.
    unsafe { wallet_engine_protected_secret_store_completion_free(completion) };
}

unsafe extern "C" fn record_result(
    context: *mut c_void,
    abi_status: WalletEngineAbiStatus,
    wallet: *const WalletEngineCreatedWalletView,
    error: *const WalletEngineWalletLifecycleErrorView,
) {
    let valid = abi_status == WalletEngineAbiStatus::Ok
        && ((!wallet.is_null() && error.is_null()) || (wallet.is_null() && !error.is_null()));
    // SAFETY: The poll call supplies a live `TestContext` for this synchronous
    // result callback.
    let context = unsafe { test_context(context) };
    context.results.fetch_add(1, Ordering::Relaxed);
    let mut observation = lock(&context.observation);
    observation.result_threads.push(std::thread::current().id());
    observation.valid_results.push(valid);
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
    // SAFETY: The callbacks and output pointer remain valid for this call.
    let status = unsafe { wallet_engine_lifecycle_new(callbacks, &mut lifecycle) };
    assert_eq!(status, WalletEngineAbiStatus::Ok);
    assert!(!lifecycle.is_null());
    lifecycle
}

unsafe fn start_operation(
    lifecycle: *const WalletEngineLifecycle,
    record_id: &str,
) -> *mut WalletEngineCreateWalletOperation {
    let request = WalletEngineCreateWalletRequest {
        record_id: WalletEngineStringView::from(record_id),
        network: WALLET_ENGINE_NETWORK_TESTNET,
    };
    let mut operation = std::ptr::null_mut();
    // SAFETY: The lifecycle, request, nested string, and output pointer remain
    // valid for this call.
    let status =
        unsafe { wallet_engine_lifecycle_create_wallet_start(lifecycle, &request, &mut operation) };
    assert_eq!(status, WalletEngineAbiStatus::Ok);
    assert!(!operation.is_null());
    operation
}

unsafe fn poll_operation(
    operation: *mut WalletEngineCreateWalletOperation,
    context: &TestContext,
) -> (WalletEngineAbiStatus, WalletEngineOperationPollState) {
    let mut state = WalletEngineOperationPollState::Ready;
    // SAFETY: The operation and context remain live for this call; `state` is
    // writable and the result callback is synchronous.
    let status = unsafe {
        wallet_engine_create_wallet_operation_poll(
            operation,
            std::ptr::from_ref(context).cast_mut().cast(),
            Some(record_result),
            &mut state,
        )
    };
    (status, state)
}

#[test]
fn client_driven_api_validates_arguments_before_advancing() {
    let context = TestContext::regular();
    let callbacks = callbacks(&context, store_synchronously);
    // SAFETY: The callback table and output storage remain live.
    let lifecycle = unsafe { lifecycle(&callbacks) };
    let request = WalletEngineCreateWalletRequest {
        record_id: WalletEngineStringView::from(""),
        network: WALLET_ENGINE_NETWORK_TESTNET,
    };

    // SAFETY: A null output pointer is rejected without being dereferenced.
    assert_eq!(
        unsafe {
            wallet_engine_lifecycle_create_wallet_start(lifecycle, &request, std::ptr::null_mut())
        },
        WalletEngineAbiStatus::InvalidArgument
    );
    let mut operation = std::ptr::dangling_mut::<WalletEngineCreateWalletOperation>();
    // SAFETY: A null lifecycle is rejected and the writable output is cleared.
    assert_eq!(
        unsafe {
            wallet_engine_lifecycle_create_wallet_start(std::ptr::null(), &request, &mut operation)
        },
        WalletEngineAbiStatus::InvalidArgument
    );
    assert!(operation.is_null());
    // SAFETY: A null request is rejected and the writable output is cleared.
    assert_eq!(
        unsafe {
            wallet_engine_lifecycle_create_wallet_start(lifecycle, std::ptr::null(), &mut operation)
        },
        WalletEngineAbiStatus::InvalidArgument
    );
    assert!(operation.is_null());

    // SAFETY: All start arguments are valid and remain live for this call.
    operation = unsafe { start_operation(lifecycle, "") };
    let mut state = WalletEngineOperationPollState::Ready;
    // SAFETY: A null operation is rejected; the writable state is initialized
    // conservatively to pending.
    assert_eq!(
        unsafe {
            wallet_engine_create_wallet_operation_poll(
                std::ptr::null_mut(),
                std::ptr::from_ref(&context).cast_mut().cast(),
                Some(record_result),
                &mut state,
            )
        },
        WalletEngineAbiStatus::InvalidArgument
    );
    assert_eq!(state, WalletEngineOperationPollState::Pending);
    // SAFETY: A null result callback is rejected before the live operation is
    // polled.
    assert_eq!(
        unsafe {
            wallet_engine_create_wallet_operation_poll(
                operation,
                std::ptr::from_ref(&context).cast_mut().cast(),
                None,
                &mut state,
            )
        },
        WalletEngineAbiStatus::InvalidArgument
    );
    // SAFETY: A null state output is rejected before the live operation is
    // polled.
    assert_eq!(
        unsafe {
            wallet_engine_create_wallet_operation_poll(
                operation,
                std::ptr::from_ref(&context).cast_mut().cast(),
                Some(record_result),
                std::ptr::null_mut(),
            )
        },
        WalletEngineAbiStatus::InvalidArgument
    );
    assert_eq!(context.results.load(Ordering::Relaxed), 0);

    // SAFETY: The rejected calls above did not advance this operation.
    assert_eq!(
        unsafe { poll_operation(operation, &context) },
        (
            WalletEngineAbiStatus::Ok,
            WalletEngineOperationPollState::Ready,
        )
    );
    // SAFETY: Re-polling a completed live handle is rejected without another
    // result callback.
    assert_eq!(
        unsafe { poll_operation(operation, &context) },
        (
            WalletEngineAbiStatus::InvalidArgument,
            WalletEngineOperationPollState::Pending,
        )
    );
    assert_eq!(context.results.load(Ordering::Relaxed), 1);

    // SAFETY: The operation and lifecycle handles remain uniquely owned.
    unsafe {
        wallet_engine_create_wallet_operation_free(operation);
        wallet_engine_lifecycle_free(lifecycle);
    }
    assert_eq!(context.releases.load(Ordering::Relaxed), 1);
}

#[test]
fn start_only_copies_input_and_does_not_poll() {
    let context = TestContext::regular();
    let callbacks = callbacks(&context, store_synchronously);
    // SAFETY: Handles are used according to their unique ownership contracts.
    unsafe {
        let lifecycle = lifecycle(&callbacks);
        let operation = start_operation(lifecycle, "wallet-1");
        wallet_engine_lifecycle_free(lifecycle);

        assert_eq!(context.stores.load(Ordering::Relaxed), 0);
        assert_eq!(context.results.load(Ordering::Relaxed), 0);
        assert_eq!(context.releases.load(Ordering::Relaxed), 0);

        wallet_engine_create_wallet_operation_free(operation);
    }
    assert_eq!(context.releases.load(Ordering::Relaxed), 1);
}

#[test]
fn ready_result_and_host_callback_run_on_the_polling_thread() {
    let context = TestContext::regular();
    let callbacks = callbacks(&context, store_synchronously);
    // SAFETY: The lifecycle is live for operation construction.
    let lifecycle = unsafe { lifecycle(&callbacks) };
    // SAFETY: The lifecycle remains live for this start call.
    let operation = unsafe { start_operation(lifecycle, "wallet-thread") };
    // SAFETY: The operation owns the core lifecycle after start.
    unsafe { wallet_engine_lifecycle_free(lifecycle) };

    let operation_address = operation.addr();
    let context_address = std::ptr::from_ref(&context).addr();
    let poll_thread = std::thread::spawn(move || {
        // SAFETY: The parent keeps both allocations alive until this thread is
        // joined and no other thread polls or frees the operation.
        let operation = operation_address as *mut WalletEngineCreateWalletOperation;
        let context = unsafe { &*(context_address as *const TestContext) };
        let result = unsafe { poll_operation(operation, context) };
        (std::thread::current().id(), result)
    });
    let (poll_thread_id, (status, state)) = poll_thread.join().expect("poll thread panicked");

    assert_eq!(status, WalletEngineAbiStatus::Ok);
    assert_eq!(state, WalletEngineOperationPollState::Ready);
    assert_eq!(context.stores.load(Ordering::Relaxed), 1);
    assert_eq!(context.results.load(Ordering::Relaxed), 1);
    let observation = lock(&context.observation);
    assert_eq!(observation.host_threads.as_slice(), [poll_thread_id]);
    assert_eq!(observation.result_threads.as_slice(), [poll_thread_id]);
    assert_eq!(observation.valid_results.as_slice(), [true]);
    drop(observation);

    // SAFETY: The poll thread is joined and this is the unique live operation
    // handle.
    unsafe { wallet_engine_create_wallet_operation_free(operation) };
    assert_eq!(context.releases.load(Ordering::Relaxed), 1);
}

#[test]
fn lifecycle_can_create_operations_from_multiple_threads() {
    let context = TestContext::regular();
    let callbacks = callbacks(&context, store_synchronously);
    // SAFETY: The callback table remains live until all operations are freed.
    let lifecycle = unsafe { lifecycle(&callbacks) };
    let lifecycle_address = lifecycle.addr();
    let start_barrier = Arc::new(Barrier::new(3));

    let start_on_thread = || {
        let barrier = Arc::clone(&start_barrier);
        std::thread::spawn(move || {
            barrier.wait();
            let lifecycle = lifecycle_address as *const WalletEngineLifecycle;
            // Empty record IDs make both operations complete without invoking
            // the protected-storage callback.
            // SAFETY: The parent keeps the shared lifecycle live until both
            // start threads have joined.
            unsafe { start_operation(lifecycle, "").addr() }
        })
    };
    let first_thread = start_on_thread();
    let second_thread = start_on_thread();
    start_barrier.wait();
    let first = first_thread.join().expect("first start thread panicked")
        as *mut WalletEngineCreateWalletOperation;
    let second = second_thread.join().expect("second start thread panicked")
        as *mut WalletEngineCreateWalletOperation;

    // SAFETY: Both start calls have returned and this test uniquely owns the
    // lifecycle handle. Each operation retained the core lifecycle.
    unsafe { wallet_engine_lifecycle_free(lifecycle) };
    assert_eq!(context.releases.load(Ordering::Relaxed), 0);

    // SAFETY: The two distinct operations are live and uniquely owned here.
    assert_eq!(
        unsafe { poll_operation(first, &context) }.1,
        WalletEngineOperationPollState::Ready
    );
    // SAFETY: The two distinct operations are live and uniquely owned here.
    assert_eq!(
        unsafe { poll_operation(second, &context) }.1,
        WalletEngineOperationPollState::Ready
    );
    // SAFETY: Polling is complete and the handles are uniquely owned.
    unsafe {
        wallet_engine_create_wallet_operation_free(first);
        wallet_engine_create_wallet_operation_free(second);
    }
    assert_eq!(context.results.load(Ordering::Relaxed), 2);
    assert_eq!(context.releases.load(Ordering::Relaxed), 1);
}

#[test]
fn different_operations_can_be_polled_from_different_threads() {
    let context = TestContext::regular();
    let callbacks = callbacks(&context, store_synchronously);
    // SAFETY: The lifecycle is live while both operations are constructed.
    let lifecycle = unsafe { lifecycle(&callbacks) };
    // Empty record IDs become domain failures on the first poll without
    // calling the protected-storage host.
    // SAFETY: The lifecycle remains live for both start calls.
    let first = unsafe { start_operation(lifecycle, "") };
    // SAFETY: The lifecycle remains live for both start calls.
    let second = unsafe { start_operation(lifecycle, "") };
    // SAFETY: Both operation futures retain the core lifecycle.
    unsafe { wallet_engine_lifecycle_free(lifecycle) };

    let start_barrier = Arc::new(Barrier::new(3));
    let context_address = std::ptr::from_ref(&context).addr();
    let poll_on_thread = |operation: *mut WalletEngineCreateWalletOperation| {
        let operation_address = operation.addr();
        let barrier = Arc::clone(&start_barrier);
        std::thread::spawn(move || {
            barrier.wait();
            // SAFETY: The parent keeps the context live, each thread owns a
            // different operation, and both threads are joined before free.
            let context = unsafe { &*(context_address as *const TestContext) };
            let operation = operation_address as *mut WalletEngineCreateWalletOperation;
            let result = unsafe { poll_operation(operation, context) };
            (std::thread::current().id(), result)
        })
    };
    let first_thread = poll_on_thread(first);
    let second_thread = poll_on_thread(second);
    start_barrier.wait();
    let (first_thread_id, first_result) = first_thread.join().expect("first poll thread panicked");
    let (second_thread_id, second_result) =
        second_thread.join().expect("second poll thread panicked");

    assert_ne!(first_thread_id, second_thread_id);
    assert_eq!(
        first_result,
        (
            WalletEngineAbiStatus::Ok,
            WalletEngineOperationPollState::Ready,
        )
    );
    assert_eq!(first_result, second_result);
    assert_eq!(context.stores.load(Ordering::Relaxed), 0);
    assert_eq!(context.results.load(Ordering::Relaxed), 2);
    let observation = lock(&context.observation);
    assert!(observation.result_threads.contains(&first_thread_id));
    assert!(observation.result_threads.contains(&second_thread_id));
    drop(observation);

    // SAFETY: Both poll threads are joined and the handles remain uniquely
    // owned by this test.
    unsafe {
        wallet_engine_create_wallet_operation_free(first);
        wallet_engine_create_wallet_operation_free(second);
    }
    assert_eq!(context.releases.load(Ordering::Relaxed), 1);
}

#[test]
fn asynchronous_host_completion_requires_an_explicit_second_poll() {
    let context = TestContext::regular();
    let callbacks = callbacks(&context, store_later);
    // SAFETY: Handles are used according to their ownership contracts.
    unsafe {
        let lifecycle = lifecycle(&callbacks);
        let operation = start_operation(lifecycle, "wallet-pending");
        wallet_engine_lifecycle_free(lifecycle);

        assert_eq!(
            poll_operation(operation, &context),
            (
                WalletEngineAbiStatus::Ok,
                WalletEngineOperationPollState::Pending,
            )
        );
        assert_eq!(context.results.load(Ordering::Relaxed), 0);

        let completion = context.pending_completion.load(Ordering::Relaxed);
        assert!(!completion.is_null());
        let completion_address = completion as usize;
        let completion_thread = std::thread::spawn(move || {
            let completion = completion_address as *mut WalletEngineProtectedSecretStoreCompletion;
            // SAFETY: The host owns this live completion handle and may use it
            // from any client-owned thread.
            let status = wallet_engine_protected_secret_store_completion_complete(
                completion,
                std::ptr::null(),
            );
            wallet_engine_protected_secret_store_completion_free(completion);
            status
        });
        assert_eq!(
            completion_thread
                .join()
                .expect("completion thread panicked"),
            WalletEngineAbiStatus::Ok
        );

        assert_eq!(context.results.load(Ordering::Relaxed), 0);
        assert_eq!(
            poll_operation(operation, &context),
            (
                WalletEngineAbiStatus::Ok,
                WalletEngineOperationPollState::Ready,
            )
        );
        assert_eq!(context.results.load(Ordering::Relaxed), 1);
        wallet_engine_create_wallet_operation_free(operation);
    }
    assert_eq!(context.releases.load(Ordering::Relaxed), 1);
}

#[test]
fn concurrent_poll_returns_operation_busy_without_waiting() {
    let context = TestContext::blocking_store();
    let callbacks = callbacks(&context, store_while_blocking_poll);
    // SAFETY: The lifecycle is live for operation construction.
    let lifecycle = unsafe { lifecycle(&callbacks) };
    // SAFETY: The lifecycle remains live for this start call.
    let operation = unsafe { start_operation(lifecycle, "wallet-busy") };
    // SAFETY: The operation owns the core lifecycle after start.
    unsafe { wallet_engine_lifecycle_free(lifecycle) };

    let operation_address = operation.addr();
    let context_address = std::ptr::from_ref(&context).addr();
    let polling_thread = std::thread::spawn(move || {
        // SAFETY: The parent keeps both allocations live and waits for the
        // blocking host callback before attempting the supported second poll.
        let operation = operation_address as *mut WalletEngineCreateWalletOperation;
        let context = unsafe { &*(context_address as *const TestContext) };
        unsafe { poll_operation(operation, context) }
    });

    let Some(entered) = &context.store_entered else {
        panic!("blocking context has no entry barrier");
    };
    entered.wait();
    // SAFETY: The operation remains live; this concurrent poll must return
    // `OPERATION_BUSY` without touching the active future.
    assert_eq!(
        unsafe { poll_operation(operation, &context) },
        (
            WalletEngineAbiStatus::OperationBusy,
            WalletEngineOperationPollState::Pending,
        )
    );

    let Some(release) = &context.release_store else {
        panic!("blocking context has no release barrier");
    };
    release.wait();
    assert_eq!(
        polling_thread.join().expect("polling thread panicked"),
        (
            WalletEngineAbiStatus::Ok,
            WalletEngineOperationPollState::Ready,
        )
    );
    assert_eq!(context.results.load(Ordering::Relaxed), 1);

    // SAFETY: The polling thread is joined and this is the unique live handle.
    unsafe { wallet_engine_create_wallet_operation_free(operation) };
    assert_eq!(context.releases.load(Ordering::Relaxed), 1);
}

#[test]
fn host_completion_remains_safe_after_freeing_a_pending_operation() {
    let context = TestContext::regular();
    let callbacks = callbacks(&context, store_later);
    // SAFETY: Handles are used according to their ownership contracts.
    unsafe {
        let lifecycle = lifecycle(&callbacks);
        let operation = start_operation(lifecycle, "wallet-cancel");
        wallet_engine_lifecycle_free(lifecycle);
        assert_eq!(
            poll_operation(operation, &context).1,
            WalletEngineOperationPollState::Pending
        );
        let completion = context.pending_completion.load(Ordering::Relaxed);
        assert!(!completion.is_null());

        wallet_engine_create_wallet_operation_free(operation);
        assert_eq!(
            wallet_engine_protected_secret_store_completion_complete(completion, std::ptr::null(),),
            WalletEngineAbiStatus::InvalidArgument
        );
        wallet_engine_protected_secret_store_completion_free(completion);
        wallet_engine_create_wallet_operation_free(std::ptr::null_mut());
    }
    assert_eq!(context.results.load(Ordering::Relaxed), 0);
    assert_eq!(context.releases.load(Ordering::Relaxed), 1);
}
