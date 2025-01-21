use std::path::PathBuf;

use anyhow::{Context, Result};
use btclib::crypto::PublicKey;
use btclib::network::Message;
use btclib::Saveable;
use clap::Parser;
use tokio::net::TcpStream;

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

    println!("Connecting to {} to mine with {public_key:?}", cli.address);

    let mut stream = TcpStream::connect(&cli.address)
        .await
        .context(format!("Failed to connect to server: {}", cli.address))?;

    // Ask the node for work
    println!("requesting work from {}", cli.address);
    let message = Message::FetchTemplate(public_key);
    message.send_async(&mut stream).await?;

    Ok(())
}
