use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use semver::Version;
use serde_json::Value;
use toml_edit::{DocumentMut, value};

const ROOT_MANIFEST: &str = "Cargo.toml";
const WASM_MANIFEST: &str = "bindgen/wasm/Cargo.toml";
const WEB_MANIFEST: &str = "web/package.json";
const CHANGELOG: &str = "CHANGELOG.md";
const INTERNAL_CRATES: [&str; 2] = ["ton-connect-client", "ton-connect-core"];

/// Identifies one release version and its canonical Git tag.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReleaseVersion {
    pub(crate) version: Version,
    pub(crate) tag: String,
}

impl ReleaseVersion {
    /// Parses a caller-supplied version and assigns the canonical `v`-prefixed tag.
    pub(crate) fn parse(value: &str) -> Result<Self> {
        let version = Version::parse(value)
            .with_context(|| format!("release version `{value}` is not valid SemVer"))?;
        let tag = format!("v{version}");
        Ok(Self { version, tag })
    }

    /// Parses a release tag and rejects non-canonical spellings.
    pub(crate) fn from_tag(tag: &str) -> Result<Self> {
        let value = tag
            .strip_prefix('v')
            .with_context(|| format!("release tag `{tag}` must start with `v`"))?;
        let release = Self::parse(value)?;
        if release.tag != tag {
            bail!(
                "release tag `{tag}` is not canonical; expected `{}`",
                release.tag
            );
        }
        Ok(release)
    }
}

/// Reads the public Wallet Engine version from the workspace manifest.
pub(crate) fn project_version(root: &Path) -> Result<Version> {
    read_toml_version(root, ROOT_MANIFEST, &["workspace", "package", "version"])
}

/// Verifies that every public package and the changelog use the release tag version.
pub(crate) fn verify_release_tag(root: &Path, tag: &str) -> Result<ReleaseVersion> {
    let release = ReleaseVersion::from_tag(tag)?;
    let expected = &release.version;
    verify_version(ROOT_MANIFEST, &project_version(root)?, expected)?;
    for crate_name in INTERNAL_CRATES {
        verify_version(
            &format!("{ROOT_MANIFEST} workspace dependency `{crate_name}`"),
            &read_toml_version(
                root,
                ROOT_MANIFEST,
                &["workspace", "dependencies", crate_name, "version"],
            )?,
            expected,
        )?;
    }
    verify_version(
        WASM_MANIFEST,
        &read_toml_version(root, WASM_MANIFEST, &["package", "version"])?,
        expected,
    )?;
    verify_version(
        WEB_MANIFEST,
        &read_json_version(root, WEB_MANIFEST)?,
        expected,
    )?;
    let _notes = release_notes(root, expected)?;
    Ok(release)
}

/// Updates every public package manifest to the selected release version.
pub(crate) fn write_project_versions(root: &Path, version: &Version) -> Result<()> {
    update_toml_version(
        root,
        ROOT_MANIFEST,
        &["workspace", "package", "version"],
        version,
    )?;
    for crate_name in INTERNAL_CRATES {
        update_toml_version(
            root,
            ROOT_MANIFEST,
            &["workspace", "dependencies", crate_name, "version"],
            version,
        )?;
    }
    update_toml_version(root, WASM_MANIFEST, &["package", "version"], version)?;
    update_json_version(root, WEB_MANIFEST, version)
}

/// Returns the Markdown body for one release from `CHANGELOG.md`.
pub(crate) fn release_notes(root: &Path, version: &Version) -> Result<String> {
    let path = root.join(CHANGELOG);
    let source =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    extract_release_notes(&source, version)
        .with_context(|| format!("{CHANGELOG} has no release section for `{version}`"))
}

/// Extracts one version section without including its heading.
fn extract_release_notes(source: &str, version: &Version) -> Result<String> {
    let bracketed = format!("## [{version}]");
    let plain = format!("## {version}");
    let mut collecting = false;
    let mut lines = Vec::new();

    for line in source.lines() {
        if line.starts_with("## ") {
            if collecting {
                break;
            }
            collecting = line.starts_with(&bracketed) || line.starts_with(&plain);
            continue;
        }
        if collecting {
            lines.push(line);
        }
    }

    let notes = lines.join("\n").trim().to_owned();
    if notes.is_empty() {
        bail!("release notes are empty")
    }
    Ok(format!("{notes}\n"))
}

/// Reads one string version from a TOML key path.
fn read_toml_version(root: &Path, relative: &str, keys: &[&str]) -> Result<Version> {
    let path = root.join(relative);
    let source =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let document = source
        .parse::<DocumentMut>()
        .with_context(|| format!("failed to parse {}", path.display()))?;
    let mut item = document.as_item();
    for key in keys {
        item = item
            .get(*key)
            .with_context(|| format!("missing `{}` in {relative}", keys.join(".")))?;
    }
    let raw = item
        .as_str()
        .with_context(|| format!("`{}` in {relative} is not a string", keys.join(".")))?;
    Version::parse(raw).with_context(|| format!("invalid version `{raw}` in {relative}"))
}

/// Writes one string version to a TOML key path used by the release manifests.
fn update_toml_version(
    root: &Path,
    relative: &str,
    keys: &[&str],
    version: &Version,
) -> Result<()> {
    let path = root.join(relative);
    let source =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut document = source
        .parse::<DocumentMut>()
        .with_context(|| format!("failed to parse {}", path.display()))?;
    let (first, rest) = keys
        .split_first()
        .context("TOML version key path must not be empty")?;
    let mut item = &mut document[*first];
    for key in rest {
        item = &mut item[*key];
    }
    *item = value(version.to_string());
    fs::write(&path, document.to_string())
        .with_context(|| format!("failed to write {}", path.display()))
}

/// Reads the version field from a JSON package manifest.
fn read_json_version(root: &Path, relative: &str) -> Result<Version> {
    let path = root.join(relative);
    let source =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let document: Value = serde_json::from_str(&source)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    let raw = document
        .get("version")
        .and_then(Value::as_str)
        .with_context(|| format!("missing string `version` in {relative}"))?;
    Version::parse(raw).with_context(|| format!("invalid version `{raw}` in {relative}"))
}

/// Writes the version field in a JSON package manifest.
fn update_json_version(root: &Path, relative: &str, version: &Version) -> Result<()> {
    let path = root.join(relative);
    let source =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut document: Value = serde_json::from_str(&source)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    document["version"] = Value::String(version.to_string());
    let output = format!("{}\n", serde_json::to_string_pretty(&document)?);
    fs::write(&path, output).with_context(|| format!("failed to write {}", path.display()))
}

/// Reports a manifest whose version does not match the release.
fn verify_version(path: &str, actual: &Version, expected: &Version) -> Result<()> {
    if actual == expected {
        Ok(())
    } else {
        bail!("{path} version `{actual}` does not match release version `{expected}`")
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use semver::Version;

    use super::{ReleaseVersion, WEB_MANIFEST, extract_release_notes, update_json_version};

    #[test]
    fn accepts_canonical_release_and_prerelease_tags() {
        assert!(ReleaseVersion::from_tag("v1.2.3").is_ok());
        assert!(ReleaseVersion::from_tag("v1.2.3-rc.1").is_ok());
    }

    #[test]
    fn rejects_noncanonical_release_tags() {
        assert!(ReleaseVersion::from_tag("1.2.3").is_err());
        assert!(ReleaseVersion::from_tag("v01.2.3").is_err());
    }

    #[test]
    fn extracts_only_the_requested_changelog_section() {
        let source =
            "# Changelog\n\n## [1.2.3] - today\n\n- Added release.\n\n## [1.2.2]\n\n- Older.\n";
        let notes = extract_release_notes(source, &Version::new(1, 2, 3));
        assert_eq!(notes.ok().as_deref(), Some("- Added release.\n"));
    }

    #[test]
    fn updates_web_version_without_reordering_manifest_fields() -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        fs::create_dir(root.path().join("web"))?;
        fs::write(
            root.path().join(WEB_MANIFEST),
            "{\n  \"name\": \"@ton/wallet-engine\",\n  \"version\": \"1.0.0\",\n  \"private\": true\n}\n",
        )?;

        update_json_version(root.path(), WEB_MANIFEST, &Version::new(1, 1, 0))?;

        let updated = fs::read_to_string(root.path().join(WEB_MANIFEST))?;
        let name = updated.find("\"name\"").expect("name field remains");
        let version = updated.find("\"version\"").expect("version field remains");
        let private = updated.find("\"private\"").expect("private field remains");
        assert!(name < version && version < private);
        assert!(updated.contains("\"version\": \"1.1.0\""));
        Ok(())
    }
}
