#!/bin/sh

set -eu

mode=${1:-generate}
case "$mode" in
    generate|--check) ;;
    *) echo "usage: generate-kotlin-bindings.sh [--check]" >&2; exit 1 ;;
esac

engine_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
output_dir="$engine_dir/bindings/kotlin/src/main/kotlin"
package_dir="$output_dir/org/ton/wallet/engine"
temporary_dir=$(mktemp -d "${TMPDIR:-/tmp}/wallet-engine-kotlin-bindings.XXXXXX")
trap 'rm -rf "$temporary_dir"' EXIT HUP INT TERM
engine_target_dir="$engine_dir/target/kotlin-bindings"
bindgen_target_dir="$engine_dir/kotlin-bindgen/target"

if command -v cargo >/dev/null 2>&1; then
    cargo_bin=$(command -v cargo)
elif [ -x "$HOME/.cargo/bin/cargo" ]; then
    cargo_bin="$HOME/.cargo/bin/cargo"
else
    echo "error: cargo was not found; install Rust with rustup" >&2
    exit 1
fi

env -u SDKROOT -u LIBRARY_PATH -u CPATH -u C_INCLUDE_PATH \
    -u CPLUS_INCLUDE_PATH -u CFLAGS -u CXXFLAGS -u CPPFLAGS -u LDFLAGS \
    CARGO_TARGET_DIR="$engine_target_dir" "$cargo_bin" build \
    --manifest-path "$engine_dir/Cargo.toml" \
    --release \
    --locked

mkdir -p "$temporary_dir/generated"
(
    cd "$engine_dir"
    CARGO_TARGET_DIR="$bindgen_target_dir" "$cargo_bin" run \
        --manifest-path "$engine_dir/kotlin-bindgen/Cargo.toml" \
        --release \
        --locked \
        -- \
        generate \
        --library \
        --language kotlin \
        --no-format \
        --out-dir "$temporary_dir/generated" \
        "$engine_target_dir/release/libwallet_engine.dylib"
)

generated_kotlin="$temporary_dir/generated/org/ton/wallet/engine/wallet_engine.kt"
generated_output="$package_dir/wallet_engine.kt"

if [ ! -f "$generated_kotlin" ]; then
    echo "error: UniFFI did not generate expected file: $generated_kotlin" >&2
    exit 1
fi

sed -E 's/[[:space:]]+$//' "$generated_kotlin" > "$generated_kotlin.normalized"
mv "$generated_kotlin.normalized" "$generated_kotlin"

if ! grep -q '^package org\.ton\.wallet\.engine$' "$generated_kotlin" || \
   ! grep -q '^public interface WalletHttpHost {' "$generated_kotlin" || \
   ! grep -q '^public interface WalletPlatformHost {' "$generated_kotlin"; then
    echo "error: generated Kotlin binding is missing the expected public API" >&2
    exit 1
fi

if [ "$mode" = "--check" ]; then
    exit 0
fi

mkdir -p "$package_dir"
cp "$generated_kotlin" "$generated_output"
