all: check

build:
    cargo build --release --locked

build-dev:
    cargo build --locked

build-release-size:
    cargo build --profile release-size --locked

fmt:
    cargo fmt --all
    cargo fmt --manifest-path apple-bindgen/Cargo.toml
    cargo fmt --manifest-path kotlin-bindgen/Cargo.toml
    cargo fmt --manifest-path xtask/Cargo.toml

fmt-check:
    cargo fmt --all --check
    cargo fmt --manifest-path apple-bindgen/Cargo.toml -- --check
    cargo fmt --manifest-path kotlin-bindgen/Cargo.toml -- --check
    cargo fmt --manifest-path xtask/Cargo.toml -- --check

check-build:
    cargo check --locked --all-targets
    cargo check --locked --manifest-path apple-bindgen/Cargo.toml --all-targets
    cargo check --locked --manifest-path kotlin-bindgen/Cargo.toml --all-targets
    cargo check --locked --manifest-path xtask/Cargo.toml --all-targets

clippy:
    cargo clippy --locked --all-targets -- -D warnings
    cargo clippy --locked --manifest-path apple-bindgen/Cargo.toml --all-targets -- -D warnings
    cargo clippy --locked --manifest-path kotlin-bindgen/Cargo.toml --all-targets -- -D warnings
    cargo clippy --locked --manifest-path xtask/Cargo.toml --all-targets -- -D warnings

test:
    cargo test --locked
    cargo test --locked --manifest-path xtask/Cargo.toml

check-wasm:
    cargo check --locked --target wasm32-unknown-unknown

check-ios-simulator:
    cargo check --locked --target aarch64-apple-ios-sim

bindings-swift:
    cargo xtask bindings swift

bindings-swift-check:
    cargo xtask bindings swift --check

bindings-kotlin:
    cargo xtask bindings kotlin

bindings-kotlin-check:
    cargo xtask bindings kotlin --check

build-android abi="all":
    cargo xtask android --abi {{abi}}

check-deny:
    cargo deny check

check-deps:
    cargo shear --deny-warnings

ci: fmt-check check-build clippy test check-wasm check-ios-simulator bindings-swift-check bindings-kotlin-check

check: ci check-deny

install-tools:
    cargo install cargo-shear --version 1.13.1 --locked
    cargo install cargo-deny --version 0.19.8 --locked

clean:
    cargo clean
    cargo clean --manifest-path apple-bindgen/Cargo.toml
    cargo clean --manifest-path kotlin-bindgen/Cargo.toml
    cargo clean --manifest-path xtask/Cargo.toml
