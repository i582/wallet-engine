use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::Args;

use crate::bindings::generate_cpp;
use crate::dist::{
    copy_file, copy_package_metadata, create_tar_gz, create_zip, prepare_output_directory,
    require_file, strip_release_library, write_checksum,
};
use crate::process::{cargo_command, run_command};
use crate::version::project_version;

/// Selects one supported native target and output directory.
#[derive(Args)]
pub(crate) struct NativeArgs {
    /// Rust target triple to compile and package.
    #[arg(long)]
    target: String,
    /// Directory that receives the archive and checksum.
    #[arg(long)]
    output: Option<PathBuf>,
}

/// Describes target-specific native library and archive naming.
struct NativeTarget {
    triple: &'static str,
    static_library: &'static str,
    dynamic_library: &'static str,
    import_library: Option<&'static str>,
    dynamic_directory: &'static str,
    archive: NativeArchive,
    strip_libraries: bool,
}

/// Selects the archive format expected by one native platform.
enum NativeArchive {
    TarGz,
    Zip,
}

const TARGETS: &[NativeTarget] = &[
    NativeTarget {
        triple: "x86_64-unknown-linux-gnu",
        static_library: "libwallet_engine.a",
        dynamic_library: "libwallet_engine.so",
        import_library: None,
        dynamic_directory: "lib",
        archive: NativeArchive::TarGz,
        strip_libraries: true,
    },
    NativeTarget {
        triple: "aarch64-unknown-linux-gnu",
        static_library: "libwallet_engine.a",
        dynamic_library: "libwallet_engine.so",
        import_library: None,
        dynamic_directory: "lib",
        archive: NativeArchive::TarGz,
        strip_libraries: true,
    },
    NativeTarget {
        triple: "x86_64-apple-darwin",
        static_library: "libwallet_engine.a",
        dynamic_library: "libwallet_engine.dylib",
        import_library: None,
        dynamic_directory: "lib",
        archive: NativeArchive::TarGz,
        strip_libraries: true,
    },
    NativeTarget {
        triple: "aarch64-apple-darwin",
        static_library: "libwallet_engine.a",
        dynamic_library: "libwallet_engine.dylib",
        import_library: None,
        dynamic_directory: "lib",
        archive: NativeArchive::TarGz,
        strip_libraries: true,
    },
    NativeTarget {
        triple: "x86_64-pc-windows-msvc",
        static_library: "wallet_engine.lib",
        dynamic_library: "wallet_engine.dll",
        import_library: Some("wallet_engine.dll.lib"),
        dynamic_directory: "bin",
        archive: NativeArchive::Zip,
        strip_libraries: false,
    },
];

/// Builds a native library pair and packages its matching C++ wrapper.
pub(crate) fn run(root: &Path, args: &NativeArgs) -> Result<()> {
    let target = target(&args.target)?;
    let version = project_version(root)?;
    let output = prepare_output_directory(root, args.output.as_deref())?;
    let build_root = root.join("target/release-native");

    run_command(
        cargo_command(root, &build_root)
            .arg("build")
            .arg("--release")
            .arg("--locked")
            .arg("--target")
            .arg(target.triple),
    )?;
    generate_cpp()?;

    let release_dir = build_root.join(target.triple).join("release");
    let static_library = release_dir.join(target.static_library);
    let dynamic_library = release_dir.join(target.dynamic_library);
    require_file(&static_library)?;
    require_file(&dynamic_library)?;
    if let Some(import_library) = target.import_library {
        require_file(&release_dir.join(import_library))?;
    }

    let package_name = format!("wallet-engine-{version}-{}", target.triple);
    let temporary = tempfile::Builder::new()
        .prefix("wallet-engine-native-release-")
        .tempdir()?;
    let package = temporary.path().join(&package_name);
    let library_dir = package.join("lib");
    let dynamic_dir = package.join(target.dynamic_directory);
    let include_dir = package.join("include");
    let source_dir = package.join("src");
    fs::create_dir_all(&library_dir)?;
    fs::create_dir_all(&dynamic_dir)?;
    fs::create_dir_all(&include_dir)?;
    fs::create_dir_all(&source_dir)?;

    let packaged_static_library = library_dir.join(target.static_library);
    copy_file(&static_library, &packaged_static_library)?;
    let packaged_dynamic_library = dynamic_dir.join(target.dynamic_library);
    copy_file(&dynamic_library, &packaged_dynamic_library)?;
    if target.strip_libraries {
        strip_release_library(&packaged_static_library)?;
        strip_release_library(&packaged_dynamic_library)?;
    }
    if let Some(import_library) = target.import_library {
        copy_file(
            &release_dir.join(import_library),
            &library_dir.join(import_library),
        )?;
    }
    let generated = root.join("bindings/cpp-experimental");
    copy_file(
        &generated.join("wallet_engine.hpp"),
        &include_dir.join("wallet_engine.hpp"),
    )?;
    copy_file(
        &generated.join("wallet_engine_scaffolding.hpp"),
        &include_dir.join("wallet_engine_scaffolding.hpp"),
    )?;
    copy_file(
        &generated.join("wallet_engine.cpp"),
        &source_dir.join("wallet_engine.cpp"),
    )?;
    fs::write(package.join("CMakeLists.txt"), cmake_package(target))?;
    copy_package_metadata(root, &package)?;

    let archive = match target.archive {
        NativeArchive::TarGz => {
            let archive = output.join(format!("{package_name}.tar.gz"));
            create_tar_gz(temporary.path(), &package_name, &archive)?;
            archive
        }
        NativeArchive::Zip => {
            let archive = output.join(format!("{package_name}.zip"));
            create_zip(temporary.path(), &package_name, &archive)?;
            archive
        }
    };
    let _checksum = write_checksum(&archive)?;
    println!("{}", archive.display());
    Ok(())
}

/// Resolves a supported target triple to its artifact naming contract.
fn target(triple: &str) -> Result<&'static NativeTarget> {
    TARGETS
        .iter()
        .find(|target| target.triple == triple)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "unsupported native release target `{triple}`; expected one of {}",
                TARGETS
                    .iter()
                    .map(|target| target.triple)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
}

/// Produces a `CMake` package that exposes shared, static, and C++ wrapper targets.
fn cmake_package(target: &NativeTarget) -> String {
    let dynamic_library = format!("{}/{}", target.dynamic_directory, target.dynamic_library);
    let import_library = target.import_library.map_or_else(String::new, |library| {
        format!("\n    IMPORTED_IMPLIB \"${{CMAKE_CURRENT_LIST_DIR}}/lib/{library}\"")
    });
    let static_library = target.static_library;
    format!(
        r#"cmake_minimum_required(VERSION 3.16)
project(wallet_engine_release LANGUAGES CXX)

add_library(wallet_engine_native SHARED IMPORTED GLOBAL)
set_target_properties(wallet_engine_native PROPERTIES
    IMPORTED_LOCATION "${{CMAKE_CURRENT_LIST_DIR}}/{dynamic_library}"{import_library}
)
add_library(WalletEngine::native ALIAS wallet_engine_native)

add_library(wallet_engine_native_static STATIC IMPORTED GLOBAL)
set_target_properties(wallet_engine_native_static PROPERTIES
    IMPORTED_LOCATION "${{CMAKE_CURRENT_LIST_DIR}}/lib/{static_library}"
)
add_library(WalletEngine::native_static ALIAS wallet_engine_native_static)

add_library(wallet_engine_cpp STATIC src/wallet_engine.cpp)
target_compile_features(wallet_engine_cpp PUBLIC cxx_std_20)
target_include_directories(wallet_engine_cpp PUBLIC "${{CMAKE_CURRENT_LIST_DIR}}/include")
target_link_libraries(wallet_engine_cpp PUBLIC wallet_engine_native)
add_library(WalletEngine::cpp ALIAS wallet_engine_cpp)
"#
    )
}

#[cfg(test)]
mod tests {
    use super::{cmake_package, target};

    #[test]
    fn supports_the_release_target_matrix() {
        assert!(target("x86_64-unknown-linux-gnu").is_ok());
        assert!(target("aarch64-unknown-linux-gnu").is_ok());
        assert!(target("x86_64-apple-darwin").is_ok());
        assert!(target("aarch64-apple-darwin").is_ok());
        assert!(target("x86_64-pc-windows-msvc").is_ok());
    }

    #[test]
    fn cmake_package_uses_the_target_dynamic_library() {
        let source = cmake_package(target("x86_64-unknown-linux-gnu").unwrap());
        assert!(source.contains("lib/libwallet_engine.so"));
        assert!(source.contains("WalletEngine::cpp"));
    }

    #[test]
    fn windows_cmake_package_uses_runtime_and_link_libraries() {
        let source = cmake_package(target("x86_64-pc-windows-msvc").unwrap());
        assert!(source.contains("bin/wallet_engine.dll"));
        assert!(source.contains("lib/wallet_engine.dll.lib"));
        assert!(source.contains("lib/wallet_engine.lib"));
    }
}
