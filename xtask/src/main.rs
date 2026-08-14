mod android;
mod bindings;
mod files;
mod paths;
mod process;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::android::{AndroidAbi, build_android};
use crate::bindings::{generate_kotlin, generate_swift, generate_wasm};

#[derive(Parser)]
#[command(about = "Wallet Engine repository tasks")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Generate or verify foreign-language bindings.
    Bindings {
        #[command(subcommand)]
        language: BindingLanguage,
    },
    /// Build Android shared libraries.
    Android {
        #[arg(long, value_enum, default_value_t = AndroidAbi::All)]
        abi: AndroidAbi,
    },
}

#[derive(Subcommand)]
enum BindingLanguage {
    /// Generate Swift sources and the Apple C module.
    Swift {
        /// Verify generation without updating ignored outputs.
        #[arg(long)]
        check: bool,
    },
    /// Generate Kotlin sources.
    Kotlin {
        /// Verify generation without updating ignored outputs.
        #[arg(long)]
        check: bool,
    },
    /// Generate the browser WebAssembly package.
    Wasm {
        /// Verify generation without updating ignored outputs.
        #[arg(long)]
        check: bool,
    },
}

fn main() -> Result<()> {
    run()
}

fn run() -> Result<()> {
    match Cli::parse().command {
        Command::Bindings {
            language: BindingLanguage::Swift { check },
        } => generate_swift(check),
        Command::Bindings {
            language: BindingLanguage::Kotlin { check },
        } => generate_kotlin(check),
        Command::Bindings {
            language: BindingLanguage::Wasm { check },
        } => generate_wasm(check),
        Command::Android { abi } => build_android(abi),
    }
}
