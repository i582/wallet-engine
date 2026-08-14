#!/bin/sh

set -eu

mode=${1:-generate}
case "$mode" in
    generate|--check) ;;
    *) echo "usage: generate-apple-bindings.sh [--check]" >&2; exit 1 ;;
esac

engine_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_dir=$(CDPATH= cd -- "$engine_dir/.." && pwd)
output_dir="$repo_dir/WalletEngineFFI"
swift_output_dir="$output_dir/Sources/WalletEngineFFI"
c_output_dir="$output_dir/Sources/wallet_engineFFI"
temporary_dir=$(mktemp -d "${TMPDIR:-/tmp}/wallet-engine-bindings.XXXXXX")
trap 'rm -rf "$temporary_dir"' EXIT HUP INT TERM
engine_target_dir="$engine_dir/target/apple-bindings"
bindgen_target_dir="$engine_dir/apple-bindgen/target"

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
        --manifest-path "$engine_dir/apple-bindgen/Cargo.toml" \
        --release \
        --locked \
        -- \
        --swift-sources \
        --headers \
        --modulemap \
        --module-name wallet_engineFFI \
        --modulemap-filename module.modulemap \
        "$engine_target_dir/release/libwallet_engine.dylib" \
        "$temporary_dir/generated"
)

generated_swift="$temporary_dir/generated/WalletEngineFFI.swift"
generated_header="$temporary_dir/generated/wallet_engineFFI.h"
generated_modulemap="$temporary_dir/generated/module.modulemap"

for generated_file in "$generated_swift" "$generated_header" "$generated_modulemap"; do
    if [ ! -f "$generated_file" ]; then
        echo "error: UniFFI did not generate expected file: $generated_file" >&2
        exit 1
    fi
done

awk -f "$engine_dir/postprocess-apple-bindings.awk" "$generated_swift" \
    > "$generated_swift.swift6"
mv "$generated_swift.swift6" "$generated_swift"

# UniFFI templates can contain trailing horizontal whitespace. Normalize all
# committed artifacts before generation and stale-binding comparison.
for generated_file in "$generated_swift" "$generated_header" "$generated_modulemap"; do
    sed -E 's/[[:space:]]+$//' "$generated_file" > "$generated_file.normalized"
    mv "$generated_file.normalized" "$generated_file"
done

if [ "$mode" = "--check" ]; then
    check_file() {
        generated_path=$1
        committed_path=$2
        if [ ! -f "$committed_path" ]; then
            echo "error: committed binding is missing: $committed_path" >&2
            exit 1
        fi
        if ! cmp -s "$generated_path" "$committed_path"; then
            echo "error: generated Apple binding is stale: $committed_path" >&2
            diff -u "$committed_path" "$generated_path" || true
            exit 1
        fi
    }

    check_file "$generated_swift" "$swift_output_dir/WalletEngineFFI.swift"
    check_file "$generated_header" "$c_output_dir/wallet_engineFFI.h"
    check_file "$generated_modulemap" "$c_output_dir/module.modulemap"
    if ! grep -q '^@preconcurrency import wallet_engineFFI$' "$generated_swift" || \
       ! grep -q '@Sendable () async throws ->' "$generated_swift" || \
       ! grep -q 'private func uniffiTraitInterfaceCallAsync<T: Sendable>(' "$generated_swift"; then
        echo "error: generated Apple binding is missing Swift 6 callback annotations" >&2
        exit 1
    fi
    exit 0
fi

mkdir -p "$swift_output_dir" "$c_output_dir"
cp "$generated_swift" "$swift_output_dir/WalletEngineFFI.swift"
cp "$generated_header" "$c_output_dir/wallet_engineFFI.h"
cp "$generated_modulemap" "$c_output_dir/module.modulemap"
