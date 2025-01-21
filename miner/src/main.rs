use std::path::PathBuf;

use anyhow::Result;
use btclib::crypto::PublicKey;
use btclib::Saveable;
use clap::Parser;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[arg(short, long)]
    address: String,
    #[arg(short, long)]
    public_key_file: PathBuf,
}

#[tokio::main]
pub async fn main() -> Result<()> {
    let cli = Cli::parse();
    println!("address: {}", cli.address);
    println!("public key file: {:?}", cli.public_key_file);

    let public_key = PublicKey::load_from_file(&cli.public_key_file)
        .map_err(|e| anyhow::anyhow!("Error reading public key: {}", e))?;

    println!(
        "TODO: Connecting to {} to mine with {public_key:?}",
        cli.address
    );
    Ok(())
}
