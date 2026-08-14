use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};

pub(crate) fn repository_root() -> Result<PathBuf> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| anyhow!("xtask must be located directly below the repository root"))
}
