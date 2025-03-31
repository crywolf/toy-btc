use anyhow::Result;
use btclib::crypto::PublicKey;
use btclib::Saveable;
use grpc::miner::Miner;
use tokio::signal::unix::{signal, SignalKind};
use tokio_util::sync::CancellationToken;

mod cli;
mod grpc;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = cli::parse();

    let public_key = PublicKey::load_from_file(&cli.public_key_file)
        .map_err(|e| anyhow::anyhow!("Error reading public key: {}", e))?;

    // Graceful shutdown
    let token = CancellationToken::new();
    let cancel_token = token.clone();
    tokio::spawn(async move {
        let mut hup = signal(SignalKind::hangup()).unwrap();
        let mut term = signal(SignalKind::terminate()).unwrap();
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                println!("\n>>> ctrl-c received!");
                token.cancel();
            }
           _= hup.recv() => {
                println!(">>> got signal HUP");
                token.cancel();
            }
            _= term.recv() => {
                println!(">>> got signal TERM");
                token.cancel();
            }
        }
    });

    let mut miner = Miner::new(cli.address, public_key).await?;
    miner.run(cancel_token).await
}
