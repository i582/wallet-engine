//! Custom `UniFFI` backend for the Wallet Engine typed C facade.

mod cli;
mod compound_map;
mod custom_type_map;
mod enum_map;
mod error_map;
mod loader;
mod model;
mod naming;
mod object_map;
mod optional_map;
mod record_map;
mod render;
mod sequence_map;
mod template;
mod type_map;
mod type_registry;

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
