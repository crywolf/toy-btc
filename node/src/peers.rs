use anyhow::{Context, Result};
use btclib::network::Message;
use dashmap::DashMap;
use static_init::dynamic;
use tokio::net::TcpStream;

// Node pool
#[dynamic]
pub static NODES: DashMap<String, TcpStream> = DashMap::new();

pub async fn populate_connections(nodes: &[String]) -> Result<()> {
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
                    NODES.insert(child_node, new_stream);
                }
            }
            _ => {
                println!("unexpected message from {}", node);
            }
        }

        NODES.insert(node.clone(), stream);
    }

    Ok(())
}

pub async fn find_longest_chain_node() -> Result<Option<(String, u32)>> {
    println!("finding nodes with the highest blockchain length...");
    let mut longest_name = String::new();
    let mut longest_count = 0;

    let all_nodes = NODES.iter().map(|x| x.key().clone()).collect::<Vec<_>>();
    for node in all_nodes {
        println!("asking {} for blockchain length", node);
        let mut stream = NODES.get_mut(&node).context("missing node in the pool")?;

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
                println!("unexpected message from {}: {:?}", node, e);
            }
        }
    }

    if longest_count == 0 {
        return Ok(None); // all the peer nodes does not have any blocks yet
    }

    Ok(Some((longest_name, longest_count as u32)))
}

pub async fn download_blockchain(node: &str, count: u32) -> Result<()> {
    dbg!(node);
    let mut stream = NODES.get_mut(node).expect("node name exists");

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
                println!("unexpected message from {}", node);
            }
        }
    }

    Ok(())
}
