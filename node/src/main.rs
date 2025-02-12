mod blockchain;
mod handler;
mod peers;

use std::{path::Path, sync::Arc};

use anyhow::{Context, Result};
use argh::FromArgs;
use blockchain::BLOCKCHAIN;
use tokio::{net::TcpListener, sync::Mutex};

#[derive(FromArgs)]
/// A toy bitcoin node
struct Args {
    #[argh(option)]
    /// port number
    port: u16,
    #[argh(option, default = "String::from(\"./blockchain.cbor\")")]
    /// blockchain file location
    blockchain_file: String,
    #[argh(positional)]
    /// addresses of initial nodes
    nodes: Vec<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Parse command line arguments
    let args: Args = argh::from_env();

    let port = args.port;
    let blockchain_file = args.blockchain_file;
    let node_addrs = args.nodes;

    let peers = peers::Peers::new();

    peers
        .populate_connections(&node_addrs)
        .await
        .context("populate connections")?;
    println!("total amount of known peer nodes: {}", peers.count());

    // Check if the blockchain_file exists
    if Path::new(&blockchain_file).exists() {
        println!("blockchain file exists, loading...");
        blockchain::load_from_file(&blockchain_file)
            .await
            .context("load blockchain")?;
    } else {
        println!("blockchain file does not exist!");
        if !node_addrs.is_empty() {
            if let Some((longest_name, longest_count)) = peers
                .find_longest_chain_node()
                .await
                .context("find node with longest chain")?
            {
                // request the blockchain from the node with the longest blockchain
                peers
                    .download_blockchain(&longest_name, longest_count)
                    .await
                    .with_context(|| format!("download blockchain from {longest_name}"))?;
                println!("blockchain downloaded from {}", longest_name);

                // recalculate utxos
                {
                    let mut blockchain = BLOCKCHAIN.write().await;
                    blockchain.rebuild_utxo_set();
                }
                // try to adjust difficulty
                {
                    let mut blockchain = BLOCKCHAIN.write().await;
                    blockchain.try_adjust_target();
                }
            } else {
                println!("peers do not have any blocks yet, starting with empty blockchain");
            }
        } else {
            println!("no initial nodes provided, starting as a seed node");
        }
    }

    // Start the TCP listener on 0.0.0.0:port
    let addr = format!("0.0.0.0:{}", port);
    let listener = TcpListener::bind(&addr).await?;
    println!("Listening on {}", addr);

    // start a task to periodically cleanup the mempool
    tokio::spawn(blockchain::cleanup());
    // and a task to periodically save the blockchain
    tokio::spawn(blockchain::save(blockchain_file.clone()));

    let nodes = Arc::new(Mutex::new(peers));

    loop {
        let (socket, _) = listener.accept().await?;
        tokio::spawn(handler::handle_connection(Arc::clone(&nodes), socket));
    }
}
