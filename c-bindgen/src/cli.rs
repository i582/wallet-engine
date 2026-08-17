use camino::Utf8PathBuf;
use clap::Parser;

/// Command-line arguments for the custom `UniFFI` C backend.
#[derive(Debug, Parser)]
#[command(
    name = "wallet-engine-c-bindgen",
    about = "Generate the typed Wallet Engine C facade from UniFFI metadata"
)]
pub struct Cli {
    /// Compiled library containing the Wallet Engine `UniFFI` metadata.
    #[arg(long, value_name = "PATH")]
    pub(super) library: Utf8PathBuf,

    /// Directory in which generated C binding artifacts are written.
    #[arg(long, value_name = "DIR")]
    pub(super) out_dir: Utf8PathBuf,
}
