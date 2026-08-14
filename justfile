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

fmt-check:
    cargo fmt --all --check
    cargo fmt --manifest-path apple-bindgen/Cargo.toml -- --check

check-build:
    cargo check --locked --all-targets
    cargo check --locked --manifest-path apple-bindgen/Cargo.toml --all-targets

clippy:
    cargo clippy --locked --all-targets -- -D warnings
    cargo clippy --locked --manifest-path apple-bindgen/Cargo.toml --all-targets -- -D warnings

test:
    cargo test --locked

check-wasm:
    cargo check --locked --target wasm32-unknown-unknown

check-ios-simulator:
    cargo check --locked --target aarch64-apple-ios-sim

bindings:
    ./generate-apple-bindings.sh

bindings-check:
    ./generate-apple-bindings.sh --check

check-deny:
    cargo deny check

check-deps:
    cargo shear --deny-warnings

ci: fmt-check check-build clippy test check-wasm check-ios-simulator

check: ci check-deny

install-tools:
    cargo install cargo-shear --version 1.13.1 --locked
    cargo install cargo-deny --version 0.19.8 --locked

clean:
    cargo clean
    cargo clean --manifest-path apple-bindgen/Cargo.toml
