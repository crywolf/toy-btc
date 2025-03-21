use std::path::Path;

use anyhow::{Context, Result};
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

    let reflection = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(grpc::FILE_DESCRIPTOR_SET)
        .build_v1()
        .unwrap();

    println!("---");
    println!("Listening on {} (gRPC)", addr);

    Server::builder()
        .add_service(reflection)
        .add_service(grpc::node_api::create_server(peers))
        .add_service(grpc::miner_api::create_server())
        .serve(addr)
        .await?;

    Ok(())
}
