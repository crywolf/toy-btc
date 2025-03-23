use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
use btclib::blockchain::{Block, Tx};
use btclib::Saveable;
use dashmap::DashMap;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tonic::transport::Channel;
use tonic::Request;

use super::node::pb;
use super::node::pb::node_api_client::NodeApiClient;
use super::node_api::SubscriptionItem;

pub struct Subscriber {
    channel_sender: mpsc::Sender<SubscriptionItem>,
}

/// Connected peer nodes
pub struct Peers {
    listener_addr: String,

    /// DashMap<conn addr, (grpc client, Option<skip_source_addr>)>
    nodes: DashMap<String, (NodeApiClient<Channel>, Option<String>)>,

    pub subscribers: DashMap<usize, Subscriber>,
}

impl Peers {
    pub fn new(host: &str, port: u16) -> Self {
        Self {
            listener_addr: format!("{host}:{port}"),
            nodes: DashMap::new(),
            subscribers: DashMap::default(),
        }
    }

    pub fn listener_addr(&self) -> &str {
        &self.listener_addr
    }

    pub fn add_subscriber(&self, sender: mpsc::Sender<SubscriptionItem>) -> usize {
        static COUNTER: AtomicUsize = AtomicUsize::new(1);
        let subscriber_id = COUNTER.fetch_add(1, Ordering::Relaxed);
        self.subscribers.insert(
            subscriber_id,
            Subscriber {
                channel_sender: sender,
            },
        );
        subscriber_id
    }

    pub fn remove_subscriber(&self, subscriber_id: usize) {
        println!("removing failed subscriber id={}", subscriber_id);
        self.subscribers.remove(&subscriber_id);
    }

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
        let (client, _) = entry.value_mut();

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

    pub async fn broadcast(&self, item: SubscriptionItem) -> Result<()> {
        println!(
            "broadcasting {item} to {} subscribers",
            self.subscribers.len()
        );

        for subscriber in self.subscribers.iter() {
            let id = subscriber.key();

            subscriber
                .channel_sender
                .send(item.clone())
                .await
                .with_context(|| format!("sending {item} to subscriber's channel (no: {})", id))?
        }

        Ok(())
    }
}

pub async fn subscribe_to_nodes(
    peers: Arc<Peers>,
    tracker: TaskTracker,
    cancel: CancellationToken,
) -> Result<()> {
    println!("subscribing to connected nodes...");

    for mut item in peers.nodes.iter_mut() {
        let node = item.key().clone();
        let (client, _) = item.value_mut();

        let mut stream = client
            .subscribe_for_new_items(Request::new(pb::Empty {}))
            .await
            .context("calling subscribe_for_new_items RPC")?
            .into_inner();

        println!("subscription request for new items sent to {node}");

        let peers = Arc::clone(&peers);
        let cancel_token = cancel.clone();
        tracker.spawn(async move {
            println!("subscribed to {node}");
            tokio::select! {
                _ = cancel_token.cancelled() => {
                    println!("'subscribe_to_nodes for new items' task terminated");
                }
                res = async {
                    while let Some(item_response) = stream.message().await? {
                        let item_type = pb::ItemType::try_from(item_response.item_type)?;

                        let item = item_response.item.ok_or(anyhow!("missing item field"))?;
                        let bytes = item.cbor;

                        let item = match item_type {
                            pb::ItemType::Block => {
                                let block = Block::load(&bytes[..]).context("deserialize block")?;
                                println!("> received new block from peer {:?}", block.header.hash());

                                let mut blockchain = crate::BLOCKCHAIN.write().await;
                                if let Err(e) = blockchain.add_block(block.clone()) {
                                    eprintln!("adding new block: {}", e);
                                    continue;
                                }
                                blockchain.rebuild_utxo_set();
                                SubscriptionItem::Block(block)
                            },
                            pb::ItemType::Transaction => {
                                println!("> new transaction from peer, adding to mempool");
                                let transaction = Tx::load(&bytes[..]).context("deserialize transaction")?;
                                let mut blockchain = crate::BLOCKCHAIN.write().await;
                                if let Err(e) = blockchain.add_to_mempool(transaction.clone()) {
                                    match e {
                                        btclib::error::BtcError::TxAlreadyInMempool(_) => {
                                            eprintln!("transaction rejected: {e}");
                                            continue;
                                        }
                                        _ => {
                                            bail!("transaction rejected: {e}, closing connection");
                                        }
                                    }
                                }
                                SubscriptionItem::Transaction(transaction)
                            },
                            pb::ItemType::Unspecified => bail!("received incorrect item type: {:?}", item_type),
                        };

                        let msg = format!("broadcasting received {item} to all subscribers");
                        println!("{msg}");

                        peers.broadcast(item)
                            .await
                            .context(msg)?;
                    }
                    Ok::<_, anyhow::Error>(())
                } => {
                    res.inspect_err(|e| eprintln!("subscription connection to {node} failed: {}", e.root_cause()))?;
                }
            }
            Ok::<_, anyhow::Error>(())
        });
    }

    Ok(())
}
