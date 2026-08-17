use anyhow::Result;
use clap::Parser as _;
use wallet_engine_c_bindgen::{Cli, run};

fn main() -> Result<()> {
    let cli = Cli::parse();
    run(&cli)
}
