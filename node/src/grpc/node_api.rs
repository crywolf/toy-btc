use std::sync::Arc;

use super::log_error;
use super::node::pb;
use super::node::pb::node_api_server::{NodeApi, NodeApiServer};
use super::peers::Peers;
use crate::blockchain::BLOCKCHAIN;
use anyhow::{Context, Result};
use btclib::Saveable;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};

pub struct NodeSvc {
    peers: Arc<Peers>,
}

impl NodeSvc {
    pub fn new(peers: Arc<Peers>) -> Self {
        Self { peers }
    }
}

#[tonic::async_trait]
impl NodeApi for NodeSvc {
    /// Ask a node to report all the other nodes it knows about
    async fn discover_nodes(
        &self,
        _request: Request<pb::Empty>,
    ) -> Result<Response<pb::NodeList>, Status> {
        let nodes = self.peers.addresses();
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
                if let Err(e) = block.save(&mut bytes).context("serialize block") {
                    log_error(&e);
                    if let Err(e) = tx
                        .send(Err(Status::internal(e.to_string())))
                        .await
                        .context("send error to channel")
                    {
                        log_error(&e);
                        return;
                    }
                    continue;
                }
                let block = pb::Block { cbor: bytes };
                if let Err(e) = tx.send(Ok(block)).await.context("send block to channel") {
                    log_error(e);
                }
            }
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }

    type SubscribeForNewBlocksStream = ReceiverStream<Result<pb::Block, Status>>;

    /// Ask a node to send stream of newly received blocks
    async fn subscribe_for_new_blocks(
        &self,
        _request: Request<pb::Empty>,
    ) -> Result<Response<Self::SubscribeForNewBlocksStream>, Status> {
        // store subscriber's sender part of the channel
        let (subscriber_tx, mut subscriber_rx) = mpsc::channel(1);
        let subscriber_id = self.peers.add_block_subscriber(subscriber_tx);
        println!(
            "subscription for new blocks received, n_subscribers: {}",
            self.peers.block_subscribers.len()
        );

        let (stream_tx, stream_rx) = mpsc::channel(1);

        let peers = Arc::clone(&self.peers);
        tokio::spawn(async move {
            // when subscriber's receiver got a new block, send it to the stream sender
            while let Some(block) = subscriber_rx.recv().await {
                println!("sending block to subscriber {}", subscriber_id);
                let mut bytes = Vec::new();
                if let Err(e) = block.save(&mut bytes).context("serialize block") {
                    log_error(&e);
                    continue;
                }

                let block = pb::Block { cbor: bytes };

                if let Err(e) = stream_tx.send(Ok(block)).await {
                    eprintln!(
                        "failed to send block to subscriber {}: {}",
                        subscriber_id, e
                    );
                    // remove failed subscriber
                    peers.remove_block_subscriber(subscriber_id);
                }
            }
        });

        Ok(Response::new(ReceiverStream::new(stream_rx)))
    }
}

pub fn create_server(peers: Arc<Peers>) -> NodeApiServer<NodeSvc> {
    NodeApiServer::new(NodeSvc::new(peers))
}
