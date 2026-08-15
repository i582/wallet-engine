NEXTEST_PROFILE_ARGS := if env_var_or_default("CI", "") != "" { "-P ci" } else { "" }
NEXTEST_CONFIG_ARGS := "--config-file .config/nextest.toml"

all: check

build:
    cargo build --release --locked

build-dev:
    cargo build --locked

build-release-size:
    cargo build --profile release-size --locked

fmt:
    cargo fmt --all
    cargo fmt --manifest-path c-bindings/Cargo.toml
    cargo fmt --manifest-path apple-bindgen/Cargo.toml
    cargo fmt --manifest-path kotlin-bindgen/Cargo.toml
    cargo fmt --manifest-path wasm-bindings/Cargo.toml
    cargo fmt --manifest-path xtask/Cargo.toml

fmt-check:
    cargo fmt --all --check
    cargo fmt --manifest-path c-bindings/Cargo.toml -- --check
    cargo fmt --manifest-path apple-bindgen/Cargo.toml -- --check
    cargo fmt --manifest-path kotlin-bindgen/Cargo.toml -- --check
    cargo fmt --manifest-path wasm-bindings/Cargo.toml -- --check
    cargo fmt --manifest-path xtask/Cargo.toml -- --check

check-build:
    cargo check --locked --all-targets
    cargo check --locked --manifest-path c-bindings/Cargo.toml --all-targets
    cargo check --locked --manifest-path apple-bindgen/Cargo.toml --all-targets
    cargo check --locked --manifest-path kotlin-bindgen/Cargo.toml --all-targets
    cargo check --locked --manifest-path wasm-bindings/Cargo.toml --target wasm32-unknown-unknown
    cargo check --locked --manifest-path xtask/Cargo.toml --all-targets

clippy:
    cargo clippy --locked --all-targets -- -D warnings
    cargo clippy --locked --manifest-path c-bindings/Cargo.toml --all-targets -- -D warnings
    cargo clippy --locked --manifest-path apple-bindgen/Cargo.toml --all-targets -- -D warnings
    cargo clippy --locked --manifest-path kotlin-bindgen/Cargo.toml --all-targets -- -D warnings
    cargo clippy --locked --manifest-path wasm-bindings/Cargo.toml --target wasm32-unknown-unknown -- -D warnings
    cargo clippy --locked --manifest-path xtask/Cargo.toml --all-targets -- -D warnings

test-rust:
    cargo nextest run --locked {{ NEXTEST_PROFILE_ARGS }}
    cargo nextest run --locked --manifest-path xtask/Cargo.toml {{ NEXTEST_CONFIG_ARGS }} {{ NEXTEST_PROFILE_ARGS }}
    cargo test --locked --doc

test-c-abi-rust:
    cargo nextest run --locked --manifest-path c-bindings/Cargo.toml {{ NEXTEST_CONFIG_ARGS }} {{ NEXTEST_PROFILE_ARGS }}

test: test-c test-cpp test-rust test-c-abi-rust

coverage-setup:
    cargo install cargo-llvm-cov --locked
    rustup component add llvm-tools-preview

coverage:
    env WALLET_ENGINE_SCENARIO_TIMEOUT_SECS=300 cargo llvm-cov nextest --locked --all-features --all-targets --ignore-filename-regex '(^|/)vendor/|(^|/)src/engine/host\.rs$' --lcov --output-path lcov.info {{ NEXTEST_PROFILE_ARGS }}

coverage-html:
    env WALLET_ENGINE_SCENARIO_TIMEOUT_SECS=300 cargo llvm-cov nextest --locked --all-features --all-targets --ignore-filename-regex '(^|/)vendor/|(^|/)src/engine/host\.rs$' --html --output-dir coverage/html {{ NEXTEST_PROFILE_ARGS }}

coverage-clean:
    cargo llvm-cov clean

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

bindings-c:
    cargo xtask bindings c

bindings-c-check:
    cargo xtask bindings c --check

build-c:
    cargo build --release --locked --manifest-path c-bindings/Cargo.toml

test-c: bindings-c build-c
    cmake -S c-bindings/tests/c -B target/c-tests
    cmake --build target/c-tests
    ctest --test-dir target/c-tests --output-on-failure

build-cpp: bindings-c
    cmake -S cpp-bindings -B target/cpp-bindings -DBUILD_TESTING=OFF
    cmake --build target/cpp-bindings

test-cpp: bindings-c
    cmake -S cpp-bindings -B target/cpp-bindings -DBUILD_TESTING=ON
    cmake --build target/cpp-bindings
    ctest --test-dir target/cpp-bindings --output-on-failure

test-cpp-sanitized: bindings-c
    cmake -S cpp-bindings -B target/cpp-bindings-sanitized -DBUILD_TESTING=ON -DWALLET_ENGINE_CPP_ENABLE_SANITIZERS=ON
    cmake --build target/cpp-bindings-sanitized
    ctest --test-dir target/cpp-bindings-sanitized --output-on-failure

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

example-web-install: web-install
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

web-check: web-fmt-check web-lint web-build web-test example-web-fmt-check example-web-lint example-web-build example-web-test

example-c-build: bindings-c build-c
    cmake -S examples/c -B target/c-example
    cmake --build target/c-example

example-c-run: example-c-build
    ./target/c-example/wallet_engine_c_example

example-tui-run:
    cargo run --locked --manifest-path examples/tui/Cargo.toml

example-tui-fmt:
    cargo fmt --manifest-path examples/tui/Cargo.toml

example-tui-fmt-check:
    cargo fmt --manifest-path examples/tui/Cargo.toml -- --check

example-tui-check:
    cargo check --locked --manifest-path examples/tui/Cargo.toml --all-targets

example-tui-clippy:
    cargo clippy --locked --manifest-path examples/tui/Cargo.toml --all-targets -- -D warnings

example-tui-test:
    cargo nextest run --locked --manifest-path examples/tui/Cargo.toml {{ NEXTEST_CONFIG_ARGS }} {{ NEXTEST_PROFILE_ARGS }}

example-swift-build-macos: bindings-swift
    xcodebuild -project examples/swift/WalletEngineApp.xcodeproj -scheme WalletEngineApp -configuration Debug -destination 'platform=macOS,arch=arm64' -derivedDataPath target/swift-example CODE_SIGNING_ALLOWED=NO build

example-swift-build-ios: bindings-swift
    xcodebuild -project examples/swift/WalletEngineApp.xcodeproj -scheme WalletEngineApp -configuration Debug -destination 'generic/platform=iOS Simulator' -derivedDataPath target/swift-example-ios ARCHS=arm64 ONLY_ACTIVE_ARCH=YES build

example-swift-open: bindings-swift
    open examples/swift/WalletEngineApp.xcodeproj

build-android abi="all":
    cargo xtask android --abi {{abi}}

example-android-build: bindings-kotlin build-android
    examples/android/gradlew -p examples/android :app:assembleDebug --no-configuration-cache

example-android-check: bindings-kotlin build-android
    examples/android/gradlew -p examples/android :app:assembleDebug :app:testInstrumentedTestUnitTest :app:lintDebug --no-configuration-cache

example-android-install: example-android-build
    examples/android/gradlew -p examples/android :app:installDebug --no-configuration-cache

check-deny:
    cargo deny check

check-deps:
    cargo shear --deny-warnings

ci: fmt-check check-build clippy test check-wasm check-ios-simulator bindings-swift-check bindings-kotlin-check bindings-wasm-check web-fmt-check web-lint web-build web-test example-web-fmt-check example-web-lint example-web-build example-web-test example-tui-fmt-check example-tui-check example-tui-clippy example-tui-test

check: ci check-deny

install-tools:
    cargo install cargo-shear --version 1.13.1 --locked
    cargo install cargo-deny --version 0.19.8 --locked
    cargo install wasm-pack --version 0.15.0 --locked

clean:
    cargo clean
    cargo clean --manifest-path c-bindings/Cargo.toml
    cargo clean --manifest-path apple-bindgen/Cargo.toml
    cargo clean --manifest-path kotlin-bindgen/Cargo.toml
    cargo clean --manifest-path wasm-bindings/Cargo.toml
    cargo clean --manifest-path xtask/Cargo.toml
