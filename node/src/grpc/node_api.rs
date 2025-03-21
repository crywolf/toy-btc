use std::sync::Arc;

use super::node::pb;
use super::node::pb::node_api_server::{NodeApi, NodeApiServer};
use super::peers::Peers;
use crate::blockchain::BLOCKCHAIN;
use anyhow::Result;
use btclib::Saveable;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};

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
impl NodeApi for NodeSvc {
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

pub fn create_server(peers: Peers) -> NodeApiServer<NodeSvc> {
    NodeApiServer::new(NodeSvc::new(peers))
}
