use std::path::PathBuf;

use clap::Parser;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    #[arg(short, long)]
    /// Node socket address to connect to
    pub address: String,
    #[arg(short, long)]
    /// Pay the mining reward to this public key
    pub public_key_file: PathBuf,
}

pub fn parse() -> Cli {
    Cli::parse()
}
