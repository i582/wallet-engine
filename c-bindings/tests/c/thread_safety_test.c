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
static void thread_yield(void) {(void)SwitchToThread();}
static TestThreadId current_thread_id(void) {return GetCurrentThreadId();}
static bool thread_ids_equal(TestThreadId first, TestThreadId second) {return first == second;}
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
    return pthread_create(&thread->handle, NULL, thread_entry, thread) == 0;
}
static bool thread_join(TestThread *thread, int *result) {
    if (pthread_join(thread->handle, NULL) != 0) {
        return false;
    }
    *result = thread->result;
    return true;
}
static void thread_yield(void) {(void)sched_yield();}
static TestThreadId current_thread_id(void) {return pthread_self();}
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
} HostContext;

typedef struct CallTask {
    WalletEngineLifecycle *lifecycle;
    const char *record_id;
    ThreadGate *gate;
    WalletEngineAbiStatus status;
    TestThreadId call_thread;
    bool result_called;
    bool result_thread_matches;
    bool result_shape_valid;
} CallTask;

static void gate_wait(ThreadGate *gate) {
    atomic_fetch_add_explicit(&gate->ready, 1, memory_order_release);
    while (!atomic_load_explicit(&gate->open, memory_order_acquire)) {
        thread_yield();
    }
}

static void retain_context(void *context) {
    atomic_fetch_add_explicit(&((HostContext *)context)->retains, 1, memory_order_relaxed);
}

static void release_context(void *context) {
    atomic_fetch_add_explicit(&((HostContext *)context)->releases, 1, memory_order_release);
}

static void store_protected_secret(
    void *context,
    const WalletEngineProtectedSecretStoreView *request,
    void *result_context,
    WalletEngineProtectedSecretStoreResultFn result
) {
    HostContext *host = context;
    if (request == NULL || request->bytes.data == NULL || request->bytes.len == 0 ||
        result == NULL) {
        atomic_store_explicit(&host->valid, false, memory_order_relaxed);
        return;
    }
    atomic_fetch_add_explicit(&host->stores, 1, memory_order_relaxed);
    if (result(result_context, NULL) != WALLET_ENGINE_ABI_STATUS_OK) {
        atomic_store_explicit(&host->valid, false, memory_order_relaxed);
    }
}

static void create_wallet_result(
    void *context,
    WalletEngineAbiStatus abi_status,
    const WalletEngineCreatedWalletView *wallet,
    const WalletEngineWalletLifecycleErrorView *error
) {
    CallTask *task = context;
    task->result_called = true;
    task->result_thread_matches = thread_ids_equal(task->call_thread, current_thread_id());
    task->result_shape_valid = abi_status == WALLET_ENGINE_ABI_STATUS_OK &&
                               wallet != NULL && error == NULL;
}

static int call_create_wallet(void *context) {
    CallTask *task = context;
    gate_wait(task->gate);
    task->call_thread = current_thread_id();
    const WalletEngineCreateWalletRequest request = {
        .record_id = {task->record_id, strlen(task->record_id)},
        .network = WALLET_ENGINE_NETWORK_TESTNET,
    };
    task->status = wallet_engine_lifecycle_create_wallet(
        task->lifecycle,
        &request,
        task,
        create_wallet_result
    );
    return 0;
}

#define CHECK(condition)                                                       \
    do {                                                                       \
        if (!(condition)) {                                                    \
            fprintf(stderr, "%s:%d: check failed: %s\n", __FILE__, __LINE__, #condition); \
            return 1;                                                          \
        }                                                                      \
    } while (0)

int main(void) {
    HostContext host;
    atomic_init(&host.retains, 0);
    atomic_init(&host.releases, 0);
    atomic_init(&host.stores, 0);
    atomic_init(&host.valid, true);
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

    ThreadGate gate;
    atomic_init(&gate.ready, 0);
    atomic_init(&gate.open, false);
    CallTask first = {.lifecycle = lifecycle, .record_id = "thread-wallet-1", .gate = &gate};
    CallTask second = {.lifecycle = lifecycle, .record_id = "thread-wallet-2", .gate = &gate};
    TestThread first_thread;
    TestThread second_thread;
    CHECK(thread_start(&first_thread, call_create_wallet, &first));
    CHECK(thread_start(&second_thread, call_create_wallet, &second));
    while (atomic_load_explicit(&gate.ready, memory_order_acquire) != 2) {
        thread_yield();
    }
    atomic_store_explicit(&gate.open, true, memory_order_release);

    int thread_result = -1;
    CHECK(thread_join(&first_thread, &thread_result));
    CHECK(thread_result == 0);
    CHECK(thread_join(&second_thread, &thread_result));
    CHECK(thread_result == 0);
    CHECK(first.status == WALLET_ENGINE_ABI_STATUS_OK);
    CHECK(second.status == WALLET_ENGINE_ABI_STATUS_OK);
    CHECK(first.result_called && second.result_called);
    CHECK(first.result_thread_matches && second.result_thread_matches);
    CHECK(first.result_shape_valid && second.result_shape_valid);
    CHECK(!thread_ids_equal(first.call_thread, second.call_thread));
    CHECK(atomic_load_explicit(&host.stores, memory_order_acquire) == 2);
    CHECK(atomic_load_explicit(&host.valid, memory_order_acquire));

    wallet_engine_lifecycle_free(lifecycle);
    CHECK(atomic_load_explicit(&host.retains, memory_order_acquire) == 1);
    CHECK(atomic_load_explicit(&host.releases, memory_order_acquire) == 1);
    puts("Wallet Engine synchronous C thread-safety test passed");
    return 0;
}
