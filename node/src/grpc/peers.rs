use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
use btclib::blockchain::{Block, Tx};
use btclib::Serializable;
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

    /// DashMap<peer addr, grpc client>
    nodes: DashMap<String, NodeApiClient<Channel>>,

    /// DashMap<peer addr, Subscriber>
    subscribers: DashMap<String, Subscriber>,
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

    pub fn add_subscriber(
        &self,
        peer_addr: &str,
        sender: mpsc::Sender<SubscriptionItem>,
    ) -> String {
        let subscriber_id = peer_addr.to_string();
        self.subscribers.insert(
            subscriber_id.clone(),
            Subscriber {
                channel_sender: sender,
            },
        );
        subscriber_id
    }

    pub fn remove_subscriber(&self, peer_addr: &str) {
        println!("removing failed subscriber {}", peer_addr);
        self.subscribers.remove(peer_addr);
    }

    /// Returns number of connected nodes
    pub fn count(&self) -> usize {
        self.nodes.len()
    }

    /// Returns number of subscribers
    pub fn subscriber_count(&self) -> usize {
        self.subscribers.len()
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
            println!("sent DiscoverNodes request to {}", node);

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
                        self.nodes.insert(child_node, client);
                    }
                }
            }
            // do not add itself (it might happen when reconnecting)
            if node != self.listener_addr() {
                println!("adding '{}' to the list of connected nodes", node);
                self.nodes.insert(node.clone(), client);
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

            let client = entry.value_mut();

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
        let client = entry.value_mut();

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
            let block = Block::deserialize(&bytes[..]).context("deserialize block")?;

            let mut blockchain = crate::BLOCKCHAIN.write().await;
            blockchain.add_block(block).context("add new block")?;
            blockchain.rebuild_utxo_set();
        }

        let blockchain = crate::BLOCKCHAIN.read().await;
        println!("block height = {}", blockchain.block_height());

        Ok(())
    }

    pub async fn broadcast(&self, item: SubscriptionItem, skip_addr: Option<&str>) -> Result<()> {
        println!(
            "broadcasting {item} to {} subscribers",
            self.subscribers.len()
        );

        for subscriber in self.subscribers.iter() {
            let peer_addr = subscriber.key();

            // do not send the item back to its originator
            if let Some(skip_addr) = skip_addr {
                if peer_addr == skip_addr {
                    continue;
                }
            }

            subscriber
                .channel_sender
                .send(item.clone())
                .await
                .with_context(|| {
                    format!("sending {item} to subscriber's channel ({})", peer_addr)
                })?
        }

        Ok(())
    }
}

pub async fn subscribe_to_nodes(
    peers: Arc<Peers>,
    tracker: TaskTracker,
    cancel: CancellationToken,
) -> Result<()> {
    println!("subscribing to connected nodes ({})...", peers.count());

    for mut item in peers.nodes.iter_mut() {
        let peer_addr = item.key().clone();
        let client = item.value_mut();

        create_subscription(
            &peer_addr,
            client,
            Arc::clone(&peers),
            tracker.clone(),
            cancel.clone(),
        )
        .await?;
    }

    Ok(())
}

/// Subscribes to peers subscribed to us, and then periodically renews lost connections
pub async fn subscribe_to_subscribers(
    peers: Arc<Peers>,
    tracker: TaskTracker,
    cancel: CancellationToken,
) -> Result<()> {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(15));

    tokio::select! {
        res = async {
            loop {
                interval.tick().await;
                println!("renewing lost connections to subscribers, n_subscribers: {}...", peers.subscriber_count());
                for item in peers.subscribers.iter() {
                    let subscriber_addr = item.key().clone();

                    if peers.nodes.contains_key(&subscriber_addr) {
                        continue;
                    }

                    let addr = format!("http://{subscriber_addr}");
                    let mut client = NodeApiClient::connect(addr.clone())
                    .await
                    .with_context(|| format!("connecting to {subscriber_addr}"))?;

                    create_subscription(
                        &subscriber_addr,
                        &mut client,
                        Arc::clone(&peers),
                        tracker.clone(),
                        cancel.clone(),
                    )
                    .await?;

                    let before = peers.count();
                    peers.nodes.insert(subscriber_addr.clone(), client);
                    let after = peers.count();
                    println!("created or renewed lost connections to subscribers ({})", after-before);
                }
            }
        } => {
            eprintln!("error in subscribe_to_subscribers task: {:?}", res);
            res
        }
        _ = cancel.cancelled() => {
            println!("'periodically subscribe to subscribers' task terminated");
            Ok(())
        }
    }
}

pub async fn create_subscription(
    peer_addr: &str,
    client: &mut NodeApiClient<Channel>,
    peers: Arc<Peers>,
    tracker: TaskTracker,
    cancel: CancellationToken,
) -> Result<()> {
    println!("calling subscribe_for_new_items RPC: {}", peer_addr);
    let my_addr = peers.listener_addr().to_string();

    let mut stream = client
        .subscribe_for_new_items(Request::new(pb::SubscriptionRequest { addr: my_addr }))
        .await
        .context("calling subscribe_for_new_items RPC")?
        .into_inner();

    let peer_addr = peer_addr.to_owned();
    println!("subscription request for new items sent to {peer_addr}");

    let peers = Arc::clone(&peers);
    let cancel_token = cancel.clone();
    tracker.spawn(async move {
            println!("subscribed to {peer_addr}, message listener started, n_peers: {}", peers.count());
            tokio::select! {
                _ = cancel_token.cancelled() => {
                    println!("'subscription for new items' task terminated, {peer_addr}");
                }
                res = async {
                    while let Some(item_response) = stream.message().await? {
                        let item_type = pb::ItemType::try_from(item_response.item_type)?;

                        let item = item_response.item.ok_or(anyhow!("missing item field"))?;
                        let bytes = item.cbor;

                        let item = match item_type {
                            pb::ItemType::Block => {
                                let block = Block::deserialize(&bytes[..]).context("deserialize block")?;
                                println!("> received new block from peer {peer_addr} ({})", block.header.hash());

                                let mut blockchain = crate::BLOCKCHAIN.write().await;
                                if let Err(e) = blockchain.add_block(block.clone()) {
                                    eprintln!("adding new block: {}", e);
                                    continue;
                                }
                                blockchain.rebuild_utxo_set();
                                SubscriptionItem::Block(block)
                            },
                            pb::ItemType::Transaction => {
                                let transaction = Tx::deserialize(&bytes[..]).context("deserialize transaction")?;
                                println!(
                                    "> received new transaction from peer {peer_addr}, adding to mempool ({})",
                                    transaction.hash()
                                );
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

                        peers.broadcast(item, Some(&peer_addr))
                            .await
                            .context(msg)?;
                    }
                    Ok::<_, anyhow::Error>(())
                } => {
                    res.inspect_err(|e| eprintln!("subscription connection to {peer_addr} failed: {}", e.root_cause()))?;
                }
            }
            Ok::<_, anyhow::Error>(())
        });

    Ok(())
}
