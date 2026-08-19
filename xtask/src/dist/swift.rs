use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use clap::Args;

use crate::bindings::generate_swift;
use crate::dist::{
    copy_file, copy_package_metadata, prepare_output_directory, require_file,
    strip_release_library, write_checksum,
};
use crate::process::{cargo_command, run_command};
use crate::version::project_version;

const APPLE_TARGETS: &[&str] = &[
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "aarch64-apple-ios",
    "aarch64-apple-ios-sim",
    "x86_64-apple-ios",
];

/// Selects the output directory for the Swift release package.
#[derive(Args)]
pub(crate) struct SwiftArgs {
    /// Directory that receives the Swift package and checksum.
    #[arg(long)]
    output: Option<PathBuf>,
}

/// Builds a static `XCFramework` and a locally consumable Swift package.
pub(crate) fn run(root: &Path, args: &SwiftArgs) -> Result<()> {
    if env::consts::OS != "macos" {
        bail!("Swift release packages must be built on macOS")
    }

    let version = project_version(root)?;
    let output = prepare_output_directory(root, args.output.as_deref())?;
    let build_root = root.join("target/release-swift");
    for target in APPLE_TARGETS {
        build_apple_target(root, &build_root, target)?;
    }
    generate_swift(false)?;

    let temporary = tempfile::Builder::new()
        .prefix("wallet-engine-swift-release-")
        .tempdir()?;
    let products = temporary.path().join("products");
    fs::create_dir_all(&products)?;
    let macos = products.join("libwallet_engine_macos.a");
    let simulator = products.join("libwallet_engine_ios_simulator.a");
    create_universal_library(
        &macos,
        &[
            static_library(&build_root, "aarch64-apple-darwin"),
            static_library(&build_root, "x86_64-apple-darwin"),
        ],
    )?;
    create_universal_library(
        &simulator,
        &[
            static_library(&build_root, "aarch64-apple-ios-sim"),
            static_library(&build_root, "x86_64-apple-ios"),
        ],
    )?;
    let ios = static_library(&build_root, "aarch64-apple-ios");
    require_file(&ios)?;

    let headers = temporary.path().join("headers");
    fs::create_dir_all(&headers)?;
    let generated = root.join("bindings/swift/Sources/wallet_engineFFI");
    copy_file(
        &generated.join("wallet_engineFFI.h"),
        &headers.join("wallet_engineFFI.h"),
    )?;
    copy_file(
        &generated.join("module.modulemap"),
        &headers.join("module.modulemap"),
    )?;

    let package_name = format!("wallet-engine-swift-{version}");
    let package = temporary.path().join(&package_name);
    let xcframework = package.join("wallet_engineFFI.xcframework");
    fs::create_dir_all(package.join("Sources/WalletEngineFFI"))?;
    create_xcframework(&macos, &ios, &simulator, &headers, &xcframework)?;
    copy_file(
        &root.join("bindings/swift/Sources/WalletEngineFFI/WalletEngineFFI.swift"),
        &package.join("Sources/WalletEngineFFI/WalletEngineFFI.swift"),
    )?;
    fs::write(package.join("Package.swift"), SWIFT_PACKAGE)?;
    copy_package_metadata(root, &package)?;
    validate_swift_package(&package)?;

    let archive = output.join(format!("{package_name}.zip"));
    if archive.exists() {
        fs::remove_file(&archive)?;
    }
    run_command(
        Command::new("ditto")
            .arg("-c")
            .arg("-k")
            .arg("--norsrc")
            .arg("--noextattr")
            .arg("--noqtn")
            .arg("--noacl")
            .arg("--keepParent")
            .arg(&package)
            .arg(&archive),
    )?;
    let _checksum = write_checksum(&archive)?;
    println!("{}", archive.display());
    Ok(())
}

/// Compiles the release static library for one Apple Rust target.
fn build_apple_target(root: &Path, build_root: &Path, target: &str) -> Result<()> {
    let mut command = cargo_command(root, build_root);
    command
        .arg("build")
        .arg("--release")
        .arg("--locked")
        .arg("--target")
        .arg(target);
    if target.ends_with("apple-darwin") {
        command.env("MACOSX_DEPLOYMENT_TARGET", "15.0");
    } else {
        command.env("IPHONEOS_DEPLOYMENT_TARGET", "18.0");
    }
    run_command(&mut command)?;
    strip_release_library(&static_library(build_root, target))
}

/// Returns the static library emitted for one Apple target.
fn static_library(build_root: &Path, target: &str) -> PathBuf {
    build_root.join(target).join("release/libwallet_engine.a")
}

/// Combines compatible Apple architecture slices into one static library.
fn create_universal_library(output: &Path, inputs: &[PathBuf]) -> Result<()> {
    for input in inputs {
        require_file(input)?;
    }
    let mut command = Command::new("lipo");
    command.arg("-create");
    command.args(inputs);
    command.arg("-output").arg(output);
    run_command(&mut command)
}

/// Creates the binary target consumed by the generated Swift wrapper.
fn create_xcframework(
    macos: &Path,
    ios: &Path,
    simulator: &Path,
    headers: &Path,
    output: &Path,
) -> Result<()> {
    run_command(
        Command::new("xcodebuild")
            .arg("-create-xcframework")
            .arg("-library")
            .arg(macos)
            .arg("-headers")
            .arg(headers)
            .arg("-library")
            .arg(ios)
            .arg("-headers")
            .arg(headers)
            .arg("-library")
            .arg(simulator)
            .arg("-headers")
            .arg(headers)
            .arg("-output")
            .arg(output),
    )
}

/// Parses and builds the staged package against its macOS `XCFramework` slice.
fn validate_swift_package(package: &Path) -> Result<()> {
    let validation = tempfile::Builder::new()
        .prefix("wallet-engine-swift-validation-")
        .tempdir()?;
    let module_cache = validation.path().join("module-cache");
    let cache = validation.path().join("cache");
    let configuration = validation.path().join("configuration");
    let security = validation.path().join("security");
    let scratch = validation.path().join("scratch");
    for directory in [&module_cache, &cache, &configuration, &security, &scratch] {
        fs::create_dir_all(directory)?;
    }
    let mut dump = Command::new("swift");
    dump.env("CLANG_MODULE_CACHE_PATH", &module_cache)
        .env("SWIFTPM_MODULECACHE_OVERRIDE", &module_cache)
        .arg("package")
        .arg("--disable-sandbox")
        .arg("--package-path")
        .arg(package)
        .arg("--cache-path")
        .arg(&cache)
        .arg("--config-path")
        .arg(&configuration)
        .arg("--security-path")
        .arg(&security)
        .arg("--scratch-path")
        .arg(&scratch)
        .arg("dump-package");
    run_command(&mut dump)?;

    let mut build = Command::new("swift");
    build
        .env("CLANG_MODULE_CACHE_PATH", &module_cache)
        .env("SWIFTPM_MODULECACHE_OVERRIDE", &module_cache)
        .arg("build")
        .arg("--disable-sandbox")
        .arg("--package-path")
        .arg(package)
        .arg("--cache-path")
        .arg(&cache)
        .arg("--config-path")
        .arg(&configuration)
        .arg("--security-path")
        .arg(&security)
        .arg("--scratch-path")
        .arg(&scratch)
        .arg("--configuration")
        .arg("release");
    run_command(&mut build).with_context(|| {
        format!(
            "failed to build staged Swift package at {}",
            package.display()
        )
    })
}

const SWIFT_PACKAGE: &str = r#"// swift-tools-version: 6.2
import PackageDescription

let package = Package(
    name: "WalletEngineFFI",
    platforms: [
        .iOS(.v18),
        .macOS(.v15),
    ],
    products: [
        .library(name: "WalletEngineFFI", targets: ["WalletEngineFFI"]),
    ],
    targets: [
        .binaryTarget(
            name: "wallet_engineFFI",
            path: "wallet_engineFFI.xcframework"
        ),
        .target(
            name: "WalletEngineFFI",
            dependencies: ["wallet_engineFFI"],
            path: "Sources/WalletEngineFFI",
            swiftSettings: [.defaultIsolation(nil)]
        ),
    ]
)
"#;

#[cfg(test)]
mod tests {
    use super::{APPLE_TARGETS, SWIFT_PACKAGE};

    #[test]
    fn swift_release_contains_all_supported_slices() {
        assert_eq!(APPLE_TARGETS.len(), 5);
        assert!(APPLE_TARGETS.contains(&"aarch64-apple-ios"));
        assert!(APPLE_TARGETS.contains(&"x86_64-apple-ios"));
    }

    #[test]
    fn swift_manifest_uses_the_local_xcframework() {
        assert!(SWIFT_PACKAGE.contains(".binaryTarget("));
        assert!(SWIFT_PACKAGE.contains("wallet_engineFFI.xcframework"));
    }
}
