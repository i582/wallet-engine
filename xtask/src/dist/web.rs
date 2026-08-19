use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use clap::Args;
use serde_json::{Map, Value, json};

use crate::bindings::generate_wasm;
use crate::dist::{copy_file, prepare_output_directory, require_file, write_checksum};
use crate::process::{command_output, run_command};
use crate::version::project_version;

/// Selects the output directory for the browser release package.
#[derive(Args)]
pub(crate) struct WebArgs {
    /// Directory that receives the npm-compatible package and checksum.
    #[arg(long)]
    output: Option<PathBuf>,
}

/// Builds the high-level TypeScript wrapper and WebAssembly runtime into one package.
pub(crate) fn run(root: &Path, args: &WebArgs) -> Result<()> {
    let version = project_version(root)?;
    let output = prepare_output_directory(root, args.output.as_deref())?;
    generate_wasm(false)?;

    let temporary = tempfile::Builder::new()
        .prefix("wallet-engine-web-release-")
        .tempdir()?;
    let package = temporary.path().join("package");
    copy_directory(&root.join("web/src"), &package.join("src"))?;
    copy_directory(&root.join("bindings/wasm"), &package.join("wasm"))?;
    rewrite_generated_imports(&package.join("src"))?;
    let source_manifest = fs::read_to_string(root.join("web/package.json"))?;
    fs::write(package.join("package.json"), &source_manifest)?;
    copy_file(&root.join("web/bun.lock"), &package.join("bun.lock"))?;
    fs::write(package.join("tsconfig.json"), RELEASE_TSCONFIG)?;
    copy_file(&root.join("WASM.md"), &package.join("README.md"))?;
    copy_file(&root.join("LICENSE-MIT"), &package.join("LICENSE-MIT"))?;
    copy_file(
        &root.join("LICENSE-APACHE"),
        &package.join("LICENSE-APACHE"),
    )?;

    run_command(
        Command::new("bun")
            .current_dir(&package)
            .arg("install")
            .arg("--frozen-lockfile"),
    )?;
    run_command(
        Command::new("bun")
            .current_dir(&package)
            .arg("build")
            .arg("src/index.ts")
            .arg("--outdir")
            .arg("dist")
            .arg("--target")
            .arg("browser")
            .arg("--format")
            .arg("esm")
            .arg("--minify")
            .arg("--external")
            .arg("@tonconnect/protocol")
            .arg("--external")
            .arg("idb"),
    )?;
    run_command(
        Command::new(package.join("node_modules/.bin/tsc"))
            .current_dir(&package)
            .arg("--project")
            .arg("tsconfig.json"),
    )?;

    copy_file(
        &package.join("wasm/wallet_engine_bg.wasm"),
        &package.join("dist/wallet_engine_bg.wasm"),
    )?;
    copy_file(
        &package.join("wasm/wallet_engine.d.ts"),
        &package.join("dist/types/wasm/wallet_engine.d.ts"),
    )?;
    copy_file(
        &package.join("wasm/wallet_engine_bg.wasm.d.ts"),
        &package.join("dist/types/wasm/wallet_engine_bg.wasm.d.ts"),
    )?;
    fs::write(
        package.join("package.json"),
        release_package_manifest(&source_manifest)?,
    )?;
    validate_web_package(&package)?;

    let packed = command_output(
        Command::new("npm")
            .current_dir(&package)
            .env("npm_config_cache", temporary.path().join("npm-cache"))
            .arg("pack")
            .arg("--pack-destination")
            .arg(&output),
    )?;
    let file_name = packed
        .lines()
        .last()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .context("npm pack did not report an artifact name")?;
    let artifact = output.join(file_name);
    require_file(&artifact)?;
    let expected = format!("ton-wallet-engine-{version}.tgz");
    if file_name != expected {
        bail!("npm produced `{file_name}`, expected `{expected}`")
    }
    let _checksum = write_checksum(&artifact)?;
    println!("{}", artifact.display());
    Ok(())
}

/// Copies a generated source tree without preserving build artifacts.
fn copy_directory(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)?;
    for entry in
        fs::read_dir(source).with_context(|| format!("failed to read {}", source.display()))?
    {
        let entry = entry?;
        let destination_path = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_directory(&entry.path(), &destination_path)?;
        } else {
            copy_file(&entry.path(), &destination_path)?;
        }
    }
    Ok(())
}

/// Repoints repository-relative WASM imports to the staged package directory.
fn rewrite_generated_imports(source_root: &Path) -> Result<()> {
    for entry in fs::read_dir(source_root)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            rewrite_generated_imports(&path)?;
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("ts") {
            let source = fs::read_to_string(&path)?;
            let rewritten = source.replace("../../bindings/wasm/", "../wasm/");
            fs::write(&path, rewritten)?;
        }
    }
    Ok(())
}

/// Verifies the files referenced by the package export map.
fn validate_web_package(package: &Path) -> Result<()> {
    for relative in [
        "dist/index.js",
        "dist/types/src/index.d.ts",
        "dist/wallet_engine_bg.wasm",
    ] {
        require_file(&package.join(relative))?;
    }
    Ok(())
}

/// Converts the checked-in package manifest to generated release entry points.
fn release_package_manifest(source: &str) -> Result<String> {
    let mut manifest: Value = serde_json::from_str(source).context("invalid web/package.json")?;
    let object = manifest
        .as_object_mut()
        .context("web/package.json must contain a JSON object")?;
    object.insert("private".to_owned(), Value::Bool(false));
    object.insert("sideEffects".to_owned(), Value::Bool(false));
    object.insert(
        "main".to_owned(),
        Value::String("./dist/index.js".to_owned()),
    );
    object.insert(
        "types".to_owned(),
        Value::String("./dist/types/src/index.d.ts".to_owned()),
    );
    object.insert(
        "exports".to_owned(),
        json!({
            ".": {
                "types": "./dist/types/src/index.d.ts",
                "import": "./dist/index.js"
            },
            "./wallet_engine_bg.wasm": "./dist/wallet_engine_bg.wasm"
        }),
    );
    object.insert(
        "files".to_owned(),
        json!(["dist", "README.md", "LICENSE-MIT", "LICENSE-APACHE"]),
    );
    remove_dev_fields(object);
    Ok(format!("{}\n", serde_json::to_string_pretty(&manifest)?))
}

/// Removes repository-only commands and tools from the downloadable manifest.
fn remove_dev_fields(manifest: &mut Map<String, Value>) {
    for field in ["scripts", "devDependencies", "packageManager"] {
        let _removed = manifest.remove(field);
    }
}

const RELEASE_TSCONFIG: &str = r#"{
  "compilerOptions": {
    "target": "ESNext",
    "module": "ESNext",
    "lib": ["DOM", "DOM.Iterable", "ESNext"],
    "strict": true,
    "skipLibCheck": true,
    "moduleResolution": "Bundler",
    "isolatedModules": true,
    "declaration": true,
    "emitDeclarationOnly": true,
    "rootDir": ".",
    "outDir": "dist/types"
  },
  "include": ["src/**/*.ts", "wasm/*.d.ts", "wasm/*_base64.ts"]
}
"#;

#[cfg(test)]
mod tests {
    use super::release_package_manifest;

    #[test]
    fn release_manifest_reuses_source_metadata_and_exports_built_files() -> anyhow::Result<()> {
        let source = r#"{
          "name": "@ton/wallet-engine",
          "version": "1.2.3",
          "dependencies": {"idb": "8.0.3"},
          "scripts": {"test": "bun test"}
        }"#;
        let manifest = release_package_manifest(source)?;
        assert!(manifest.contains("\"name\": \"@ton/wallet-engine\""));
        assert!(manifest.contains("\"version\": \"1.2.3\""));
        assert!(manifest.contains("\"idb\": \"8.0.3\""));
        assert!(manifest.contains("dist/types/src/index.d.ts"));
        assert!(manifest.contains("wallet_engine_bg.wasm"));
        assert!(!manifest.contains("bun test"));
        Ok(())
    }
}
