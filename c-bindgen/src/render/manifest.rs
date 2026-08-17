use anyhow::{Context, Result};

use crate::model::Manifest;

pub(super) fn render(manifest: &Manifest) -> Result<String> {
    let mut contents = serde_json::to_string_pretty(manifest)
        .context("failed to serialize C binding interface manifest")?;
    contents.push('\n');
    Ok(contents)
}
