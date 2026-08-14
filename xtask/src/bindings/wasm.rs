use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::files::require_file;
use crate::paths::repository_root;
use crate::process::run_command;

pub(crate) fn generate_wasm(check: bool) -> Result<()> {
    let root = repository_root()?;
    let output = root.join("bindings/wasm");
    let temporary = tempfile::Builder::new()
        .prefix("wallet-engine-wasm-bindings-")
        .tempdir()
        .context("failed to create temporary WASM bindings directory")?;
    let generated = temporary.path().join("generated");

    let mut command = Command::new("wasm-pack");
    command
        .current_dir(&root)
        .env("CARGO_TARGET_DIR", root.join("target/wasm-bindings"))
        .arg("build")
        .arg(root.join("wasm-bindings"))
        .arg("--target")
        .arg("web")
        .arg("--release")
        .arg("--out-dir")
        .arg(&generated)
        .arg("--out-name")
        .arg("wallet_engine");
    run_command(&mut command)?;

    validate_wasm(&generated)?;
    if !check {
        replace_directory(&generated, &output)?;
    }
    Ok(())
}

fn validate_wasm(generated: &Path) -> Result<()> {
    for filename in [
        "package.json",
        "wallet_engine.js",
        "wallet_engine.d.ts",
        "wallet_engine_bg.wasm",
        "wallet_engine_bg.wasm.d.ts",
    ] {
        require_file(&generated.join(filename))?;
    }

    let declarations = fs::read_to_string(generated.join("wallet_engine.d.ts"))
        .context("failed to read generated WASM declarations")?;
    if declarations.contains("export class WalletClient")
        && declarations.contains("export class WalletLifecycle")
        && declarations.contains("export interface WalletHttpHost")
        && declarations.contains("export interface WalletPlatformHost")
    {
        Ok(())
    } else {
        bail!("generated WASM package is missing the expected public API")
    }
}

fn replace_directory(source: &Path, destination: &Path) -> Result<()> {
    if destination.exists() {
        fs::remove_dir_all(destination).with_context(|| {
            format!("failed to remove stale directory {}", destination.display())
        })?;
    }
    copy_directory(source, destination)
}

fn copy_directory(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)
        .with_context(|| format!("failed to create {}", destination.display()))?;
    for entry in
        fs::read_dir(source).with_context(|| format!("failed to read {}", source.display()))?
    {
        let entry = entry.context("failed to read generated WASM directory entry")?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if entry
            .file_type()
            .context("failed to inspect generated WASM entry")?
            .is_dir()
        {
            copy_directory(&source_path, &destination_path)?;
        } else {
            fs::copy(&source_path, &destination_path).with_context(|| {
                format!(
                    "failed to copy {} to {}",
                    source_path.display(),
                    destination_path.display()
                )
            })?;
        }
    }
    Ok(())
}
