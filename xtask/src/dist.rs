mod android;
mod manifest;
mod native;
mod swift;
mod web;

use std::fs;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use sha2::{Digest, Sha256};

use crate::paths::repository_root;
use crate::process::run_command;
use crate::version::{project_version, verify_release_tag};

const DEFAULT_OUTPUT: &str = "target/distrib";

/// Selects one release artifact operation.
#[derive(Args)]
pub(crate) struct DistArgs {
    #[command(subcommand)]
    command: DistCommand,
}

/// Lists the artifacts and validations available to release CI.
#[derive(Subcommand)]
enum DistCommand {
    /// Build one native archive for a supported Rust target.
    Native(native::NativeArgs),
    /// Build the Swift package and `XCFramework`.
    Swift(swift::SwiftArgs),
    /// Build the Android AAR and Maven metadata.
    Android(android::AndroidArgs),
    /// Build the TypeScript package with its WebAssembly runtime.
    Web(web::WebArgs),
    /// Validate and describe a complete set of release assets.
    Manifest(manifest::ManifestArgs),
    /// Verify that a Git tag matches every public package.
    VerifyTag {
        /// Canonical `v`-prefixed release tag.
        #[arg(long)]
        tag: String,
    },
    /// Print the current public package version.
    Version,
}

/// Dispatches one release artifact operation.
pub(crate) fn run_dist(args: &DistArgs) -> Result<()> {
    let root = repository_root()?;
    match &args.command {
        DistCommand::Native(args) => native::run(&root, args),
        DistCommand::Swift(args) => swift::run(&root, args),
        DistCommand::Android(args) => android::run(&root, args),
        DistCommand::Web(args) => web::run(&root, args),
        DistCommand::Manifest(args) => manifest::run(&root, args),
        DistCommand::VerifyTag { tag } => {
            let release = verify_release_tag(&root, tag)?;
            println!("{}", release.version);
            Ok(())
        }
        DistCommand::Version => {
            println!("{}", project_version(&root)?);
            Ok(())
        }
    }
}

/// Resolves a caller-selected output below the repository unless it is absolute.
fn output_directory(root: &Path, output: Option<&Path>) -> PathBuf {
    let output = output.unwrap_or_else(|| Path::new(DEFAULT_OUTPUT));
    if output.is_absolute() {
        output.to_path_buf()
    } else {
        root.join(output)
    }
}

/// Creates a distribution output directory and returns its resolved path.
fn prepare_output_directory(root: &Path, output: Option<&Path>) -> Result<PathBuf> {
    let directory = output_directory(root, output);
    fs::create_dir_all(&directory)
        .with_context(|| format!("failed to create {}", directory.display()))?;
    Ok(directory)
}

/// Requires a generated or compiled release input to exist as a file.
fn require_file(path: &Path) -> Result<()> {
    if path.is_file() {
        Ok(())
    } else {
        bail!("release input does not exist: {}", path.display())
    }
}

/// Copies one release file and creates its destination directory.
fn copy_file(source: &Path, destination: &Path) -> Result<()> {
    require_file(source)?;
    let parent = destination
        .parent()
        .with_context(|| format!("{} has no parent directory", destination.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    fs::copy(source, destination).map(|_| ()).with_context(|| {
        format!(
            "failed to copy {} to {}",
            source.display(),
            destination.display()
        )
    })
}

/// Copies licenses and the repository readme into a staged package.
fn copy_package_metadata(root: &Path, destination: &Path) -> Result<()> {
    for name in ["LICENSE-MIT", "LICENSE-APACHE", "README.md"] {
        copy_file(&root.join(name), &destination.join(name))?;
    }
    Ok(())
}

/// Creates a gzip-compressed tar archive containing one top-level directory.
fn create_tar_gz(staging_root: &Path, directory_name: &str, output: &Path) -> Result<()> {
    if output.exists() {
        fs::remove_file(output)
            .with_context(|| format!("failed to replace {}", output.display()))?;
    }
    run_command(
        Command::new("tar")
            .arg("-C")
            .arg(staging_root)
            .arg("-czf")
            .arg(output)
            .arg(directory_name),
    )
}

/// Computes the lowercase SHA-256 digest of one release asset.
fn sha256(path: &Path) -> Result<String> {
    let mut file =
        File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .with_context(|| format!("failed to read {}", path.display()))?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

/// Writes the conventional sidecar checksum for one release asset.
fn write_checksum(path: &Path) -> Result<PathBuf> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .with_context(|| format!("invalid release asset name: {}", path.display()))?;
    let checksum_path = path.with_file_name(format!("{name}.sha256"));
    let mut output = File::create(&checksum_path)
        .with_context(|| format!("failed to create {}", checksum_path.display()))?;
    writeln!(output, "{}  {name}", sha256(path)?)
        .with_context(|| format!("failed to write {}", checksum_path.display()))?;
    Ok(checksum_path)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{output_directory, sha256, write_checksum};

    #[test]
    fn resolves_relative_distribution_directory_below_repository() {
        assert_eq!(
            output_directory(
                std::path::Path::new("/repo"),
                Some(std::path::Path::new("out"))
            ),
            std::path::Path::new("/repo/out")
        );
    }

    #[test]
    fn writes_a_conventional_sha256_sidecar() -> anyhow::Result<()> {
        let temporary = tempfile::tempdir()?;
        let artifact = temporary.path().join("artifact.bin");
        fs::write(&artifact, b"wallet-engine")?;
        assert_eq!(
            sha256(&artifact)?,
            "021d901f62b692764b8f3ea2681976e22d8bb0434f19bc38faa2c6eb301567f8"
        );
        let checksum = write_checksum(&artifact)?;
        assert_eq!(
            fs::read_to_string(checksum)?,
            "021d901f62b692764b8f3ea2681976e22d8bb0434f19bc38faa2c6eb301567f8  artifact.bin\n"
        );
        Ok(())
    }
}
