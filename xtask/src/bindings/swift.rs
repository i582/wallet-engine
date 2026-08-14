use std::fs;

use anyhow::{Context, Result, bail};

use crate::bindings::swift_postprocess::postprocess_swift;
use crate::files::{copy_generated, normalize_file, normalize_text, require_file};
use crate::paths::repository_root;
use crate::process::{build_engine_cdylib, cargo_command, run_command};

pub(crate) fn generate_swift(check: bool) -> Result<()> {
    let root = repository_root()?;
    let output_root = root.join("bindings/swift");
    let swift_output = output_root.join("Sources/WalletEngineFFI/WalletEngineFFI.swift");
    let c_output_root = output_root.join("Sources/wallet_engineFFI");
    let header_output = c_output_root.join("wallet_engineFFI.h");
    let modulemap_output = c_output_root.join("module.modulemap");
    let package_output = output_root.join("Package.swift");
    let temporary = tempfile::Builder::new()
        .prefix("wallet-engine-swift-bindings-")
        .tempdir()
        .context("failed to create temporary Swift bindings directory")?;
    let generated = temporary.path().join("generated");
    fs::create_dir_all(&generated).context("failed to create Apple generation directory")?;

    let engine_library = build_engine_cdylib(&root, &root.join("target/swift-bindings"))?;
    run_command(
        cargo_command(&root, &root.join("apple-bindgen/target"))
            .arg("run")
            .arg("--manifest-path")
            .arg(root.join("apple-bindgen/Cargo.toml"))
            .arg("--release")
            .arg("--locked")
            .arg("--")
            .arg("--swift-sources")
            .arg("--headers")
            .arg("--modulemap")
            .arg("--module-name")
            .arg("wallet_engineFFI")
            .arg("--modulemap-filename")
            .arg("module.modulemap")
            .arg(&engine_library)
            .arg(&generated),
    )?;

    let swift = generated.join("WalletEngineFFI.swift");
    let header = generated.join("wallet_engineFFI.h");
    let modulemap = generated.join("module.modulemap");
    require_file(&swift)?;
    require_file(&header)?;
    require_file(&modulemap)?;

    let swift_source = fs::read_to_string(&swift).context("failed to read generated Swift")?;
    fs::write(&swift, normalize_text(&postprocess_swift(&swift_source)?))
        .context("failed to write postprocessed Swift")?;
    normalize_file(&header)?;
    normalize_file(&modulemap)?;
    validate_swift(&fs::read_to_string(&swift).context("failed to read processed Swift")?)?;

    if !check {
        copy_generated(&swift, &swift_output)?;
        copy_generated(&header, &header_output)?;
        copy_generated(&modulemap, &modulemap_output)?;
        fs::create_dir_all(&output_root).context("failed to create Swift package directory")?;
        fs::write(&package_output, SWIFT_PACKAGE_MANIFEST)
            .context("failed to write generated Swift package manifest")?;
    }
    Ok(())
}

const SWIFT_PACKAGE_MANIFEST: &str = r#"// swift-tools-version: 6.2
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
        .systemLibrary(
            name: "wallet_engineFFI",
            path: "Sources/wallet_engineFFI"
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

fn validate_swift(source: &str) -> Result<()> {
    if source.contains("@preconcurrency import wallet_engineFFI\n")
        && source.contains("@Sendable () async throws ->")
        && source.contains("private func uniffiTraitInterfaceCallAsync<T: Sendable>(")
    {
        Ok(())
    } else {
        bail!("generated Swift binding is missing Swift 6 callback annotations")
    }
}
