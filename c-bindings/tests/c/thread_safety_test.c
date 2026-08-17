#include "wallet_engine.h"

#include <stdatomic.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdio.h>
#include <string.h>

#if defined(_WIN32)
#include <windows.h>
#else
#include <pthread.h>
#include <sched.h>
#endif

typedef int (*ThreadFunction)(void *context);

#if defined(_WIN32)
typedef DWORD TestThreadId;

typedef struct TestThread {
    HANDLE handle;
    ThreadFunction function;
    void *context;
    int result;
} TestThread;

static DWORD WINAPI thread_entry(void *context) {
    TestThread *thread = context;
    thread->result = thread->function(thread->context);
    return 0;
}

static bool thread_start(TestThread *thread, ThreadFunction function, void *context) {
    thread->function = function;
    thread->context = context;
    thread->result = -1;
    thread->handle = CreateThread(NULL, 0, thread_entry, thread, 0, NULL);
    return thread->handle != NULL;
}

static bool thread_join(TestThread *thread, int *result) {
    if (WaitForSingleObject(thread->handle, INFINITE) != WAIT_OBJECT_0) {
        return false;
    }
    *result = thread->result;
    return CloseHandle(thread->handle) != 0;
}

static void thread_yield(void) {
    (void)SwitchToThread();
}

static TestThreadId current_thread_id(void) {
    return GetCurrentThreadId();
}

static bool thread_ids_equal(TestThreadId first, TestThreadId second) {
    return first == second;
}
#else
typedef pthread_t TestThreadId;

typedef struct TestThread {
    pthread_t handle;
    ThreadFunction function;
    void *context;
    int result;
} TestThread;

static void *thread_entry(void *context) {
    TestThread *thread = context;
    thread->result = thread->function(thread->context);
    return NULL;
}

static bool thread_start(TestThread *thread, ThreadFunction function, void *context) {
    thread->function = function;
    thread->context = context;
    thread->result = -1;
    return pthread_create(&thread->handle, NULL, thread_entry, thread) == 0;
}

static bool thread_join(TestThread *thread, int *result) {
    if (pthread_join(thread->handle, NULL) != 0) {
        return false;
    }
    *result = thread->result;
    return true;
}

static void thread_yield(void) {
    (void)sched_yield();
}

static TestThreadId current_thread_id(void) {
    return pthread_self();
}

static bool thread_ids_equal(TestThreadId first, TestThreadId second) {
    return pthread_equal(first, second) != 0;
}
#endif

typedef struct ThreadGate {
    atomic_size_t ready;
    atomic_bool open;
} ThreadGate;

typedef struct HostContext {
    atomic_size_t retains;
    atomic_size_t releases;
    atomic_size_t stores;
    atomic_bool valid;
    _Atomic(WalletEngineProtectedSecretStoreCompletion *) pending_completion;
} HostContext;

typedef struct OperationTask {
    WalletEngineLifecycle *lifecycle;
    const char *record_id;
    bool expect_wallet;
    ThreadGate *gate;
    WalletEngineCreateWalletOperation *operation;
    WalletEngineAbiStatus start_status;
    WalletEngineAbiStatus poll_status;
    WalletEngineOperationPollState poll_state;
    TestThreadId poll_thread;
    bool result_called;
    bool result_thread_matches;
    bool result_shape_valid;
} OperationTask;

typedef struct CompletionTask {
    WalletEngineProtectedSecretStoreCompletion *completion;
    WalletEngineAbiStatus status;
} CompletionTask;

static void gate_init(ThreadGate *gate) {
    atomic_init(&gate->ready, 0);
    atomic_init(&gate->open, false);
}

static void gate_wait(ThreadGate *gate) {
    atomic_fetch_add_explicit(&gate->ready, 1, memory_order_release);
    while (!atomic_load_explicit(&gate->open, memory_order_acquire)) {
        thread_yield();
    }
}

static void gate_open(ThreadGate *gate, size_t expected_threads) {
    while (atomic_load_explicit(&gate->ready, memory_order_acquire) != expected_threads) {
        thread_yield();
    }
    atomic_store_explicit(&gate->open, true, memory_order_release);
}

static void retain_context(void *context) {
    HostContext *host = context;
    atomic_fetch_add_explicit(&host->retains, 1, memory_order_relaxed);
}

static void release_context(void *context) {
    HostContext *host = context;
    atomic_fetch_add_explicit(&host->releases, 1, memory_order_release);
}

static void store_protected_secret(
    void *context,
    WalletEngineProtectedSecretStoreCompletion *completion,
    const WalletEngineProtectedSecretStoreView *request
) {
    HostContext *host = context;
    const bool valid = completion != NULL && request != NULL && request->bytes.data != NULL &&
                       request->bytes.len != 0 && request->secret_ref.value.data != NULL &&
                       request->secret_ref.value.len != 0;
    if (!valid) {
        atomic_store_explicit(&host->valid, false, memory_order_relaxed);
    }
    atomic_fetch_add_explicit(&host->stores, 1, memory_order_relaxed);
    atomic_store_explicit(&host->pending_completion, completion, memory_order_release);
}

static void create_wallet_result(
    void *context,
    WalletEngineAbiStatus abi_status,
    const WalletEngineCreatedWalletView *wallet,
    const WalletEngineWalletLifecycleErrorView *error
) {
    OperationTask *task = context;
    task->result_called = true;
    task->result_thread_matches = thread_ids_equal(task->poll_thread, current_thread_id());
    task->result_shape_valid = task->expect_wallet ?
                                          abi_status == WALLET_ENGINE_ABI_STATUS_OK &&
                                              wallet != NULL && error == NULL :
                                          abi_status == WALLET_ENGINE_ABI_STATUS_OK &&
                                              wallet == NULL && error != NULL;
}

static int start_operation(void *context) {
    OperationTask *task = context;
    gate_wait(task->gate);
    const WalletEngineCreateWalletRequest request = {
        .record_id = {task->record_id, strlen(task->record_id)},
        .network = WALLET_ENGINE_NETWORK_TESTNET,
    };
    task->start_status = wallet_engine_lifecycle_create_wallet_start(
        task->lifecycle,
        &request,
        &task->operation
    );
    return 0;
}

static int poll_operation(void *context) {
    OperationTask *task = context;
    if (task->gate != NULL) {
        gate_wait(task->gate);
    }
    task->poll_thread = current_thread_id();
    task->poll_state = WALLET_ENGINE_OPERATION_POLL_STATE_PENDING;
    task->poll_status = wallet_engine_create_wallet_operation_poll(
        task->operation,
        task,
        create_wallet_result,
        &task->poll_state
    );
    return 0;
}

static int complete_store(void *context) {
    CompletionTask *task = context;
    task->status = wallet_engine_protected_secret_store_completion_complete(
        task->completion,
        NULL
    );
    wallet_engine_protected_secret_store_completion_free(task->completion);
    return 0;
}

#define CHECK(condition)                                                       \
    do {                                                                       \
        if (!(condition)) {                                                    \
            fprintf(                                                           \
                stderr,                                                        \
                "%s:%d: check failed: %s\n",                                 \
                __FILE__,                                                      \
                __LINE__,                                                      \
                #condition                                                     \
            );                                                                 \
            return 1;                                                          \
        }                                                                      \
    } while (0)

int main(void) {
    HostContext host;
    atomic_init(&host.retains, 0);
    atomic_init(&host.releases, 0);
    atomic_init(&host.stores, 0);
    atomic_init(&host.valid, true);
    atomic_init(&host.pending_completion, NULL);
    const WalletEnginePlatformHostCallbacks callbacks = {
        .struct_size = sizeof(WalletEnginePlatformHostCallbacks),
        .context = &host,
        .retain = retain_context,
        .release = release_context,
        .store_protected_secret = store_protected_secret,
    };
    WalletEngineLifecycle *lifecycle = NULL;
    CHECK(
        wallet_engine_lifecycle_new(&callbacks, &lifecycle) ==
        WALLET_ENGINE_ABI_STATUS_OK
    );
    CHECK(lifecycle != NULL);

    ThreadGate start_gate;
    gate_init(&start_gate);
    OperationTask invalid = {
        .lifecycle = lifecycle,
        .record_id = "",
        .expect_wallet = false,
        .gate = &start_gate,
    };
    OperationTask valid = {
        .lifecycle = lifecycle,
        .record_id = "thread-wallet",
        .expect_wallet = true,
        .gate = &start_gate,
    };
    TestThread first_start;
    TestThread second_start;
    CHECK(thread_start(&first_start, start_operation, &invalid));
    CHECK(thread_start(&second_start, start_operation, &valid));
    gate_open(&start_gate, 2);
    int thread_result = -1;
    CHECK(thread_join(&first_start, &thread_result));
    CHECK(thread_result == 0);
    CHECK(thread_join(&second_start, &thread_result));
    CHECK(thread_result == 0);
    CHECK(invalid.start_status == WALLET_ENGINE_ABI_STATUS_OK);
    CHECK(valid.start_status == WALLET_ENGINE_ABI_STATUS_OK);
    CHECK(invalid.operation != NULL);
    CHECK(valid.operation != NULL);

    wallet_engine_lifecycle_free(lifecycle);
    lifecycle = NULL;
    CHECK(atomic_load_explicit(&host.releases, memory_order_acquire) == 0);

    ThreadGate poll_gate;
    gate_init(&poll_gate);
    invalid.gate = &poll_gate;
    valid.gate = &poll_gate;
    TestThread first_poll;
    TestThread second_poll;
    CHECK(thread_start(&first_poll, poll_operation, &invalid));
    CHECK(thread_start(&second_poll, poll_operation, &valid));
    gate_open(&poll_gate, 2);
    CHECK(thread_join(&first_poll, &thread_result));
    CHECK(thread_result == 0);
    CHECK(thread_join(&second_poll, &thread_result));
    CHECK(thread_result == 0);

    CHECK(invalid.poll_status == WALLET_ENGINE_ABI_STATUS_OK);
    CHECK(invalid.poll_state == WALLET_ENGINE_OPERATION_POLL_STATE_READY);
    CHECK(invalid.result_called);
    CHECK(invalid.result_thread_matches);
    CHECK(invalid.result_shape_valid);
    CHECK(valid.poll_status == WALLET_ENGINE_ABI_STATUS_OK);
    CHECK(valid.poll_state == WALLET_ENGINE_OPERATION_POLL_STATE_PENDING);
    CHECK(!valid.result_called);
    CHECK(atomic_load_explicit(&host.stores, memory_order_acquire) == 1);

    CompletionTask completion = {
        .completion = atomic_exchange_explicit(
            &host.pending_completion,
            NULL,
            memory_order_acq_rel
        ),
        .status = WALLET_ENGINE_ABI_STATUS_PANIC,
    };
    CHECK(completion.completion != NULL);
    TestThread completion_thread;
    CHECK(thread_start(&completion_thread, complete_store, &completion));
    CHECK(thread_join(&completion_thread, &thread_result));
    CHECK(thread_result == 0);
    CHECK(completion.status == WALLET_ENGINE_ABI_STATUS_OK);
    CHECK(!valid.result_called);

    valid.gate = NULL;
    TestThread final_poll;
    CHECK(thread_start(&final_poll, poll_operation, &valid));
    CHECK(thread_join(&final_poll, &thread_result));
    CHECK(thread_result == 0);
    CHECK(valid.poll_status == WALLET_ENGINE_ABI_STATUS_OK);
    CHECK(valid.poll_state == WALLET_ENGINE_OPERATION_POLL_STATE_READY);
    CHECK(valid.result_called);
    CHECK(valid.result_thread_matches);
    CHECK(valid.result_shape_valid);

    wallet_engine_create_wallet_operation_free(invalid.operation);
    wallet_engine_create_wallet_operation_free(valid.operation);
    CHECK(atomic_load_explicit(&host.retains, memory_order_acquire) == 1);
    CHECK(atomic_load_explicit(&host.releases, memory_order_acquire) == 1);
    CHECK(atomic_load_explicit(&host.valid, memory_order_acquire));

    puts("Wallet Engine C thread-safety test passed");
    return 0;
}
