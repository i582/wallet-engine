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
    cargo fmt --manifest-path wasm-bindings/Cargo.toml
    cargo fmt --manifest-path xtask/Cargo.toml

fmt-check:
    cargo fmt --all --check
    cargo fmt --manifest-path apple-bindgen/Cargo.toml -- --check
    cargo fmt --manifest-path kotlin-bindgen/Cargo.toml -- --check
    cargo fmt --manifest-path wasm-bindings/Cargo.toml -- --check
    cargo fmt --manifest-path xtask/Cargo.toml -- --check

check-build:
    cargo check --locked --all-targets
    cargo check --locked --manifest-path apple-bindgen/Cargo.toml --all-targets
    cargo check --locked --manifest-path kotlin-bindgen/Cargo.toml --all-targets
    cargo check --locked --manifest-path wasm-bindings/Cargo.toml --target wasm32-unknown-unknown
    cargo check --locked --manifest-path xtask/Cargo.toml --all-targets

clippy:
    cargo clippy --locked --all-targets -- -D warnings
    cargo clippy --locked --manifest-path apple-bindgen/Cargo.toml --all-targets -- -D warnings
    cargo clippy --locked --manifest-path kotlin-bindgen/Cargo.toml --all-targets -- -D warnings
    cargo clippy --locked --manifest-path wasm-bindings/Cargo.toml --target wasm32-unknown-unknown -- -D warnings
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

bindings-wasm:
    cargo xtask bindings wasm

bindings-wasm-check:
    cargo xtask bindings wasm --check

web-install:
    bun install --cwd web --frozen-lockfile

web-fmt: web-install
    bun --cwd web fmt

web-fmt-check: web-install
    bun --cwd web fmt:check

web-lint: web-install
    bun --cwd web lint

web-build: web-install bindings-wasm
    bun --cwd web build

web-test: web-install bindings-wasm
    bun --cwd web test

example-web-install:
    bun install --cwd examples/web --frozen-lockfile

example-web-dev: example-web-install bindings-wasm
    bun --cwd examples/web dev

example-web-fmt: example-web-install
    bun --cwd examples/web fmt

example-web-fmt-check: example-web-install
    bun --cwd examples/web fmt:check

example-web-lint: example-web-install
    bun --cwd examples/web lint

example-web-build: example-web-install bindings-wasm
    bun --cwd examples/web build

example-web-test: example-web-install bindings-wasm
    bun --cwd examples/web test

build-android abi="all":
    cargo xtask android --abi {{abi}}

check-deny:
    cargo deny check

check-deps:
    cargo shear --deny-warnings

ci: fmt-check check-build clippy test check-wasm check-ios-simulator bindings-swift-check bindings-kotlin-check bindings-wasm-check web-fmt-check web-lint web-build web-test example-web-fmt-check example-web-lint example-web-build example-web-test

check: ci check-deny

install-tools:
    cargo install cargo-shear --version 1.13.1 --locked
    cargo install cargo-deny --version 0.19.8 --locked
    cargo install wasm-pack --version 0.15.0 --locked

clean:
    cargo clean
    cargo clean --manifest-path apple-bindgen/Cargo.toml
    cargo clean --manifest-path kotlin-bindgen/Cargo.toml
    cargo clean --manifest-path wasm-bindings/Cargo.toml
    cargo clean --manifest-path xtask/Cargo.toml
