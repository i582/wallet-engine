mod manifest;

use anyhow::Result;
use camino::{Utf8Path, Utf8PathBuf};

use crate::model::Manifest;

pub(super) fn write_manifest(out_dir: &Utf8Path, value: &Manifest) -> Result<Utf8PathBuf> {
    manifest::write_manifest(out_dir, value)
}
