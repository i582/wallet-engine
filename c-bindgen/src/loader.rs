use anyhow::{Context, Result, bail};
use camino::Utf8Path;
use uniffi_bindgen::{BindgenLoader, BindgenPaths, ComponentInterface, GlobalConfig};

pub(super) fn load_component_interfaces(library: &Utf8Path) -> Result<Vec<ComponentInterface>> {
    if !library.is_file() {
        bail!("input library does not exist or is not a file: {library}");
    }

    let loader = BindgenLoader::new(BindgenPaths::default(), GlobalConfig::default());
    let metadata = loader
        .load_metadata(library)
        .with_context(|| format!("failed to load UniFFI metadata from {library}"))?;
    let mut components = loader
        .load_cis(metadata)
        .with_context(|| format!("failed to build ComponentInterface from {library}"))?;

    if components.is_empty() {
        bail!("no UniFFI component interfaces found in {library}");
    }

    for component in &mut components {
        component.check_consistency().with_context(|| {
            format!(
                "inconsistent UniFFI component interface for crate {}",
                component.crate_name()
            )
        })?;
        component.derive_ffi_funcs().with_context(|| {
            format!(
                "failed to derive UniFFI FFI functions for crate {}",
                component.crate_name()
            )
        })?;
    }

    components.sort_by(|left, right| left.crate_name().cmp(right.crate_name()));
    Ok(components)
}
