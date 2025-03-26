use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use grpc::peers;
use tokio::signal::unix::{signal, SignalKind};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tonic::transport::Server;

use blockchain::BLOCKCHAIN;

mod args;
mod blockchain;
mod grpc;

#[tokio::main]
async fn main() -> Result<()> {
    let args = args::args();

    let blockchain_file = args.blockchain_file;
    let node_addrs = args.nodes;

    let peers = grpc::peers::Peers::new(&args.host, args.port);

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

    // Start gRPC server
    let listener_addr = peers.listener_addr();
    let addr = listener_addr
        .parse()
        .with_context(|| format!("parsing address {listener_addr}"))?;

    // gRPC Reflection Service
    let reflection = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(grpc::FILE_DESCRIPTOR_SET)
        .build_v1()
        .unwrap();

    println!("---");
    println!("Listening on {} (gRPC)", addr);

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
                println!("graceful shutdown triggered");
                tracker_clone.close();
                token.cancel();
            }
           _= hup.recv() => {
                println!(">>> got signal HUP");
                println!("graceful shutdown triggered");
                tracker_clone.close();
                token.cancel();
            }
            _= term.recv() => {
                println!(">>> got signal TERM");
                println!("graceful shutdown triggered");
                tracker_clone.close();
                token.cancel();
            }
        }
    });

    let peers = Arc::new(peers);

    tracker.spawn(peers::subscribe_to_nodes(
        Arc::clone(&peers),
        tracker.clone(),
        cancel_token.clone(),
    ));

    tracker.spawn(peers::subscribe_to_subscribers(
        Arc::clone(&peers),
        tracker.clone(),
        cancel_token.clone(),
    ));

    tokio::select! {
        // Build and start gRPC server
        res = Server::builder()
            //.layer(remote_addr_extension)
            .add_service(reflection)
            .add_service(grpc::node_api::create_server(Arc::clone(&peers)))
            .add_service(grpc::miner_api::create_server(Arc::clone(&peers)))
            .add_service(grpc::wallet_api::create_server(Arc::clone(&peers)))
            .serve(addr) => {
                res?;
            }
        _ = cancel_token.cancelled() => {
            println!("terminating server");
        }
    }

    println!("gRPC server ended, waiting for tasks to complete...");

    tracker.wait().await;

    println!("all tasks exited");
    println!("node stopped");

    Ok(())
}
