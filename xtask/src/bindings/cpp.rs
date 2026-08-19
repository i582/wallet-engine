use std::fs;

use anyhow::{Context, Result};

use crate::paths::repository_root;
use crate::process::{bindgen_target_dir, build_engine_cdylib, cargo_command, run_command};

pub(crate) fn generate_cpp() -> Result<()> {
    let root = repository_root()?;
    let output = root.join("bindings/cpp-experimental");
    fs::create_dir_all(&output).context("failed to create C++ generation directory")?;

    let engine_library = build_engine_cdylib(&root)?;
    run_command(
        cargo_command(&root, &bindgen_target_dir(&root))
            .arg("run")
            .arg("--manifest-path")
            .arg(root.join("bindgen/cpp/bindgen/Cargo.toml"))
            .arg("--locked")
            .arg("--")
            .arg("--library")
            .arg("--out-dir")
            .arg(&output)
            .arg(&engine_library),
    )
}
