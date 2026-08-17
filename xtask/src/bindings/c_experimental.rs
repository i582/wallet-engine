use anyhow::Result;

use crate::files::require_file;
use crate::paths::repository_root;
use crate::process::{bindgen_target_dir, build_engine_cdylib, cargo_command, run_command};

const HEADER_FILENAME: &str = "wallet_engine.h";
const FACADE_FILENAME: &str = "wallet_engine.c";
const MANIFEST_FILENAME: &str = "wallet_engine.c-api.json";

pub(crate) fn generate_c_experimental() -> Result<()> {
    let root = repository_root()?;
    let output = root.join("bindings/c-experimental");
    let engine_library = build_engine_cdylib(&root)?;

    run_command(
        cargo_command(&root, &bindgen_target_dir(&root))
            .arg("run")
            .arg("--manifest-path")
            .arg(root.join("c-bindgen/Cargo.toml"))
            .arg("--locked")
            .arg("--")
            .arg("--library")
            .arg(&engine_library)
            .arg("--out-dir")
            .arg(&output),
    )?;

    require_file(&output.join(HEADER_FILENAME))?;
    require_file(&output.join(FACADE_FILENAME))?;
    require_file(&output.join(MANIFEST_FILENAME))
}
