use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

const CLEAN_BUILD_ENV: &[&str] = &[
    "SDKROOT",
    "LIBRARY_PATH",
    "CPATH",
    "C_INCLUDE_PATH",
    "CPLUS_INCLUDE_PATH",
    "CFLAGS",
    "CXXFLAGS",
    "CPPFLAGS",
    "LDFLAGS",
];

fn cargo() -> OsString {
    env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"))
}

pub(crate) fn cargo_command(root: &Path, target_dir: &Path) -> Command {
    let mut command = Command::new(cargo());
    command
        .current_dir(root)
        .env("CARGO_TARGET_DIR", target_dir);
    for variable in CLEAN_BUILD_ENV {
        command.env_remove(variable);
    }
    command
}

pub(crate) fn run_command(command: &mut Command) -> Result<()> {
    eprintln!("+ {command:?}");
    let status = command
        .status()
        .with_context(|| format!("failed to start {command:?}"))?;
    if status.success() {
        Ok(())
    } else {
        bail!("command failed with {status}: {command:?}")
    }
}

pub(crate) fn build_engine_cdylib(root: &Path, target_dir: &Path) -> Result<PathBuf> {
    run_command(
        cargo_command(root, target_dir)
            .arg("build")
            .arg("--manifest-path")
            .arg(root.join("Cargo.toml"))
            .arg("--release")
            .arg("--locked"),
    )?;
    let library = engine_cdylib_path(target_dir);
    if !library.is_file() {
        bail!(
            "Cargo did not produce expected cdylib: {}",
            library.display()
        );
    }
    Ok(library)
}

fn engine_cdylib_path(target_dir: &Path) -> PathBuf {
    let filename = format!(
        "{}wallet_engine{}",
        env::consts::DLL_PREFIX,
        env::consts::DLL_SUFFIX,
    );
    target_dir.join("release").join(filename)
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::path::Path;

    use super::engine_cdylib_path;

    #[test]
    fn locates_engine_cdylib_using_host_platform_naming() {
        let expected = format!(
            "{}wallet_engine{}",
            env::consts::DLL_PREFIX,
            env::consts::DLL_SUFFIX,
        );

        assert_eq!(
            engine_cdylib_path(Path::new("target")),
            Path::new("target/release").join(expected),
        );
    }
}
