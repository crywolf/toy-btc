use std::net::ToSocketAddrs;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use grpc::peers::{self, synchronize_blockchain};
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

    let listener_addr = peers.listener_addr();
    let socket_addr = listener_addr
        .to_socket_addrs()
        .with_context(|| format!("converting {listener_addr} to socket address"))?
        .next()
        .expect("is a valid socket address");

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

    let peers = Arc::new(peers);

    if !node_addrs.is_empty() {
        synchronize_blockchain(Arc::clone(&peers))
            .await
            .context("initial blockchain synchronization")?
    } else if !Path::new(&blockchain_file).exists() {
        println!("no initial nodes provided, starting as a seed node");
    }

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

    tracker.spawn(peers::periodically_synchronize_blockchain(
        Arc::clone(&peers),
        cancel_token.clone(),
    ));

    // gRPC Reflection Service
    let reflection = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(grpc::FILE_DESCRIPTOR_SET)
        .build_v1()
        .unwrap();

    // Start gRPC server
    println!("---");
    println!("Listening on {} (gRPC)", socket_addr);

    tokio::select! {
        // Build and start gRPC server
        res = Server::builder()
            .add_service(reflection)
            .add_service(grpc::node_api::create_server(Arc::clone(&peers)))
            .add_service(grpc::miner_api::create_server(Arc::clone(&peers)))
            .add_service(grpc::wallet_api::create_server(Arc::clone(&peers)))
            .serve(socket_addr) => {
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
