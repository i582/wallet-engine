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
- compiles the generated facade as strict C11 in its test suite.

Compound codecs, callback adapters, and export lists will be added on top of
the normalized component model. The crate intentionally has no dependency on
the `wallet-engine` or `c-bindings` crates.

From the repository root, the experimental recipe builds the library and runs
the generator without touching the production `bindings/c` output:

```shell
just bindings-c-experimental
```

The output directory is always `bindings/c-experimental`.

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

The current type slice discovers the builtins, flat non-error enums, and
supported optional values used by the real `ComponentInterface`, records their
public Rust-to-C mapping in the manifest, and generates borrowed views plus
private wire helpers. Tests compile the facade with strict C11 warnings and
execute its codecs against known UniFFI wire values.

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

Lowering rejects malformed views, invalid UTF-8, lengths above `INT32_MAX`
where UniFFI uses a signed 32-bit length, and arithmetic overflow. Lifting
checks buffer bounds and rejects trailing data for a complete `Bytes` value.

The pure codec behavior is tested in `tests/codec.c`. That C11 executable
checks exact wire bytes and write/read round trips for every integer width,
booleans, strings, bytes, a flat enum, and optional values, together with
malformed, truncated, trailing, and unknown-tag inputs.

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
enums. Enums with payloads and enums used as declared errors remain planned
separate slices.

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

| Rust / UniFFI | Planned public C | Rule |
|---|---|---|
| `Vec<T>` | `WalletEngineTListView { const T *data; size_t len; }` | Borrowed contiguous sequence. |
| record `T` | `WalletEngineTView` | Fields are converted recursively. |
| enum with fields | kind/tag plus generated payload union | Only the payload selected by the tag is active. |
| error enum `E` | stable error code plus generated payload | Declared errors are separate from immediate ABI failures. |

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

## Implementation order

The generated header and facade stay compilable after every slice:

1. Builtin integer, boolean, string, and byte representations — implemented.
2. Primitive/bool wire codecs and string/bytes lower/lift — implemented.
3. Flat non-error enums — implemented.
4. Options over implemented builtins and flat enums — implemented.
5. Sequences.
6. Records and nested combinations.
7. Error and payload enums.
8. Object handles and synchronous methods.
9. Async methods and operation runtime.
10. Foreign callback interfaces.
11. Packaging and export hygiene.
12. C++ compatibility, only after the C ABI is complete and stable.
