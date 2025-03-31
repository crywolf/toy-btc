mod core;
mod logging;
mod tasks;
mod ui;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use core::Core;
use cursive::views::TextContent;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::time::{self, Duration};
use tracing::{debug, info};
use ui::big_mode_btc;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
    /// Start CLI version of the wallet
    #[arg(long)]
    cli: bool,
    /// Path to config file
    #[arg(short, long, value_name = "FILE", default_value = "wallet_config.toml")]
    config: PathBuf,
    /// Use gRPC connection
    #[arg(long)]
    grpc: bool,
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
    logging::setup_tracing().context("setup tracing")?;
    logging::setup_panic_hook();
    info!("Starting wallet application");

    let cli = Cli::parse();
    let config_path = cli.config;

    if let Some(cmd) = cli.command {
        match cmd {
            Commands::GenerateConfig { output } => {
                debug!("Generating dummy config at: {:?}", output);
                return core::config::generate_config_template(&output)
                    .context("generate config template");
            }
        }
    }

    info!("Loading config from: {:?}", config_path);

    let mut core = Core::load(config_path.clone(), cli.node, cli.grpc)
        .await
        .context("load Core object")?;

    let (tx_sender, tx_receiver) = kanal::bounded(10);
    core.tx_sender = tx_sender;

    let core = Arc::new(core);

    if cli.cli {
        info!("Starting CLI version");
        info!("Starting background tasks");
        tokio::spawn(update_utxos(Arc::clone(&core)));

        return run_cli(core).await.context("run cli app");
    }

    info!("Starting TUI version");

    let balance_content = TextContent::new(big_mode_btc(&core));
    info!("Starting background tasks");
    tokio::select! {
        _ = tasks::ui_task(Arc::clone(&core), balance_content.clone()).await => (),
        _ = tasks::update_utxos(Arc::clone(&core)).await => (),
        _ = tasks::handle_transactions(tx_receiver.clone_async(), Arc::clone(&core)).await => (),
        _ = tasks::update_balance_display(Arc::clone(&core), balance_content).await => (),
    }
    info!("Application shutting down");

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
                let sats = core.get_balance();
                let btc = btclib::sats_to_btc(sats);
                println!("Current balance: {} sats ({} btc)", sats, btc);
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
                    .recipient_pubkey(recipient)
                    .context("get recipient's pubkey")?;

                if let Err(e) = core.fetch_utxos().await {
                    println!("failed to fetch utxos: {e}");
                };

                let transaction = core
                    .create_transaction(&recipient_key, amount)
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
