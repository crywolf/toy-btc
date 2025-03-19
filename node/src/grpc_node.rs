use std::{path::Path, sync::Arc};

use anyhow::{Context, Result};
use blockchain::BLOCKCHAIN;
use btclib::Saveable;
use grpc::peers::Peers;
use pb::node_server::{Node, NodeServer};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{transport::Server, Request, Response, Status};

pub mod pb {
    tonic::include_proto!("node");

    pub(crate) const FILE_DESCRIPTOR_SET: &[u8] =
        tonic::include_file_descriptor_set!("reflection_descriptor");
}

mod args;
mod blockchain;
mod grpc;

pub struct NodeSvc {
    peers: Arc<tokio::sync::RwLock<Peers>>,
}

impl NodeSvc {
    pub fn new(peers: Peers) -> Self {
        Self {
            peers: Arc::new(tokio::sync::RwLock::new(peers)),
        }
    }
}

#[tonic::async_trait]
impl Node for NodeSvc {
    /// Ask a node to report all the other nodes it knows about
    async fn discover_nodes(
        &self,
        _request: Request<pb::Empty>,
    ) -> Result<Response<pb::NodeList>, Status> {
        let nodes = self.peers.read().await.addresses();
        println!("sending list of all known nodes ({})", nodes.len());
        Ok(Response::new(pb::NodeList { nodes }))
    }

    /// Ask a node what is the highest block it knows about in comparison to the local blockchain
    async fn ask_difference(
        &self,
        request: Request<pb::DifferenceRequest>,
    ) -> Result<Response<pb::DifferenceResponse>, Status> {
        let height = request.into_inner().height;
        let blockchain = BLOCKCHAIN.read().await;
        let n_blocks = blockchain.block_height() as i64 - height as i64;

        Ok(Response::new(pb::DifferenceResponse { n_blocks }))
    }

    type FetchBlocksStream = ReceiverStream<Result<pb::Block, Status>>;

    /// Ask a node to send stream of blocks starting from the specified height
    async fn fetch_blocks(
        &self,
        request: Request<pb::FetchBlockIntervalRequest>,
    ) -> Result<Response<Self::FetchBlocksStream>, Status> {
        let request = request.into_inner();
        let start = request.start as usize;
        let n_blocks = request.n_blocks as usize;

        let (tx, rx) = mpsc::channel(4);

        tokio::spawn(async move {
            let blockchain = BLOCKCHAIN.read().await;
            let blocks = blockchain.blocks().skip(start).take(n_blocks);
            for block in blocks {
                let mut bytes = Vec::new();
                block.save(&mut bytes).expect("failed to serialize block");
                let block = pb::Block { cbor: bytes };
                tx.send(Ok(block)).await.expect("failed to send block");
            }
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }
}

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
        .register_encoded_file_descriptor_set(pb::FILE_DESCRIPTOR_SET)
        .build_v1()
        .unwrap();

    let node_svc = NodeSvc::new(peers);

    println!("---");
    println!("Listening on {} (gRPC)", addr);

    Server::builder()
        .add_service(reflection)
        .add_service(NodeServer::new(node_svc))
        .serve(addr)
        .await?;

    Ok(())
}
