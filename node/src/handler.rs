use std::fmt::Debug;

use btclib::blockchain::{Block, BlockHeader, Tx, TxOutput};
use btclib::merkle_root::MerkleRoot;
use btclib::network::Message;
use btclib::sha256::Hash;
use chrono::Utc;
use tokio::net::TcpStream;
use uuid::Uuid;

use crate::blockchain::BLOCKCHAIN;
use crate::peers::NODES;

pub async fn handle_connection(mut stream: TcpStream) {
    loop {
        // read a message from the socket
        let message = match Message::receive_async(&mut stream).await {
            Ok(message) => message,
            Err(e) => {
                println!("invalid message from peer: {e}, closing connection");
                return;
            }
        };

        use btclib::network::Message::*;
        match message {
            UTXOs(_) | Template(_) | Difference(_) | TemplateValidity(_) | NodeList(_) => {
                println!("I am neither a miner nor a wallet! Terminating connection.");
                return;
            }
            FetchBlock(height) => {
                let blockchain = BLOCKCHAIN.read().await;
                let Some(block) = blockchain.blocks().nth(height).cloned() else {
                    return;
                };

                let _ = Message::NewBlock(block)
                    .send_async(&mut stream)
                    .await
                    .map_err(log_error);
            }
            DiscoverNodes => {
                let nodes = NODES.iter().map(|x| x.key().clone()).collect::<Vec<_>>();

                let _ = NodeList(nodes)
                    .send_async(&mut stream)
                    .await
                    .map_err(log_error);
            }
            AskDifference(height) => {
                let blockchain = BLOCKCHAIN.read().await;
                let count = blockchain.block_height() as i32 - height as i32;

                let _ = Difference(count)
                    .send_async(&mut stream)
                    .await
                    .map_err(log_error);
            }

            FetchUTXOs(pubkey) => {
                println!("received request to fetch UTXOs");
                let blockchain = BLOCKCHAIN.read().await;
                let utxos = blockchain
                    .utxo_set_for_pubkey(pubkey)
                    .map(|(_, (marked, txout))| (txout.clone(), *marked))
                    .collect::<Vec<_>>();

                let _ = UTXOs(utxos)
                    .send_async(&mut stream)
                    .await
                    .map_err(log_error);
            }
            NewBlock(block) => {
                println!("new block from a peer");
                let mut blockchain = BLOCKCHAIN.write().await;
                if let Err(e) = blockchain.add_block(block) {
                    println!("block rejected: {e}");
                }
            }
            NewTransaction(tx) => {
                println!("new transaction from peer, adding to mempool");
                let mut blockchain = BLOCKCHAIN.write().await;
                if let Err(e) = blockchain.add_to_mempool(tx) {
                    println!("transaction rejected: {e}, closing connection");
                    return;
                }
                // TODO send this new tx to other connected peers + prevent sending txs back to the source node (dtto with blocks)
            }
            ValidateTemplate(block_template) => {
                let blockchain = BLOCKCHAIN.read().await;
                // does the template point to the last block?
                let valid_hash = blockchain
                    .blocks()
                    .last()
                    .map(|last| last.header.hash() == block_template.header.prev_block_hash)
                    .unwrap_or(false);

                // are all transactions in the template present in mempool?
                let valid_txs = block_template
                    .transactions
                    .iter()
                    .all(|tx| blockchain.mempool().iter().any(|(_, m_tx)| tx == m_tx));

                // is the whole template valid?
                let valid = valid_hash && valid_txs;

                let _ = Message::TemplateValidity(valid)
                    .send_async(&mut stream)
                    .await
                    .map_err(log_error);
            }
            SubmitTemplate(block) => {
                println!("received newly mined template from a miner");
                let mut blockchain = BLOCKCHAIN.write().await;
                if let Err(e) = blockchain.add_block(block.clone()) {
                    println!("mined block rejected: {e}, closing connection with bad miner");
                    return;
                }

                blockchain.rebuild_utxo_set();

                // send new block to all known nodes
                let nodes = NODES.iter().map(|n| n.key().clone()).collect::<Vec<_>>();
                for node in nodes {
                    if let Some(mut node_stream) = NODES.get_mut(&node) {
                        let _ = Message::NewBlock(block.clone())
                            .send_async(&mut *node_stream)
                            .await
                            .map_err(|e| {
                                println!("failed to send block to {}", node);
                                log_error(e)
                            });
                    }
                }
                println!("new block sent to all connected nodes")
            }
            SubmitTransaction(tx) => {
                println!("new transaction was submitted");
                let mut blockchain = BLOCKCHAIN.write().await;
                if let Err(e) = blockchain.add_to_mempool(tx.clone()) {
                    println!("transaction rejected: {e}, closing connection");
                }
                println!("added transaction to mempool");
                // send the transaction to all known nodes
                let nodes = NODES.iter().map(|n| n.key().clone()).collect::<Vec<_>>();
                for node in nodes {
                    if let Some(mut node_stream) = NODES.get_mut(&node) {
                        let _ = Message::NewTransaction(tx.clone())
                            .send_async(&mut *node_stream)
                            .await
                            .map_err(|e| {
                                println!("failed to send transaction to {}", node);
                                log_error(e)
                            });
                    }
                }
                println!("transaction sent to all connected nodes")
            }
            FetchTemplate(public_key) => {
                let blockchain = BLOCKCHAIN.read().await;

                // partialy filled coinbase transaction (without reward)
                let coinbase_tx = Tx {
                    inputs: vec![],
                    outputs: vec![TxOutput {
                        pubkey: public_key,
                        unique_id: Uuid::new_v4(),
                        amount: 0,
                    }],
                };

                // get transactions from the mempool
                let mut transactions = vec![coinbase_tx];
                transactions.extend(
                    blockchain
                        .mempool()
                        .iter()
                        .take(btclib::BLOCK_TRANSACTION_CAP)
                        .map(|(_, tx)| tx.clone()),
                );

                // prepare block header
                let empty_merkle_root = MerkleRoot::calculate(&[]);

                let prev_block_hash = blockchain
                    .blocks()
                    .last()
                    .map(|b| b.header.hash())
                    .unwrap_or(Hash::zero());
                let target = blockchain.target();
                let timestamp = Utc::now();
                let nonce = 0;
                let header =
                    BlockHeader::new(timestamp, nonce, prev_block_hash, empty_merkle_root, target);

                let mut template_block = Block::new(header, transactions);

                let miner_fees = match template_block.calculate_miner_fees(blockchain.utxo_set()) {
                    Ok(fees) => fees,
                    Err(e) => {
                        log_error(e);
                        return;
                    }
                };
                let reward = blockchain.calculate_block_reward(); // block subsidy

                // update coinbase transaction with reward and validate it
                template_block
                    .transactions
                    .get_mut(0)
                    .expect("coinbase tx has at least one input")
                    .outputs
                    .get_mut(0)
                    .expect("coinbase tx has at least one output")
                    .amount = reward + miner_fees;

                if template_block
                    .verify_coinbase_transaction(blockchain.block_height() + 1, miner_fees)
                    .map_err(log_error)
                    .is_err()
                {
                    return;
                }

                // calculate Merkle root
                template_block.header.merkle_root =
                    MerkleRoot::calculate(&template_block.transactions);

                let _ = Template(template_block)
                    .send_async(&mut stream)
                    .await
                    .map_err(log_error);
            }
        }
    }
}

fn log_error(e: impl Debug) {
    eprintln!("Error: {:?}", e);
}
