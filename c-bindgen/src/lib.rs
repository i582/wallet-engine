//! Custom `UniFFI` backend for the Wallet Engine typed C facade.

mod cli;
mod enum_map;
mod loader;
mod model;
mod naming;
mod render;
mod template;
mod type_map;

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
    let model = model::BindingsModel::from_components(&components)?;
    render::write_bindings(&cli.out_dir, &model)
}
