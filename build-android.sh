#!/bin/sh

set -eu

requested_abi=${1:-all}
engine_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
sdk_root=${ANDROID_SDK_ROOT:-${ANDROID_HOME:-$HOME/Library/Android/sdk}}
ndk_root=$(find "$sdk_root/ndk" -mindepth 1 -maxdepth 1 -type d 2>/dev/null | sort -V | tail -1)

if [ -z "$ndk_root" ]; then
    echo "error: Android NDK is required" >&2
    exit 1
fi

toolchain="$ndk_root/toolchains/llvm/prebuilt/darwin-x86_64/bin"
if [ ! -d "$toolchain" ]; then
    toolchain="$ndk_root/toolchains/llvm/prebuilt/darwin-aarch64/bin"
fi

build_abi() {
    abi=$1
    case "$abi" in
        arm64-v8a)
            rust_target=aarch64-linux-android
            clang_prefix=aarch64-linux-android
            ;;
        x86_64)
            rust_target=x86_64-linux-android
            clang_prefix=x86_64-linux-android
            ;;
        *)
            echo "error: unsupported Android ABI: $abi" >&2
            exit 1
            ;;
    esac

    linker="$toolchain/${clang_prefix}28-clang"
    if [ ! -x "$linker" ]; then
        echo "error: Android linker was not found: $linker" >&2
        exit 1
    fi

    target_key=$(printf '%s' "$rust_target" | tr '[:lower:]-' '[:upper:]_')
    target_dir="$engine_dir/target/android"
    output_dir="$target_dir/jniLibs/$abi"
    mkdir -p "$output_dir"

    env \
        "CARGO_TARGET_${target_key}_LINKER=$linker" \
        CARGO_TARGET_DIR="$target_dir" \
        cargo build \
        --manifest-path "$engine_dir/Cargo.toml" \
        --target "$rust_target" \
        --release \
        --locked

    cp "$target_dir/$rust_target/release/libwallet_engine.so" \
        "$output_dir/libwallet_engine.so"
}

case "$requested_abi" in
    all)
        build_abi arm64-v8a
        build_abi x86_64
        ;;
    arm64-v8a|x86_64)
        build_abi "$requested_abi"
        ;;
    *)
        echo "usage: build-android.sh [all|arm64-v8a|x86_64]" >&2
        exit 1
        ;;
esac
