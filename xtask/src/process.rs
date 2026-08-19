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

/// Returns the Cargo executable selected by the invoking toolchain.
fn cargo() -> OsString {
    env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"))
}

/// Creates a Cargo command with repository build variables isolated from Xcode.
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

/// Runs a child process and reports a command line when it fails.
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

/// Runs a child process and returns its UTF-8 standard output.
pub(crate) fn command_output(command: &mut Command) -> Result<String> {
    eprintln!("+ {command:?}");
    let output = command
        .output()
        .with_context(|| format!("failed to start {command:?}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "command failed with {}: {command:?}: {}",
            output.status,
            stderr.trim()
        );
    }
    String::from_utf8(output.stdout).context("command output is not valid UTF-8")
}

/// Returns the build directory shared by host-side binding generation.
pub(crate) fn bindings_target_dir(root: &Path) -> PathBuf {
    root.join("target/bindings-host")
}

/// Returns the build directory shared by pinned binding generator tools.
pub(crate) fn bindgen_target_dir(root: &Path) -> PathBuf {
    root.join("target/bindgen-tools")
}

/// Builds the optimized host cdylib consumed by `UniFFI` binding generators.
pub(crate) fn build_engine_cdylib(root: &Path) -> Result<PathBuf> {
    let target_dir = bindings_target_dir(root);
    run_command(
        cargo_command(root, &target_dir)
            .arg("build")
            .arg("--manifest-path")
            .arg(root.join("Cargo.toml"))
            .arg("--release")
            .arg("--locked"),
    )?;
    let library = engine_cdylib_path(&target_dir);
    if !library.is_file() {
        bail!(
            "Cargo did not produce expected cdylib: {}",
            library.display()
        );
    }
    Ok(library)
}

/// Returns the expected host cdylib path for the current operating system.
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
