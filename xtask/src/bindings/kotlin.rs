use std::fs;

use anyhow::{Context, Result, bail};

use crate::files::{copy_generated, normalize_file, require_file};
use crate::paths::repository_root;
use crate::process::{bindgen_target_dir, build_engine_cdylib, cargo_command, run_command};

pub(crate) fn generate_kotlin(check: bool) -> Result<()> {
    let root = repository_root()?;
    let output =
        root.join("bindings/kotlin/src/main/kotlin/org/ton/wallet/engine/wallet_engine.kt");
    let temporary = tempfile::Builder::new()
        .prefix("wallet-engine-kotlin-bindings-")
        .tempdir()
        .context("failed to create temporary Kotlin bindings directory")?;
    let generated = temporary.path().join("generated");
    fs::create_dir_all(&generated).context("failed to create Kotlin generation directory")?;

    let engine_library = build_engine_cdylib(&root)?;
    run_command(
        cargo_command(&root, &bindgen_target_dir(&root))
            .arg("run")
            .arg("--manifest-path")
            .arg(root.join("kotlin-bindgen/Cargo.toml"))
            .arg("--locked")
            .arg("--")
            .arg("generate")
            .arg("--library")
            .arg("--language")
            .arg("kotlin")
            .arg("--no-format")
            .arg("--out-dir")
            .arg(&generated)
            .arg(&engine_library),
    )?;

    let kotlin = generated.join("org/ton/wallet/engine/wallet_engine.kt");
    require_file(&kotlin)?;
    normalize_file(&kotlin)?;
    validate_kotlin(&fs::read_to_string(&kotlin).context("failed to read generated Kotlin")?)?;

    if !check {
        copy_generated(&kotlin, &output)?;
    }
    Ok(())
}

fn validate_kotlin(source: &str) -> Result<()> {
    if source.contains("\npackage org.ton.wallet.engine\n")
        && source.contains("\npublic interface WalletHttpHost {")
        && source.contains("\npublic interface WalletPlatformHost {")
    {
        Ok(())
    } else {
        bail!("generated Kotlin binding is missing the expected public API")
    }
}
