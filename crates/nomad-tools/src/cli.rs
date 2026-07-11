use std::path::PathBuf;

use clap::Parser;

#[derive(Parser)]
#[command(
    name = "nodepage-rs",
    bin_name = "nodepage-rs",
    about = "Minimal NomadNetwork page-hosting node over rsReticulum",
    version
)]
pub struct Args {
    /// Path to configuration directory.
    #[arg(short, long)]
    pub config: Option<String>,

    /// Path to an alternative Reticulum configuration directory.
    #[arg(long)]
    pub rnsconfig: Option<String>,

    /// Override pages_dir from the config file.
    #[arg(long)]
    pub pages: Option<PathBuf>,

    /// Override files_dir from the config file.
    #[arg(long)]
    pub files: Option<PathBuf>,

    /// Print an example config file and exit.
    #[arg(long)]
    pub exampleconfig: bool,

    /// Increase verbosity (can be repeated).
    #[arg(short, long, action = clap::ArgAction::Count)]
    pub verbose: u8,
}
