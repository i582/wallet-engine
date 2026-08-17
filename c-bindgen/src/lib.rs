//! Custom `UniFFI` backend for the Wallet Engine typed C facade.

mod cli;
mod loader;
mod model;
mod render;

use anyhow::Result;

pub use cli::Cli;

/// Loads the `UniFFI` component model and renders the artifacts currently
/// supported by the C backend.
///
/// # Errors
///
/// Returns an error when metadata cannot be loaded or an artifact cannot be
/// rendered into the output directory.
pub fn run(cli: &Cli) -> Result<()> {
    let components = loader::load_component_interfaces(&cli.library)?;
    let manifest = model::Manifest::from_components(&components);
    let _manifest_path = render::write_manifest(&cli.out_dir, &manifest)?;
    Ok(())
}
