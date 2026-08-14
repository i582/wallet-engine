#!/bin/sh
set -eu

repository_root="$(cd "${SRCROOT}/../.." && pwd)"
target_root="${DERIVED_FILE_DIR}/wallet-engine-rust"
output_root="${DERIVED_FILE_DIR}/wallet-engine-universal"
mkdir -p "${output_root}"

libraries=""
for architecture in ${ARCHS}; do
    case "${PLATFORM_NAME}:${architecture}" in
        macosx:arm64) rust_target="aarch64-apple-darwin" ;;
        macosx:x86_64) rust_target="x86_64-apple-darwin" ;;
        iphonesimulator:arm64) rust_target="aarch64-apple-ios-sim" ;;
        iphonesimulator:x86_64) rust_target="x86_64-apple-ios" ;;
        iphoneos:arm64) rust_target="aarch64-apple-ios" ;;
        *)
            echo "Unsupported Apple target: ${PLATFORM_NAME}:${architecture}" >&2
            exit 1
            ;;
    esac

    CARGO_TARGET_DIR="${target_root}" \
        cargo build \
        --manifest-path "${repository_root}/Cargo.toml" \
        --release \
        --locked \
        --target "${rust_target}"
    library="${target_root}/${rust_target}/release/libwallet_engine.a"
    libraries="${libraries} ${library}"
done

# lipo also accepts a single input. This keeps the output path identical for
# single-architecture development and multi-architecture archive builds.
lipo -create ${libraries} -output "${output_root}/libwallet_engine.a"
