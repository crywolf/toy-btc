mod core;

use anyhow::{Context, Result};
use btclib::blockchain::Tx;
use clap::{Parser, Subcommand};
use core::Core;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::time::{self, Duration};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
    /// Path to config file
    #[arg(short, long, value_name = "FILE", default_value = "wallet_config.toml")]
    config: PathBuf,
    /// Node socket address
    #[arg(short, long, value_name = "ADDRESS")]
    node: Option<String>,
}
#[derive(Subcommand)]
enum Commands {
    /// Generate config template
    GenerateConfig {
        /// Path to the produced file
        #[arg(value_name = "FILE")]
        output: PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config_path = cli.config;

    if let Some(cmd) = cli.command {
        match cmd {
            Commands::GenerateConfig { output } => {
                return core::config::generate_config_template(&output)
                    .context("generate config template")
            }
        }
    }

    let mut core = Core::load(config_path.clone())
        .await
        .context("load Core object")?;

    if let Some(node) = cli.node {
        core.config.default_node = node;
    }

    let (tx_sender, tx_receiver) = kanal::bounded(10);
    core.tx_sender = tx_sender.clone_async();

    let core = Arc::new(core);

    tokio::spawn(update_utxos(Arc::clone(&core)));
    tokio::spawn(handle_transactions(
        tx_receiver.clone_async(),
        Arc::clone(&core),
    ));

    run_cli(core).await.context("run cli app")?;

    Ok(())
}

/// Check tbe balance with the node
async fn update_utxos(core: Arc<Core>) {
    let mut interval = time::interval(Duration::from_secs(20));
    loop {
        interval.tick().await;
        if let Err(e) = core.fetch_utxos().await {
            eprintln!("Failed to update UTXOs: {:?}", e);
        }
    }
}

async fn handle_transactions(rx: kanal::AsyncReceiver<Tx>, core: Arc<Core>) {
    while let Ok(transaction) = rx.recv().await {
        if let Err(e) = core.send_transaction(transaction).await {
            eprintln!("Failed to send transaction: {:?}", e);
        }
    }
}

/// Respond to the CLI commands
async fn run_cli(core: Arc<Core>) -> Result<()> {
    loop {
        print!("> ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        let parts = input.split_whitespace().collect::<Vec<_>>();
        if parts.is_empty() {
            continue;
        }
        match parts[0] {
            "balance" => {
                println!("Current balance: {} sats", core.get_balance());
            }
            "send" => {
                if parts.len() != 3 {
                    println!("Usage: send <recipient> <amount>");
                    continue;
                }
                let recipient = parts[1];
                let amount: u64 = parts[2].parse()?;

                let recipient_key = core
                    .config
                    .contacts
                    .iter()
                    .find(|r| r.name == recipient)
                    .ok_or_else(|| anyhow::anyhow!("Recipient not found"))?
                    .load()
                    .context("load recipient key")?
                    .key;

                if let Err(e) = core.fetch_utxos().await {
                    println!("failed to fetch utxos: {e}");
                };

                let transaction = core
                    .create_transaction(&recipient_key, amount)
                    .await
                    .context("create trasaction")?;
                core.send_transaction(transaction)
                    .await
                    .context("send transaction")?;

                println!("Transaction sent successfully");
                core.fetch_utxos().await.context("fetch utxos")?;
            }
            "exit" => break,
            _ => println!("Unknown command"),
        }
    }

    Ok(())
}
