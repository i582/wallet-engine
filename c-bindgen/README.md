# Wallet Engine C bindgen

Custom UniFFI backend that will generate the typed Wallet Engine C facade from
the `ComponentInterface` embedded in the compiled Rust library.

The generator currently:

- extracts and validates UniFFI metadata with `BindgenLoader`;
- derives the private UniFFI FFI functions;
- writes deterministic `wallet_engine.h`, `wallet_engine.c`, and
  `wallet_engine.c-api.json` artifacts;
- discovers the builtin types actually used by the component;
- generates UniFFI-compatible wire codecs for builtin values;
- generates public C types and codecs for flat non-error enums;
- generates public C wrappers and codecs for supported optional values;
- generates borrowed C list views and codecs for supported sequences;
- generates field-for-field C views and codecs for supported records;
- generates tag-plus-payload C values and codecs for supported rich errors;
- compiles the generated facade as strict C11 in its test suite.

Remaining compound types, callback adapters, and export lists will be added on
top of the normalized component model. The crate intentionally has no
dependency on the `wallet-engine` or `c-bindings` crates.

From the repository root, the experimental recipe builds the library and runs
the generator without touching the production `bindings/c` output:

```shell
just bindings-c-experimental
```

The output directory is always `bindings/c-experimental`.

To compile and execute the generated C codec tests with AddressSanitizer and
UndefinedBehaviorSanitizer, run:

```shell
just test-c-bindgen-sanitized
```

This sanitizer profile requires a C compiler with support for
`-fsanitize=address,undefined`.

Static C and header source lives in `templates/`, split into facade, public
type, and private codec blocks. Rust renderers choose the blocks required by
the normalized component model and substitute explicit `{{PLACEHOLDER}}`
values. Templates are embedded into the generator binary with `include_str!`,
so generation has no runtime template-file dependency. The small renderer
rejects missing and unresolved placeholders in tests.

The generator already produces a minimal compilable `wallet_engine.h` and
`wallet_engine.c` there. These files grow incrementally with each supported type
and callable instead of being postponed until the complete API model is
implemented. C++ compatibility is deliberately deferred until the C ABI is
complete and stable.

The current type slice discovers the builtins, flat non-error enums, supported
optional values, sequences, records, and rich declared errors used by the real
`ComponentInterface`, records their public Rust-to-C mapping in the manifest,
and generates borrowed views plus private wire helpers. Tests compile the
facade with strict C11 warnings and execute its codecs against known UniFFI
wire values.

## Rust to C mapping

This section is the concise source of truth for what the generator emits. A
mapping is marked **implemented** only when it is present in generated output
and covered by generator tests. Planned mappings describe the intended ABI but
must not be treated as available yet.

### Builtin values

| Rust / UniFFI | Public C | Status | Notes |
|---|---|---|---|
| `u8` | `uint8_t` | Implemented | From `<stdint.h>`. |
| `i8` | `int8_t` | Implemented | From `<stdint.h>`. |
| `u16` | `uint16_t` | Implemented | Used by the current API. |
| `i16` | `int16_t` | Implemented | From `<stdint.h>`. |
| `u32` | `uint32_t` | Implemented | Used by the current API. |
| `i32` | `int32_t` | Implemented | Used by the current API. |
| `u64` | `uint64_t` | Implemented | Used by the current API. |
| `i64` | `int64_t` | Implemented | From `<stdint.h>`. |
| `bool` | `bool` | Implemented | From `<stdbool.h>`; used by the current API. |
| `String` | `WalletEngineStringView` | Implemented | Borrowed UTF-8 bytes, not NUL-terminated. |
| `Vec<u8>` / UniFFI `Bytes` | `WalletEngineBytesView` | Implemented | Borrowed byte sequence. |
| `()` | no value payload | Planned | A completion still reports success or error. |
| `f32` | `float` | Planned | The current Wallet Engine API does not use it. |
| `f64` | `double` | Planned | The current Wallet Engine API does not use it. |

Only reachable types are recorded in `wallet_engine.c-api.json`. At the moment
the real Wallet Engine component uses `u16`, `u32`, `i32`, `u64`, `bool`,
`String`, and `Vec<u8>`.

### Borrowed views

```rust
String
```

becomes:

```c
typedef struct WalletEngineStringView {
    const char *data;
    size_t len;
} WalletEngineStringView;
```

```rust
Vec<u8>
```

becomes:

```c
typedef struct WalletEngineBytesView {
    const uint8_t *data;
    size_t len;
} WalletEngineBytesView;
```

For both views, `data` may be `NULL` only when `len == 0`. Input memory remains
owned by the C caller and is valid until the C function returns. A result view
will remain valid only for the duration of its result callback.

### Private UniFFI wire codecs

`RustBuffer` stays private to `wallet_engine.c`. The facade allocates and frees
it through the exact UniFFI runtime symbols discovered in the component
metadata; it never frees Rust-owned memory with the C allocator.

| Rust / UniFFI | Direct FFI value | Value nested in a `RustBuffer` |
|---|---|---|
| integers | same-width scalar | fixed-width big-endian bytes |
| `bool` | `int8_t`, `0` or `1` | one byte, `0` or non-zero |
| `String` | raw UTF-8 `RustBuffer` | big-endian `i32` byte length followed by UTF-8 |
| `Vec<u8>` / `Bytes` | length-prefixed `RustBuffer` | big-endian `i32` length followed by bytes |
| flat enum | `RustBuffer` | big-endian `i32` UniFFI discriminant |
| `Option<T>` | `RustBuffer` | one-byte `0`/`1` tag followed by nested `T` for `Some` |
| `Vec<T>` / `Sequence<T>` | `RustBuffer` | big-endian `i32` item count followed by each nested `T` |
| record | `RustBuffer` | fields serialized in their declared order without C padding |
| rich error | `RustBuffer` | big-endian one-based `i32` variant tag followed by the selected payload fields |

Lowering rejects malformed views, invalid UTF-8, lengths above `INT32_MAX`
where UniFFI uses a signed 32-bit length, and arithmetic overflow. Lifting
checks buffer bounds and rejects trailing data for a complete `Bytes` value.

The pure codec behavior is tested in `tests/codec.c`. That C11 executable
checks exact wire bytes and write/read round trips for every integer width,
booleans, strings, bytes, a flat enum, optional values, sequences of scalars,
strings, and flat enums, direct and nested records, and a rich error with both
fieldless and payload variants. The malformed cases cover truncated and
trailing data, invalid UTF-8, impossible lengths, and unknown tags.

### Flat enums

A Rust enum without payloads that is not used as an error gets its own public
C scalar type and named constants. For example:

```rust
pub enum Network {
    Mainnet,
    Testnet,
}
```

becomes:

```c
typedef uint32_t WalletEngineNetwork;
#define WALLET_ENGINE_NETWORK_MAINNET ((WalletEngineNetwork)0u)
#define WALLET_ENGINE_NETWORK_TESTNET ((WalletEngineNetwork)1u)
```

The public values are stable zero-based C ABI values and are recorded in
`wallet_engine.c-api.json`. They are deliberately separate from UniFFI's
private one-based wire tags. Generated codecs use an explicit `switch` in both
directions, so an unknown public value or wire tag is rejected instead of being
passed through accidentally.

The current Wallet Engine component contains 15 supported flat non-error
enums. Fielded non-error enums and flat declared errors remain separate planned
slices; rich declared errors are described below.

### Optional values

`Option<T>` keeps `None` distinct from every valid `T`, including zero and an
empty string:

```c
typedef struct WalletEngineOptionalU64 {
    bool has_value;
    uint64_t value;
} WalletEngineOptionalU64;
```

`value` is ignored when `has_value` is false. The public struct does not expose
the private UniFFI representation. Its direct FFI type is a `RustBuffer`
containing one tag byte: `0` for `None`, or `1` followed by the normal nested
codec for `T`. Generated readers reject every other tag and complete lifts
reject trailing bytes.

The implemented slice supports options whose inner value is an implemented
builtin or flat enum. In the current Wallet Engine component this produces six
types: `Option<String>`, `Option<u16>`, `Option<i32>`, `Option<u64>`,
`Option<PendingReason>`, and `Option<HttpHostErrorKind>`. Options over custom
types and records will be enabled with those type slices.

### Wallet Engine custom string types

These Rust types use `String` as their UniFFI builtin representation. They will
keep semantic C names while using the same `{ data, len }` ABI layout.

| Rust | Planned public C |
|---|---|
| `TonAddressString` | `WalletEngineTonAddressStringView` |
| `Base64Hash` | `WalletEngineBase64HashView` |
| `Boc` | `WalletEngineBocView` |
| `UnsignedDecimalString` | `WalletEngineUnsignedDecimalStringView` |
| `NonEmptyString` | `WalletEngineNonEmptyStringView` |

Memory and UTF-8 are checked at the C boundary. Address, hash, BOC, decimal,
and non-empty validation stays in the Rust custom-type lift implementation.

### Compound values

| Rust / UniFFI | Public C | Status | Rule |
|---|---|---|---|
| `Vec<T>` | `WalletEngineTListView { const T *data; size_t len; }` | Implemented for builtin and flat-enum items | Borrowed contiguous sequence. |
| record `T` | `WalletEngineTView` | Implemented when every field type is registered | Fields are converted recursively. |
| enum with fields | kind/tag plus generated payload union | Planned | Only the payload selected by the tag is active. |
| rich error enum `E` | stable tag plus generated payload union | Implemented when every payload field type is registered | Declared errors are separate from immediate ABI failures. |

For example, `Vec<String>` becomes `WalletEngineStringListView` containing
`const WalletEngineStringView *data` and `size_t len`. UniFFI sequence results
cannot point directly at their wire buffer: integers are big-endian and each
string carries a length prefix. The private decoder therefore materializes the
item array in a temporary arena. The public list and all nested views are valid
only during the result callback. Sequence lift already takes the private arena
and provides rollback/cleanup helpers; the later method renderer will clear it
after the callback returns. Rust-owned `RustBuffer` memory is still released
only through UniFFI.

The current slice composes sequences over registered builtins and flat enums.
Sequences over records and custom types become available when those item types
are registered by their later type slices.

A supported record is emitted as a field-for-field borrowed `*View`. Its wire
codec does not copy the in-memory C struct: it writes and reads every field in
the order stored in `ComponentInterface`, using the nested codec registered for
that field. Records are collected in dependency order, so a record can embed an
already supported record, option, or sequence by value. Lift uses the same
temporary arena as sequences and rolls allocations back if any later field is
malformed. A fieldless Rust record gets one public `uint8_t reserved` member
because ISO C does not allow an empty struct, while its UniFFI wire value stays
zero bytes.

The real Wallet Engine metadata currently produces 13 record views:
`CreateWalletRequest`, `DomainError`, `HttpHeader`, `HttpRequestId`,
`ImportWalletRequest`, `JournalKey`, `JournalRecord`, `ProtectedSecretRef`,
`ProtectedSecretStore`, `ProviderConfig`, `RecoveryPhrase`,
`JournalCompareExchange`, and `ProtectedSecretRead`. Remaining records depend
on custom types, payload enums, `Option<Record>`, or `Sequence<Record>` and are
enabled by those later type slices.

### Declared rich errors

An error gets into the C model only when it is a local, non-remote enum that
UniFFI reports as a declared `throws` type. `thiserror::Error` alone is not
enough: the Rust type must also be exported with `uniffi::Error`. Errors from
dependency crates that are not part of the Wallet Engine `ComponentInterface`
are never generated.

The public C representation uses a stable zero-based tag and a named payload
union. For example:

```c
typedef uint32_t WalletEngineHttpHostErrorTag;
#define WALLET_ENGINE_HTTP_HOST_ERROR_FAILED ((WalletEngineHttpHostErrorTag)0u)

typedef struct WalletEngineHttpHostErrorFailedPayload {
    WalletEngineHttpHostErrorKind kind;
    WalletEngineStringView diagnostic;
} WalletEngineHttpHostErrorFailedPayload;

typedef union WalletEngineHttpHostErrorPayload {
    WalletEngineHttpHostErrorFailedPayload failed;
} WalletEngineHttpHostErrorPayload;

typedef struct WalletEngineHttpHostError {
    WalletEngineHttpHostErrorTag tag;
    WalletEngineHttpHostErrorPayload payload;
} WalletEngineHttpHostError;
```

The private codec explicitly maps that tag to UniFFI's one-based `i32` wire
tag and serializes only the selected payload. Unknown public and wire tags,
malformed nested fields, truncation, trailing bytes, and size overflow are
rejected. Payload views lifted from Rust remain valid only while the owning
`RustBuffer` is alive; the later callable wrapper will keep it alive through
the result callback.

The real metadata currently produces four rich error types:
`HttpHostError`, `JournalHostError`, `ProtectedSecretHostError`, and
`WalletLifecycleError`. `WalletClientError` is deliberately not generated
partially: several variants contain `UnsignedDecimalString`, so the complete
error becomes available after the custom-type slice. The fielded non-error
`SendAmount` is also still pending.

### Objects and callables

| Rust / UniFFI | Planned public C |
|---|---|
| `Arc<WalletClient>` | opaque `WalletEngineWalletClient *` with retain/release |
| synchronous `Result<T, E>` | immediate ABI status plus typed inline result callback |
| `async fn method(...) -> Result<T, E>` | `wallet_engine_*_start`, typed result callback, and cancellable operation handle |
| `#[uniffi::export(foreign)]` trait | versioned C callback table plus one-shot completion handles |

Async Rust methods remain asynchronous. The C facade will drive UniFFI
`poll`/`complete`/`free`; it will not use `block_on` or turn an async operation
into a blocking C call.
