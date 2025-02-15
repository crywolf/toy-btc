use std::net::SocketAddr;

use anyhow::{anyhow, Context, Result};
use btclib::network::Message;
use dashmap::DashMap;
use tokio::net::TcpStream;

/// Connected peer nodes
pub struct Peers {
    listener_addr: String,

    /// DashMap<target_addr, (TcpStream>,  Option<skip_source_addr>)>
    nodes: DashMap<String, (TcpStream, Option<String>)>,
}

impl Peers {
    pub fn new(port: u16) -> Self {
        Self {
            listener_addr: format!("localhost:{}", port),
            nodes: DashMap::new(),
        }
    }

    pub fn listener_addr(&self) -> &str {
        &self.listener_addr
    }

    pub fn add(&self, addr: &str, stream: TcpStream, skip_source_addr: &str) {
        self.nodes.insert(
            addr.to_string(),
            (stream, Some(skip_source_addr.to_string())),
        );
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

    pub fn update_skip_addr(&mut self, peer_addr: &str, skip_source_addr: &str) {
        self.nodes
            .entry(peer_addr.to_string())
            .and_modify(|(_, skip)| *skip = Some(skip_source_addr.to_string()));
    }

    /// Sends message to all connected nodes (skips optional `source_addr`).
    /// Returns error containing information about all encountered failures if any
    pub async fn broadcast(&self, msg: &Message, source_addr: Option<&SocketAddr>) -> Result<()> {
        let mut errors = Vec::new();
        let mut remove = Vec::new();

        for mut item in self.nodes.iter_mut() {
            let node = item.key().clone();
            let (stream, skip_addr) = item.value_mut();

            // do not send the message back to the source
            if let Some(source_addr) = source_addr {
                let source_addr = &source_addr.to_string().replace("127.0.0.1", "localhost");

                if let Some(skip_addr) = skip_addr {
                    if source_addr == skip_addr {
                        continue;
                    }
                }
            }

            println!("broadcasting {msg} to node {node}");

            if let Err(err) = msg
                .send_async(stream)
                .await
                .with_context(|| format!("sending {msg} to node {node}"))
            {
                errors.push(format!("{}: {}", err, err.root_cause()));
                remove.push(node);
            } else {
                println!("{msg} succesfully sent to node {node}");
            }
        }

        for node in remove {
            println!("removing failed node {node} from the connected nodes list");
            self.nodes.remove(&node);
        }

        if !errors.is_empty() {
            return Err(anyhow!(errors.join("\n       ")));
        }

        Ok(())
    }

    /// Sends subscription message to all connected nodes
    pub async fn subscribe_to_nodes(&self) -> Result<()> {
        for mut item in self.nodes.iter_mut() {
            let node = item.key().clone();
            let (stream, _) = item.value_mut();

            let message = Message::Subscribe(self.listener_addr.clone());
            println!("<-- sending {message:?} to {node}");
            message
                .send_async(stream)
                .await
                .context("send Subscribe message")?;
        }
        Ok(())
    }

    /// Discovers and connects to other nodes
    pub async fn populate_connections(&self, nodes: &[String]) -> Result<()> {
        println!("trying to connect to other nodes...");

        for node in nodes {
            println!("connecting to {}", node);
            let mut stream = TcpStream::connect(&node).await?;

            // Ask connected node to report all the other nodes it knows about
            let message = Message::DiscoverNodes;
            message
                .send_async(&mut stream)
                .await
                .context("send DiscoverNodes message")?;
            println!("sent DiscoverNodes to {}", node);

            let message = Message::receive_async(&mut stream)
                .await
                .context("receive message")?;

            match message {
                Message::NodeList(child_nodes) => {
                    println!("received NodeList from {}", node);

                    for child_node in child_nodes {
                        println!("adding node {}", child_node);
                        let new_stream = TcpStream::connect(&child_node).await?;
                        self.nodes.insert(child_node, (new_stream, None));
                    }
                    self.nodes.insert(node.clone(), (stream, None));
                }
                _ => {
                    eprintln!("unexpected message from {}", node);
                }
            }
        }

        Ok(())
    }

    pub async fn find_longest_chain_node(&self) -> Result<Option<(String, u32)>> {
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

            let (ref mut stream, _) = entry.value_mut();

            let message = Message::AskDifference(0);
            message
                .send_async(stream)
                .await
                .context("send AskDifference message")?;
            println!("sent AskDifference to {}", node);

            let message = Message::receive_async(stream)
                .await
                .context("receive message")?;

            match message {
                Message::Difference(count) => {
                    println!("received Difference {count} from {node}");
                    if count > longest_count {
                        println!("new longest blockchain: {} blocks from {node}", count);
                        longest_count = count;
                        longest_name = node;
                    }
                }
                e => {
                    eprintln!("unexpected message from {}: {:?}", node, e);
                }
            }
        }

        if longest_count == 0 {
            return Ok(None); // all the peer nodes do not have any blocks yet
        }

        Ok(Some((longest_name, longest_count as u32)))
    }

    pub async fn download_blockchain(&self, node: &str, count: u32) -> Result<()> {
        let mut entry = self.nodes.get_mut(node).expect("node name exists");
        let (ref mut stream, _) = entry.value_mut();

        for i in 0..count as usize {
            let message = Message::FetchBlock(i);
            message
                .send_async(stream)
                .await
                .with_context(|| format!("send FetchBlock({i}) message"))?;

            let message = Message::receive_async(stream)
                .await
                .context("receive message")?;

            match message {
                Message::NewBlock(block) => {
                    let mut blockchain = crate::BLOCKCHAIN.write().await;
                    blockchain.add_block(block).context("add new block")?;
                    blockchain.rebuild_utxo_set();
                }
                _ => {
                    eprintln!("unexpected message from {}", node);
                }
            }
        }

        Ok(())
    }
}
