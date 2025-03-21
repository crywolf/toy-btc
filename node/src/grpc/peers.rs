use anyhow::{Context, Result};
use btclib::blockchain::Block;
use btclib::Saveable;
use dashmap::DashMap;
use tonic::transport::Channel;
use tonic::Request;

use super::node::pb;
use super::node::pb::node_api_client::NodeApiClient;

/// Connected peer nodes
pub struct Peers {
    listener_addr: String,

    /// DashMap<target_addr, (grpc client,  Option<skip_source_addr>)>
    nodes: DashMap<String, (NodeApiClient<Channel>, Option<String>)>,
}

impl Peers {
    pub fn new(host: &str, port: u16) -> Self {
        Self {
            listener_addr: format!("{host}:{port}"),
            nodes: DashMap::new(),
        }
    }

    pub fn listener_addr(&self) -> &str {
        &self.listener_addr
    }

    // pub fn add(&self, addr: &str, client: NodeClient<Channel>, skip_source_addr: &str) {
    //     self.nodes.insert(
    //         addr.to_string(),
    //         (client, Some(skip_source_addr.to_string())),
    //     );
    // }

    /// Returns number of connected nodes
    pub fn count(&self) -> usize {
        self.nodes.len()
    }

    /// Returns socket addresses of connected nodes
    pub fn addresses(&self) -> Vec<String> {
        self.nodes
            .iter()
            .map(|x| x.key().clone())
            .collect::<Vec<_>>()
    }

    /// Discovers and connects to other nodes
    pub async fn populate_connections(&self, nodes: &[String]) -> Result<()> {
        println!("trying to connect to other nodes...");

        for node in nodes {
            let addr = format!("http://{node}");
            let mut client = NodeApiClient::connect(addr)
                .await
                .with_context(|| format!("connecting to {node}"))?;

            let request = Request::new(pb::Empty {});
            let response = client
                .discover_nodes(request)
                .await
                .context("calling discover_nodes RPC")?;
            println!("sent DiscoverNodes to {}", node);

            let child_nodes = response.into_inner().nodes;

            println!(
                "received NodeList from {} with {} items",
                node,
                child_nodes.len()
            );

            for child_node in child_nodes {
                // do not add itself (it might happen when reconnecting)
                if child_node != self.listener_addr() {
                    // do not connect to already connected node
                    if !self.nodes.contains_key(&child_node) {
                        let addr = format!("http://{child_node}");
                        let client = NodeApiClient::connect(addr)
                            .await
                            .with_context(|| format!("connecting to {child_node}"))?;

                        println!("adding '{}' to the list of connected nodes", child_node);
                        self.nodes.insert(child_node, (client, None));
                    }
                }
            }
            // do not add itself (it might happen when reconnecting)
            if node != self.listener_addr() {
                println!("adding '{}' to the list of connected nodes", node);
                self.nodes.insert(node.clone(), (client, None));
            }
        }

        Ok(())
    }

    pub async fn find_longest_chain_node(&self) -> Result<Option<(String, u64)>> {
        println!("finding nodes with the highest blockchain length...");
        let mut longest_name = String::new();
        let mut longest_count = 0;

        let all_nodes = self
            .nodes
            .iter()
            .map(|x| x.key().clone())
            .collect::<Vec<_>>();

        for node in all_nodes {
            println!("asking {} for blockchain length", node);

            let mut entry = self
                .nodes
                .get_mut(&node)
                .context("missing node in the pool")?;

            let (ref mut client, _) = entry.value_mut();

            let blockchain = crate::BLOCKCHAIN.read().await;
            let height = blockchain.block_height();
            drop(blockchain);

            let response = client
                .ask_difference(Request::new(pb::DifferenceRequest { height }))
                .await
                .context("calling ask_difference RPC")?;

            let count = response.into_inner().n_blocks;

            println!("received Difference {count} from {node}");
            if count > longest_count {
                println!("new longest blockchain: {} blocks from {node}", count);
                longest_count = count;
                longest_name = node;
            }
        }

        if longest_count == 0 {
            return Ok(None); // all the peer nodes do not have any blocks yet
        }

        Ok(Some((longest_name, longest_count as u64)))
    }

    /// Request `need` missing blocks from the specified `node`
    pub async fn synchronize_blockchain(&self, node: &str, need: u64) -> Result<()> {
        let mut entry = self.nodes.get_mut(node).expect("node name exists");
        let (ref mut client, _) = entry.value_mut();

        let blockchain = crate::BLOCKCHAIN.read().await;
        let have = blockchain.block_height();
        drop(blockchain);

        println!("have: {} blocks, need: {} blocks", have, need);
        println!("downloading {} missing blocks...", need);

        let mut stream = client
            .fetch_blocks(Request::new(pb::FetchBlockIntervalRequest {
                start: have,
                n_blocks: need,
            }))
            .await
            .context("calling fetch_blocks RPC")?
            .into_inner();

        while let Some(block) = stream.message().await? {
            let bytes = block.cbor;
            let block = Block::load(&bytes[..]).context("deserialize block")?;
            println!("> fetched block {:?}", block.header.hash());

            let mut blockchain = crate::BLOCKCHAIN.write().await;
            blockchain.add_block(block).context("add new block")?;
            blockchain.rebuild_utxo_set();
        }

        let blockchain = crate::BLOCKCHAIN.read().await;
        println!("block height = {}", blockchain.block_height());

        Ok(())
    }
}
