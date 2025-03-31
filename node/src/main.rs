mod args;
mod blockchain;
mod handler;
mod peers;

use std::{path::Path, sync::Arc};

use anyhow::{Context, Result};
use blockchain::BLOCKCHAIN;
use tokio::net::TcpListener;
use tokio::signal::unix::{signal, SignalKind};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

#[tokio::main]
async fn main() -> Result<()> {
    // Parse command line arguments
    let args = args::args();

    let blockchain_file = args.blockchain_file;
    let node_addrs = args.nodes;

    let peers = peers::Peers::new(&args.host, args.port);

    println!("-");
    peers
        .populate_connections(&node_addrs)
        .await
        .context("populate connections")?;
    println!("total amount of known peer nodes: {}", peers.count());
    println!("--");

    // Check if the blockchain_file exists
    if Path::new(&blockchain_file).exists() {
        println!("blockchain file exists, loading...");
        blockchain::load_from_file(&blockchain_file)
            .await
            .context("load blockchain")?;
    } else {
        println!("blockchain file does not exist!");
    }

    if !node_addrs.is_empty() {
        if let Some((longest_name, longest_count)) = peers
            .find_longest_chain_node()
            .await
            .context("find node with longest chain")?
        {
            println!(
                "found node with longest chain: {}, {}",
                longest_name, longest_count
            );
            // request missing blocks from the node with the longest blockchain
            peers
                .synchronize_blockchain(&longest_name, longest_count)
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
            println!("no longer blockchain found, we are up to date");
        }
    } else if !Path::new(&blockchain_file).exists() {
        println!("no initial nodes provided, starting as a seed node");
    }

    // Start the TCP listener on 0.0.0.0:port
    let listener_addr = peers.listener_addr();
    let listener = TcpListener::bind(listener_addr)
        .await
        .with_context(|| format!("bind listener to addr {listener_addr}"))?;
    println!("---");
    println!("Listening on {}", listener_addr);

    // Graceful shutdown
    let token = CancellationToken::new();
    let cancel_token = token.clone();
    let tracker = TaskTracker::new();

    // start a task to periodically cleanup the mempool
    tracker.spawn(blockchain::cleanup(cancel_token.clone()));
    // and a task to periodically save the blockchain
    tracker.spawn(blockchain::save(
        blockchain_file.clone(),
        cancel_token.clone(),
    ));

    let tracker_clone = tracker.clone();
    tokio::spawn(async move {
        let mut hup = signal(SignalKind::hangup()).unwrap();
        let mut term = signal(SignalKind::terminate()).unwrap();
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                println!("\n>>> ctrl-c received!");
                tracker_clone.close();
                token.cancel();
            }
           _= hup.recv() => {
                println!(">>> got signal HUP");
                tracker_clone.close();
                token.cancel();
            }
            _= term.recv() => {
                println!(">>> got signal TERM");
                tracker_clone.close();
                token.cancel();
            }
        }
    });

    let nodes = Arc::new(peers);

    let nodes_clone = Arc::clone(&nodes);
    tracker.spawn(async move { nodes_clone.subscribe_to_nodes().await });

    // Main connection loop
    tokio::select! {
        _ = cancel_token.cancelled() => {
            println!("graceful shutdown triggered");
            println!("terminating accept loop");
        }
        res = async {
            loop {
                let (socket, _) = listener.accept().await?;
                tracker.spawn(handler::handle_connection(Arc::clone(&nodes), socket, cancel_token.clone()));
            }
            // help the rust type inferencer out
            #[allow(unreachable_code)]
            Ok::<_, anyhow::Error>(())
        } => {
            res?;
        }
    }

    println!("main accept loop ended, waiting for tasks to complete...");

    tracker.wait().await;

    println!("all tasks exited");
    println!("node stopped");

    Ok(())
}
