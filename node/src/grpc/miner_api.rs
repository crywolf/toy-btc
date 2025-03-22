use std::sync::Arc;

use super::log_error;
use super::miner::pb;
use super::miner::pb::miner_api_server::{MinerApi, MinerApiServer};
use super::peers::Peers;
use crate::blockchain::BLOCKCHAIN;

use anyhow::Context;
use btclib::blockchain::{Block, BlockHeader, Tx, TxOutput};
use btclib::crypto::PublicKey;
use btclib::merkle_root::MerkleRoot;
use btclib::sha256::Hash;
use btclib::Saveable;
use chrono::Utc;
use tonic::{Request, Response, Status};
use uuid::Uuid;

pub struct MinerSvc {
    peers: Arc<Peers>,
}

impl MinerSvc {
    pub fn new(peers: Arc<Peers>) -> Self {
        Self { peers }
    }
}

#[tonic::async_trait]
impl MinerApi for MinerSvc {
    /// Ask the node to prepare the optimal block template with the coinbase transaction paying the specified public key
    async fn fetch_template(
        &self,
        request: Request<pb::FetchTemplateRequest>,
    ) -> Result<Response<pb::Template>, Status> {
        let pubkey_bytes = request.into_inner().pubkey;
        let pubkey = PublicKey::load(&pubkey_bytes[..])?;
        println!("gRPC: fetch template called for {:?}", pubkey);

        let blockchain = BLOCKCHAIN.read().await;

        // partialy filled coinbase transaction (without reward)
        let coinbase_tx = Tx {
            inputs: vec![],
            outputs: vec![TxOutput {
                pubkey,
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
        let header = BlockHeader::new(timestamp, nonce, prev_block_hash, empty_merkle_root, target);

        let mut template_block = Block::new(header, transactions);

        let miner_fees = match template_block.calculate_miner_fees(blockchain.utxo_set()) {
            Ok(fees) => fees,
            Err(e) => {
                log_error(&e);
                return Err(Status::internal(e.to_string()));
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

        if let Err(e) =
            template_block.verify_coinbase_transaction(blockchain.block_height(), miner_fees)
        {
            log_error(&e);
            return Err(Status::internal(e.to_string()));
        }

        drop(blockchain);

        // calculate Merkle root
        template_block.header.merkle_root = MerkleRoot::calculate(&template_block.transactions);

        let mut template_bytes = Vec::new();
        template_block.save(&mut template_bytes)?;

        Ok(Response::new(pb::Template {
            cbor: template_bytes,
        }))
    }

    /// Ask the node to validate a block template.
    async fn validate_template(
        &self,
        request: Request<pb::Template>,
    ) -> Result<Response<pb::ValidateTemplateResponse>, Status> {
        let template_bytes = request.into_inner().cbor;
        let block_template = Block::load(&template_bytes[..])?;
        println!(
            "gRPC: validate template called {}",
            block_template.header.hash()
        );

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

        drop(blockchain);

        // is the whole template valid?
        let is_valid = valid_hash && valid_txs;

        Ok(Response::new(pb::ValidateTemplateResponse { is_valid }))
    }

    /// Submit a mined block to a node
    async fn submit_template(
        &self,
        request: Request<pb::Template>,
    ) -> Result<Response<pb::Empty>, Status> {
        let template_bytes = request.into_inner().cbor;
        let block = Block::load(&template_bytes[..])?;

        println!(
            "gRPC: received newly mined template from a miner, {:?}",
            block.header.hash()
        );
        let mut blockchain = BLOCKCHAIN.write().await;

        if let Err(e) = blockchain.add_block(block.clone()) {
            log_error(&e);
            return Err(Status::unknown(e.to_string()));
        }

        blockchain.rebuild_utxo_set();

        println!("block height = {}", blockchain.block_height());

        // broadcast newly mined block to all subscribers
        println!("broadcasting newly mined block to all subscribers");
        if let Err(e) = self
            .peers
            .broadcast(block)
            .await
            .context("broadcasting new block")
        {
            log_error(e);
        }

        Ok(Response::new(pb::Empty {}))
    }
}

pub fn create_server(peers: Arc<Peers>) -> MinerApiServer<MinerSvc> {
    MinerApiServer::new(MinerSvc::new(peers))
}
