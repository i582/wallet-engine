use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use clap::ValueEnum;

use crate::files::{copy_generated, require_file};
use crate::paths::repository_root;
use crate::process::{cargo_command, run_command};

#[derive(Clone, Copy, Default, ValueEnum)]
pub(crate) enum AndroidAbi {
    #[default]
    All,
    #[value(name = "arm64-v8a")]
    Arm64,
    #[value(name = "x86_64")]
    X86_64,
}

impl AndroidAbi {
    fn targets(self) -> &'static [AndroidTarget] {
        match self {
            Self::All => &ANDROID_TARGETS,
            Self::Arm64 => &ANDROID_TARGETS[..1],
            Self::X86_64 => &ANDROID_TARGETS[1..],
        }
    }
}

#[derive(Clone, Copy)]
struct AndroidTarget {
    abi: &'static str,
    rust_target: &'static str,
    clang_prefix: &'static str,
}

const ANDROID_TARGETS: [AndroidTarget; 2] = [
    AndroidTarget {
        abi: "arm64-v8a",
        rust_target: "aarch64-linux-android",
        clang_prefix: "aarch64-linux-android",
    },
    AndroidTarget {
        abi: "x86_64",
        rust_target: "x86_64-linux-android",
        clang_prefix: "x86_64-linux-android",
    },
];

pub(crate) fn build_android(abi: AndroidAbi) -> Result<()> {
    let root = repository_root()?;
    let sdk = android_sdk()?;
    let ndk = latest_ndk(&sdk.join("ndk"))?;
    let toolchain = android_toolchain(&ndk)?;
    let target_dir = root.join("target/android");

    for target in abi.targets() {
        build_target(&root, &target_dir, &toolchain, *target)?;
    }
    Ok(())
}

fn android_sdk() -> Result<PathBuf> {
    env::var_os("ANDROID_SDK_ROOT")
        .or_else(|| env::var_os("ANDROID_HOME"))
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Android/sdk")))
        .ok_or_else(|| anyhow!("ANDROID_SDK_ROOT or ANDROID_HOME must be configured"))
}

fn build_target(
    root: &Path,
    target_dir: &Path,
    toolchain: &Path,
    target: AndroidTarget,
) -> Result<()> {
    let linker = android_linker(toolchain, target.clang_prefix)?;
    let target_key = target.rust_target.replace('-', "_").to_ascii_uppercase();
    let mut command = cargo_command(root, target_dir);
    command
        .env(format!("CARGO_TARGET_{target_key}_LINKER"), &linker)
        .arg("build")
        .arg("--manifest-path")
        .arg(root.join("Cargo.toml"))
        .arg("--target")
        .arg(target.rust_target)
        .arg("--profile")
        .arg("release-size")
        .arg("--locked");
    run_command(&mut command)?;

    let library = target_dir
        .join(target.rust_target)
        .join("release-size/libwallet_engine.so");
    require_file(&library)?;
    copy_generated(
        &library,
        &target_dir
            .join("jniLibs")
            .join(target.abi)
            .join("libwallet_engine.so"),
    )
}

fn latest_ndk(ndk_root: &Path) -> Result<PathBuf> {
    let entries = fs::read_dir(ndk_root).context("failed to read Android NDK directory")?;
    let mut versions = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    versions.sort_by_key(|path| version_key(path));
    versions
        .pop()
        .ok_or_else(|| anyhow!("Android NDK is required under {}", ndk_root.display()))
}

fn version_key(path: &Path) -> Vec<u64> {
    path.file_name()
        .and_then(OsStr::to_str)
        .unwrap_or_default()
        .split('.')
        .map(|part| part.parse::<u64>().unwrap_or_default())
        .collect()
}

fn android_toolchain(ndk: &Path) -> Result<PathBuf> {
    let prebuilt = ndk.join("toolchains/llvm/prebuilt");
    let candidates: &[&str] = match env::consts::OS {
        "macos" => &["darwin-aarch64", "darwin-x86_64"],
        "linux" => &["linux-x86_64"],
        "windows" => &["windows-x86_64"],
        _ => &[],
    };
    candidates
        .iter()
        .map(|candidate| prebuilt.join(candidate).join("bin"))
        .find(|candidate| candidate.is_dir())
        .ok_or_else(|| {
            anyhow!(
                "Android NDK host toolchain was not found under {}",
                prebuilt.display()
            )
        })
}

fn android_linker(toolchain: &Path, prefix: &str) -> Result<PathBuf> {
    let base = toolchain.join(format!("{prefix}28-clang"));
    let candidates = [
        base.clone(),
        base.with_extension("cmd"),
        base.with_extension("exe"),
    ];
    candidates
        .into_iter()
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| anyhow!("Android linker was not found: {}", base.display()))
}

#[cfg(test)]
mod tests {
    use super::version_key;
    use std::path::Path;

    #[test]
    fn compares_ndk_versions_numerically() {
        assert!(version_key(Path::new("28.10.1")) > version_key(Path::new("28.2.9")));
    }
}
