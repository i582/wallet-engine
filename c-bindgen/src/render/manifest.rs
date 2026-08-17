use std::fs;

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};

use crate::model::Manifest;

const MANIFEST_FILENAME: &str = "wallet_engine.c-api.json";

pub(super) fn write_manifest(out_dir: &Utf8Path, manifest: &Manifest) -> Result<Utf8PathBuf> {
    fs::create_dir_all(out_dir)
        .with_context(|| format!("failed to create C binding output directory {out_dir}"))?;

    let output = out_dir.join(MANIFEST_FILENAME);
    let mut contents = serde_json::to_string_pretty(manifest)
        .context("failed to serialize C binding interface manifest")?;
    contents.push('\n');
    fs::write(&output, contents)
        .with_context(|| format!("failed to write C binding manifest {output}"))?;
    Ok(output)
}
