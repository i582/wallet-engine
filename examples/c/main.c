#include "wallet_engine.h"

#include <stdatomic.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <time.h>

#if defined(_WIN32)
#include <io.h>
#include <sys/stat.h>
#include <windows.h>
#else
#include <sched.h>
#include <sys/stat.h>
#endif

#define INPUT_CAPACITY 256
#define RECOVERY_WORD_COUNT 24
#define WALLET_METADATA_FILE "wallet_engine_wallets.tsv"
#define WALLET_SECRETS_FILE "wallet_engine_secrets.tsv"

typedef struct ExampleContext {
    atomic_bool completed;
    bool succeeded;
} ExampleContext;

// The context has process lifetime, so these callbacks need no memory
// management. Heap-owned application contexts should use reference counting.
static ExampleContext example_context;

static void retain_context(void *context) {
    (void)context;
}

static void release_context(void *context) {
    (void)context;
}

static bool write_view(FILE *file, WalletEngineStringView view) {
    if (view.data == NULL) {
        return view.len == 0;
    }
    return fwrite(view.data, 1, view.len, file) == view.len;
}

static bool print_view(WalletEngineStringView view) {
    return write_view(stdout, view);
}

static bool restrict_secret_file_permissions(void) {
#if defined(_WIN32)
    return _chmod(WALLET_SECRETS_FILE, _S_IREAD | _S_IWRITE) == 0;
#else
    return chmod(WALLET_SECRETS_FILE, S_IRUSR | S_IWUSR) == 0;
#endif
}

static void complete_storage_error(
    WalletEngineCompletionId completion_id,
    WalletEngineProtectedSecretHostErrorKind kind,
    const char *diagnostic
) {
    const WalletEngineProtectedSecretHostErrorView error = {
        .kind = kind,
        .diagnostic = {diagnostic, strlen(diagnostic)},
    };
    (void)wallet_engine_store_protected_secret_complete(completion_id, &error);
}

static void store_protected_secret(
    void *context,
    WalletEngineCompletionId completion_id,
    const WalletEngineProtectedSecretStoreView *request
) {
    (void)context;
    if (request == NULL || request->bytes.data == NULL || request->bytes.len == 0) {
        complete_storage_error(
            completion_id,
            WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_OTHER,
            "invalid file-storage request"
        );
        return;
    }

    FILE *file = fopen(WALLET_SECRETS_FILE, "ab");
    if (file == NULL) {
        complete_storage_error(
            completion_id,
            WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_UNAVAILABLE,
            "failed to open the secrets file"
        );
        return;
    }
    if (!restrict_secret_file_permissions()) {
        (void)fclose(file);
        complete_storage_error(
            completion_id,
            WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_POLICY_VIOLATION,
            "failed to restrict permissions on the secrets file"
        );
        return;
    }

    bool stored = write_view(file, request->secret_ref.value);
    stored = fputc('\t', file) != EOF && stored;
    stored = fputs(request->require_user_presence ? "true\t" : "false\t", file) >= 0 && stored;
    stored = fwrite(request->bytes.data, 1, request->bytes.len, file) == request->bytes.len && stored;
    stored = fputc('\n', file) != EOF && stored;
    stored = fclose(file) == 0 && stored;

    if (!stored) {
        complete_storage_error(
            completion_id,
            WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_OTHER,
            "failed to append the mnemonic to the secrets file"
        );
        return;
    }

    (void)wallet_engine_store_protected_secret_complete(completion_id, NULL);
}

static bool append_wallet_metadata(const WalletEngineCreatedWalletView *wallet) {
    FILE *file = fopen(WALLET_METADATA_FILE, "ab");
    if (file == NULL) {
        return false;
    }

    bool stored = write_view(file, wallet->descriptor.record_id);
    stored = fputc('\t', file) != EOF && stored;
    stored = fputs(
                 wallet->descriptor.network == WALLET_ENGINE_NETWORK_MAINNET ? "mainnet" :
                                                                               "testnet",
                 file
             ) >= 0 && stored;
    stored = fputc('\t', file) != EOF && stored;
    stored = write_view(file, wallet->descriptor.address) && stored;
    stored = fputc('\t', file) != EOF && stored;
    stored = write_view(file, wallet->descriptor.secret_ref.value) && stored;
    stored = fputc('\n', file) != EOF && stored;
    return fclose(file) == 0 && stored;
}

static bool print_created_wallet(const WalletEngineCreatedWalletView *wallet) {
    if (!append_wallet_metadata(wallet)) {
        fputs("Failed to append wallet metadata\n", stderr);
        return false;
    }

    fputs("\nRecord ID: ", stdout);
    bool valid = print_view(wallet->descriptor.record_id);
    fputs("\nAddress: ", stdout);
    valid = print_view(wallet->descriptor.address) && valid;
    printf(
        "\nNetwork: %s\nSecret reference: ",
        wallet->descriptor.network == WALLET_ENGINE_NETWORK_MAINNET ? "mainnet" : "testnet"
    );
    valid = print_view(wallet->descriptor.secret_ref.value) && valid;
    fputc('\n', stdout);

    const WalletEngineStringViewSlice words = wallet->recovery_phrase.words;
    if (words.data == NULL || words.len != RECOVERY_WORD_COUNT) {
        return false;
    }
    puts("Recovery phrase (display once and keep private):");
    for (size_t index = 0; index < words.len; ++index) {
        if (index != 0) {
            fputc(' ', stdout);
        }
        valid = print_view(words.data[index]) && valid;
    }
    fputc('\n', stdout);
    return valid;
}

static void print_lifecycle_error(const WalletEngineWalletLifecycleErrorView *error) {
    fprintf(stderr, "Wallet creation failed: code=%u", (unsigned)error->code);
    if (error->has_protected_secret_host_error_kind) {
        fprintf(
            stderr,
            ", host_kind=%u",
            (unsigned)error->protected_secret_host_error_kind
        );
    }
    if (error->diagnostic.len != 0) {
        fputs(", diagnostic=", stderr);
        (void)write_view(stderr, error->diagnostic);
    }
    fputc('\n', stderr);
}

static void create_wallet_complete(
    void *context,
    WalletEngineAbiStatus abi_status,
    const WalletEngineCreatedWalletView *wallet,
    const WalletEngineWalletLifecycleErrorView *error
) {
    ExampleContext *example = context;
    if (abi_status == WALLET_ENGINE_ABI_STATUS_OK && wallet != NULL && error == NULL) {
        example->succeeded = print_created_wallet(wallet);
    } else if (
        abi_status == WALLET_ENGINE_ABI_STATUS_OK && wallet == NULL && error != NULL
    ) {
        print_lifecycle_error(error);
    } else {
        fprintf(stderr, "Wallet creation failed at ABI boundary: %u\n", (unsigned)abi_status);
    }

    atomic_store_explicit(&example->completed, true, memory_order_release);
}

static void yield_thread(void) {
#if defined(_WIN32)
    (void)SwitchToThread();
#else
    (void)sched_yield();
#endif
}

static bool wait_for_completion(const ExampleContext *context) {
    const time_t deadline = time(NULL) + 30;
    while (!atomic_load_explicit(&context->completed, memory_order_acquire)) {
        if (time(NULL) > deadline) {
            return false;
        }
        yield_thread();
    }
    return true;
}

static bool read_line(char *buffer, size_t capacity) {
    if (fgets(buffer, (int)capacity, stdin) == NULL) {
        return false;
    }
    buffer[strcspn(buffer, "\r\n")] = '\0';
    return true;
}

static WalletEngineNetwork prompt_network(void) {
    char input[INPUT_CAPACITY];
    fputs("Network [1 = testnet, 2 = mainnet]: ", stdout);
    if (!read_line(input, sizeof(input))) {
        return WALLET_ENGINE_NETWORK_TESTNET;
    }
    return strcmp(input, "2") == 0 ? WALLET_ENGINE_NETWORK_MAINNET :
                                      WALLET_ENGINE_NETWORK_TESTNET;
}

static void create_wallet(WalletEngineLifecycle *lifecycle) {
    char record_id[INPUT_CAPACITY];
    fputs("Record ID: ", stdout);
    if (!read_line(record_id, sizeof(record_id)) || record_id[0] == '\0') {
        fputs("Record ID is required\n", stderr);
        return;
    }

    const WalletEngineCreateWalletRequest request = {
        .record_id = {record_id, strlen(record_id)},
        .network = prompt_network(),
    };
    example_context.succeeded = false;
    atomic_store_explicit(&example_context.completed, false, memory_order_relaxed);
    const WalletEngineAbiStatus status = wallet_engine_lifecycle_create_wallet(
        lifecycle,
        &request,
        &example_context,
        create_wallet_complete
    );
    if (status != WALLET_ENGINE_ABI_STATUS_OK) {
        fprintf(stderr, "Failed to start wallet creation: ABI status %u\n", (unsigned)status);
        return;
    }
    if (!wait_for_completion(&example_context)) {
        fputs("Timed out waiting for wallet creation\n", stderr);
        return;
    }
    if (example_context.succeeded) {
        printf("Saved metadata to %s\n", WALLET_METADATA_FILE);
        printf("Saved plaintext mnemonic to %s\n", WALLET_SECRETS_FILE);
    }
}

static void list_wallets(void) {
    FILE *file = fopen(WALLET_METADATA_FILE, "rb");
    if (file == NULL) {
        puts("No saved wallets.");
        return;
    }

    puts("\nrecord_id\tnetwork\taddress\tsecret_ref");
    char line[1024];
    while (fgets(line, sizeof(line), file) != NULL) {
        fputs(line, stdout);
    }
    fputc('\n', stdout);
    (void)fclose(file);
}

static void run_menu(WalletEngineLifecycle *lifecycle) {
    char choice[INPUT_CAPACITY];
    for (;;) {
        puts("\nWallet Engine C example");
        puts("1. Create wallet");
        puts("2. List saved wallets");
        puts("3. Exit");
        fputs("> ", stdout);
        if (!read_line(choice, sizeof(choice)) || strcmp(choice, "3") == 0) {
            return;
        }
        if (strcmp(choice, "1") == 0) {
            create_wallet(lifecycle);
        } else if (strcmp(choice, "2") == 0) {
            list_wallets();
        } else {
            puts("Unknown menu item.");
        }
    }
}

int main(void) {
    atomic_init(&example_context.completed, false);
    puts("WARNING: this example stores recovery phrases in plaintext files.");

    const WalletEnginePlatformHostCallbacks callbacks = {
        .struct_size = sizeof(WalletEnginePlatformHostCallbacks),
        .context = &example_context,
        .retain = retain_context,
        .release = release_context,
        .store_protected_secret = store_protected_secret,
    };
    WalletEngineLifecycle *lifecycle = NULL;
    const WalletEngineAbiStatus status = wallet_engine_lifecycle_new(&callbacks, &lifecycle);
    if (status != WALLET_ENGINE_ABI_STATUS_OK) {
        fprintf(stderr, "Failed to create lifecycle: ABI status %u\n", (unsigned)status);
        return 1;
    }

    run_menu(lifecycle);
    wallet_engine_lifecycle_free(lifecycle);
    return 0;
}
