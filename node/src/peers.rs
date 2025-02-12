use anyhow::{anyhow, Context, Result};
use btclib::network::Message;
use dashmap::DashMap;
use tokio::net::TcpStream;

/// Connected peer nodes
pub struct Peers {
    nodes: DashMap<String, TcpStream>,
}

impl Peers {
    pub fn new() -> Self {
        Self {
            nodes: DashMap::new(),
        }
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

    /// Sends message to all connected nodes and returns error containing information about all encountered failures if any
    pub async fn broadcast(&self, msg: &Message) -> Result<()> {
        let mut errors = Vec::new();
        let mut remove = Vec::new();

        for mut item in self.nodes.iter_mut() {
            let node = item.key().clone();
            let stream = item.value_mut();

            println!("broadcasting {msg} to node {node}");

            if let Err(err) = msg
                .send_async(stream)
                .await
                .with_context(|| format!("sending {msg} to node {node}"))
            {
                errors.push(format!("{}: {}", err, err.root_cause()));
                remove.push(node);
            } else {
                println!("{msg} sent to node {node}");
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
                        self.nodes.insert(child_node, new_stream);
                    }
                    self.nodes.insert(node.clone(), stream);
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
            let mut stream = self
                .nodes
                .get_mut(&node)
                .context("missing node in the pool")?;

            let message = Message::AskDifference(0);
            message
                .send_async(&mut *stream)
                .await
                .context("send AskDifference message")?;
            println!("sent AskDifference to {}", node);

            let message = Message::receive_async(&mut *stream)
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
        let mut stream = self.nodes.get_mut(node).expect("node name exists");

        for i in 0..count as usize {
            let message = Message::FetchBlock(i);
            message
                .send_async(&mut *stream)
                .await
                .with_context(|| format!("send FetchBlock({i}) message"))?;

            let message = Message::receive_async(&mut *stream)
                .await
                .context("receive message")?;

            match message {
                Message::NewBlock(block) => {
                    let mut blockchain = crate::BLOCKCHAIN.write().await;
                    blockchain.add_block(block).context("add new block")?;
                }
                _ => {
                    eprintln!("unexpected message from {}", node);
                }
            }
        }

        Ok(())
    }
}
