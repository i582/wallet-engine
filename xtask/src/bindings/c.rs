use std::fs;

use anyhow::{Context, Result, anyhow, bail};

use crate::files::{copy_generated, normalize_file, require_file};
use crate::paths::repository_root;

pub(crate) fn generate_c(check: bool) -> Result<()> {
    let root = repository_root()?;
    let c_bindings = root.join("c-bindings");
    let output = root.join("bindings/c/wallet_engine.h");
    let temporary = tempfile::Builder::new()
        .prefix("wallet-engine-c-bindings-")
        .tempdir()
        .context("failed to create temporary C bindings directory")?;
    let generated = temporary.path().join("wallet_engine.h");

    let config = cbindgen::Config::from_file(c_bindings.join("cbindgen.toml"))
        .map_err(|error| anyhow!("failed to load cbindgen.toml: {error}"))?;
    let bindings = cbindgen::Builder::new()
        .with_crate(c_bindings)
        .with_config(config)
        .generate()
        .map_err(|error| anyhow!("failed to generate C header: {error}"))?;
    bindings.write_to_file(&generated);

    require_file(&generated)?;
    normalize_file(&generated)?;
    validate_header(&fs::read_to_string(&generated).context("failed to read generated C header")?)?;

    if check {
        let current = fs::read_to_string(&output).with_context(|| {
            format!(
                "missing generated header {}; run `cargo xtask bindings c`",
                output.display()
            )
        })?;
        let expected =
            fs::read_to_string(&generated).context("failed to read generated C header")?;
        if current != expected {
            bail!(
                "{} is stale; run `cargo xtask bindings c`",
                output.display()
            );
        }
    } else {
        copy_generated(&generated, &output)?;
    }

    Ok(())
}

fn validate_header(source: &str) -> Result<()> {
    if source.contains("WALLET_ENGINE_API")
        && source.contains("WALLET_ENGINE_ABI_VERSION")
        && source.contains("wallet_engine_abi_version(void);")
    {
        Ok(())
    } else {
        bail!("generated C header is missing the expected public API")
    }
}
