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

#[cfg(test)]
mod tests {
    use std::{env, ffi::OsString, process::Command};

    use anyhow::{Context, Result, ensure};
    use camino::Utf8Path;
    use uniffi_bindgen::ComponentInterface;

    use super::{FACADE_FILENAME, write_bindings};
    use crate::model::BindingsModel;

    #[test]
    fn generated_skeleton_compiles_as_strict_c11() -> Result<()> {
        let component = ComponentInterface::from_webidl(
            r"
            namespace wallet_engine {};

            dictionary Example {
                boolean enabled;
                u64 revision;
                string name;
                bytes payload;
            };
            ",
            "wallet_engine",
        )?;
        let model = BindingsModel::from_components(&[component])?;
        let temporary =
            tempfile::tempdir().context("failed to create C compile-smoke directory")?;
        let out_dir = Utf8Path::from_path(temporary.path())
            .context("C compile-smoke path is not valid UTF-8")?;
        write_bindings(out_dir, &model)?;

        let compiler = env::var_os("CC").unwrap_or_else(|| OsString::from("cc"));
        let output = Command::new(compiler)
            .arg("-std=c11")
            .arg("-Wall")
            .arg("-Wextra")
            .arg("-Werror")
            .arg("-pedantic")
            .arg("-I")
            .arg(out_dir)
            .arg("-c")
            .arg(out_dir.join(FACADE_FILENAME))
            .arg("-o")
            .arg(out_dir.join("wallet_engine.o"))
            .output()
            .context("failed to start the C compiler")?;

        ensure!(
            output.status.success(),
            "generated C11 facade did not compile:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(())
    }
}
