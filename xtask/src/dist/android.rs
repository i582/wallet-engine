use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use clap::Args;

use crate::android::{AndroidAbi, android_sdk, build_android};
use crate::bindings::generate_kotlin;
use crate::dist::{copy_file, prepare_output_directory, require_file, write_checksum};
use crate::process::{command_output, run_command};
use crate::version::project_version;

/// Selects the output directory for Android release artifacts.
#[derive(Args)]
pub(crate) struct AndroidArgs {
    /// Directory that receives the AAR, POM, and checksums.
    #[arg(long)]
    output: Option<PathBuf>,
}

/// Builds the Kotlin wrapper and both supported Android native ABIs into one AAR.
pub(crate) fn run(root: &Path, args: &AndroidArgs) -> Result<()> {
    let version = project_version(root)?;
    let output = prepare_output_directory(root, args.output.as_deref())?;
    generate_kotlin(false)?;
    build_android(AndroidAbi::All)?;

    let gradle_home = env::var_os("GRADLE_USER_HOME")
        .map_or_else(|| root.join("target/release-gradle"), PathBuf::from);
    let sdk = android_sdk()?;
    run_command(
        Command::new(root.join("examples/android/gradlew"))
            .current_dir(root)
            .env("GRADLE_USER_HOME", gradle_home)
            .env("ANDROID_HOME", &sdk)
            .env("ANDROID_SDK_ROOT", &sdk)
            .arg("-p")
            .arg(root.join("packaging/android"))
            .arg("assembleRelease")
            .arg("--no-configuration-cache"),
    )?;

    let built = root.join("packaging/android/build/outputs/aar/wallet-engine-release.aar");
    require_file(&built)?;
    validate_aar(&built)?;
    let artifact = output.join(format!("wallet-engine-android-{version}.aar"));
    copy_file(&built, &artifact)?;
    let _aar_checksum = write_checksum(&artifact)?;

    let pom = output.join(format!("wallet-engine-android-{version}.pom"));
    fs::write(&pom, android_pom(&version.to_string()))
        .with_context(|| format!("failed to write {}", pom.display()))?;
    let _pom_checksum = write_checksum(&pom)?;
    println!("{}", artifact.display());
    Ok(())
}

/// Verifies that the AAR contains compiled Kotlin and both native libraries.
fn validate_aar(path: &Path) -> Result<()> {
    let listing = command_output(Command::new("jar").arg("tf").arg(path))?;
    for expected in [
        "classes.jar",
        "jni/arm64-v8a/libwallet_engine.so",
        "jni/x86_64/libwallet_engine.so",
    ] {
        if !listing.lines().any(|entry| entry == expected) {
            bail!("Android AAR is missing `{expected}`: {}", path.display())
        }
    }
    Ok(())
}

/// Produces Maven metadata for dependencies required by the generated Kotlin wrapper.
fn android_pom(version: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<project xmlns="http://maven.apache.org/POM/4.0.0"
         xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
         xsi:schemaLocation="http://maven.apache.org/POM/4.0.0 https://maven.apache.org/xsd/maven-4.0.0.xsd">
  <modelVersion>4.0.0</modelVersion>
  <groupId>org.ton</groupId>
  <artifactId>wallet-engine</artifactId>
  <version>{version}</version>
  <packaging>aar</packaging>
  <name>Wallet Engine</name>
  <licenses>
    <license><name>MIT OR Apache-2.0</name></license>
  </licenses>
  <dependencies>
    <dependency>
      <groupId>net.java.dev.jna</groupId>
      <artifactId>jna</artifactId>
      <version>5.12.0</version>
      <type>aar</type>
    </dependency>
    <dependency>
      <groupId>org.jetbrains.kotlinx</groupId>
      <artifactId>kotlinx-coroutines-android</artifactId>
      <version>1.10.2</version>
    </dependency>
    <dependency>
      <groupId>androidx.annotation</groupId>
      <artifactId>annotation</artifactId>
      <version>1.9.1</version>
      <scope>runtime</scope>
    </dependency>
  </dependencies>
</project>
"#
    )
}

#[cfg(test)]
mod tests {
    use super::android_pom;

    #[test]
    fn pom_contains_release_version_and_wrapper_dependencies() {
        let pom = android_pom("1.2.3");
        assert!(pom.contains("<version>1.2.3</version>"));
        assert!(pom.contains("<artifactId>jna</artifactId>"));
        assert!(pom.contains("<artifactId>kotlinx-coroutines-android</artifactId>"));
    }
}
