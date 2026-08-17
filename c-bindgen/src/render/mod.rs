mod facade;
mod header;
mod manifest;

use std::fs;

use anyhow::{Context, Result};
use camino::Utf8Path;

use crate::model::BindingsModel;

const HEADER_FILENAME: &str = "wallet_engine.h";
const FACADE_FILENAME: &str = "wallet_engine.c";
const MANIFEST_FILENAME: &str = "wallet_engine.c-api.json";

pub(super) fn write_bindings(out_dir: &Utf8Path, model: &BindingsModel) -> Result<()> {
    let header = header::render(model);
    let facade = facade::render();
    let manifest = manifest::render(model.manifest())?;

    fs::create_dir_all(out_dir)
        .with_context(|| format!("failed to create C binding output directory {out_dir}"))?;
    write_file(&out_dir.join(HEADER_FILENAME), &header)?;
    write_file(&out_dir.join(FACADE_FILENAME), facade)?;
    write_file(&out_dir.join(MANIFEST_FILENAME), &manifest)
}

fn write_file(path: &Utf8Path, contents: &str) -> Result<()> {
    fs::write(path, contents).with_context(|| format!("failed to write C binding artifact {path}"))
}
