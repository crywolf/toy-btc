mod block;
mod transaction;

pub use block::{Block, BlockHeader};
pub use transaction::{Tx, TxInput, TxOutput};

use std::collections::{HashMap, HashSet};

use bigdecimal::BigDecimal;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    crypto::PublicKey,
    error::{BtcError, Result},
    merkle_root::MerkleRoot,
    sha256::Hash,
    Saveable, DIFFICULTY_UPDATE_INTERVAL, HALVING_INTERVAL, IDEAL_BLOCK_TIME, INITIAL_REWARD,
    MAX_MEMPOOL_TRANSACTION_AGE, MIN_TARGET, U256,
};

/// UTXO set represented as HashMap where
/// key is 'TxOutput hash' and value is tuple of ('referenced by a tx in mempool' flag, TxOutput)
type UtxoSet = HashMap<Hash, (bool, TxOutput)>;

#[derive(Default, Serialize, Deserialize, Clone, Debug)]
pub struct Blockchain {
    blocks: Vec<Block>,
    utxo_set: UtxoSet,
    target: U256,
    #[serde(default, skip_serializing)]
    mempool: Vec<(DateTime<Utc>, Tx)>, // (Time of insertion, Tx)
}

impl Blockchain {
    pub fn new() -> Self {
        Self {
            target: MIN_TARGET,
            ..Default::default()
        }
    }

    /// Returns current block height
    pub fn block_height(&self) -> u64 {
        self.blocks.len() as u64
    }

    /// Returns current UTXO set
    pub fn utxo_set(&self) -> &UtxoSet {
        &self.utxo_set
    }

    /// Returns an iterator over UTXOs that belong to specified public key
    pub fn utxo_set_for_pubkey(
        &self,
        pubkey: PublicKey,
    ) -> impl Iterator<Item = (&Hash, &(bool, TxOutput))> {
        self.utxo_set
            .iter()
            .filter(move |(_, (_, txout))| txout.pubkey == pubkey)
    }

    /// Returns current difficulty target
    pub fn target(&self) -> U256 {
        self.target
    }

    /// Returns the mempool content with transactions sorted by fee size
    pub fn mempool(&self) -> &[(DateTime<Utc>, Tx)] {
        &self.mempool
    }

    /// Returns an iterator over all `Block`s
    pub fn blocks(&self) -> impl Iterator<Item = &Block> {
        self.blocks.iter()
    }

    /// Validates and adds new block to the blockchain
    pub fn add_block(&mut self, block: Block) -> Result<()> {
        // check if the block is valid

        if self.blocks.is_empty() {
            // if this is the first block, check if the
            // block's prev_block_hash is all zeroes
            if block.header.prev_block_hash != Hash::zero() {
                return Err(BtcError::InvalidBlock(
                    "First block must have previous block hash set to zero".to_string(),
                ));
            }
        } else {
            // if this is not the first block, check if the
            // block's prev_block_hash is the hash of the last block
            let last_block = self.blocks.last().expect("Previous block must exist");

            if block.header.prev_block_hash != last_block.header.hash() {
                return Err(BtcError::InvalidBlock(
                    "Previous block hash mismatch".to_string(),
                ));
            }

            // check if the block's hash is less than the target
            if !block.header.hash().matches_target(last_block.header.target) {
                return Err(BtcError::InvalidBlock(
                    "Block hash does not match target".to_string(),
                ));
            }

            // check if the block's Merkle root is correct
            if block.header.merkle_root != MerkleRoot::calculate(&block.transactions) {
                return Err(BtcError::InvalidMerkleRoot);
            }

            // check if the block's timestamp is after the last block's timestamp
            if !block.header.timestamp.gt(&last_block.header.timestamp) {
                return Err(BtcError::InvalidBlock(
                    "Block timestamp is not after the previous block's timestamp".to_string(),
                ));
            }
        }
        // Verify all transactions in the block
        block.verify_transactions(self.block_height(), &self.utxo_set)?;

        // Remove transactions from mempool that are now in the block
        let block_tx_hashes: HashSet<_> = block.transactions.iter().map(|tx| tx.hash()).collect();
        self.mempool
            .retain(|(_, tx)| !block_tx_hashes.contains(&tx.hash()));

        self.blocks.push(block);

        self.try_adjust_target();

        Ok(())
    }

    /// Rebuilds UTXO set from the blockchain
    pub fn rebuild_utxo_set(&mut self) {
        for block in &self.blocks {
            for tx in &block.transactions {
                // remove tx output if we see an input that spends it
                for input in &tx.inputs {
                    self.utxo_set.remove(&input.prev_tx_output_hash);
                }
                // add tx outputs
                for output in &tx.outputs {
                    self.utxo_set.insert(output.hash(), (false, output.clone()));
                }
            }
        }
    }

    /// Try to adjust the target of the blockchain
    pub fn try_adjust_target(&mut self) {
        if self.blocks.is_empty() {
            return;
        }
        // is this the correct block to do the adjustment?
        if self.blocks.len() % DIFFICULTY_UPDATE_INTERVAL as usize != 0 {
            return;
        }

        // measure the time it took to mine the last DIFFICULTY_UPDATE_INTERVAL blocks
        let start_time = self.blocks[self.blocks.len() - DIFFICULTY_UPDATE_INTERVAL as usize]
            .header
            .timestamp;
        let end_time = self
            .blocks
            .last()
            .expect("blockchain is not empty")
            .header
            .timestamp;
        let time_diff = end_time - start_time;

        // convert time_diff to seconds
        let time_diff_seconds = time_diff.num_seconds();
        // calculate the ideal number of seconds
        let target_seconds = IDEAL_BLOCK_TIME * DIFFICULTY_UPDATE_INTERVAL;

        // formula: NewTarget = OldTarget * (ActualTime / IdealTime)
        // Using BigDecimal to make the entire calculation in terms of big floating point numbers, and then convert back to U256,
        // which is the fastest and most versatile type to store the target in.
        let new_target = BigDecimal::parse_bytes(self.target.to_string().as_bytes(), 10)
            .expect("should be valid decimal number")
            * (BigDecimal::from(time_diff_seconds) / BigDecimal::from(target_seconds));
        let new_target_str = new_target
            .to_string()
            .split('.')
            .next()
            .expect("should be valid string with a decimal point")
            .to_owned();
        let new_target: U256 =
            U256::from_str_radix(&new_target_str, 10).expect("should be valid string");

        // adjust the difficulty by no more than a factor of 4x in either direction
        let new_target = if new_target < self.target / 4 {
            self.target / 4
        } else if new_target > self.target * 4 {
            self.target * 4
        } else {
            new_target
        };
        // if the new target is more than the minimum target set it to the minimum target
        self.target = new_target.min(MIN_TARGET);
    }

    /// Validates and adds new transaction to the mempool
    pub fn add_to_mempool(&mut self, tx: Tx) -> Result<()> {
        // validate transaction before insertion

        // all inputs in tx must match known UTXOs, and must be unique (to prevent double-spend within a single tx)
        let mut seen_inputs = HashSet::new();
        for input in &tx.inputs {
            if !self.utxo_set.contains_key(&input.prev_tx_output_hash) {
                return Err(BtcError::InvalidTransaction(format!(
                    "Mempool: Spending output that does not exist: {} in tx: {}",
                    input.prev_tx_output_hash,
                    tx.hash()
                )));
            }
            if seen_inputs.contains(&input.prev_tx_output_hash) {
                return Err(BtcError::InvalidTransaction(format!(
                    "Mempool: Double spend of the output: {} within a single tx: {}",
                    input.prev_tx_output_hash,
                    tx.hash()
                )));
            }
            seen_inputs.insert(input.prev_tx_output_hash);
        }

        // To replace older transactions with newer transactions that reference the same inputs (to prevent a potential double-spending problem):
        //   Check if any of the UTXOs have the bool mark set to true and if so, find the transaction
        //   that references them in the mempool, remove it, and set all the UTXOs it references to false
        // TODO: remove the transaction with smaller fee, not the older one
        for input in &tx.inputs {
            if let Some((true, _)) = self.utxo_set.get(&input.prev_tx_output_hash) {
                // this UTXO is marked 'true' => it has already been marked by another transaction in the mempool,
                // find the transaction in the mempool that references the UTXO we are trying to reference
                let referencing_tx = self.mempool.iter().enumerate().find(|(_, (_, tx))| {
                    tx.outputs
                        .iter()
                        .any(|output| output.hash() == input.prev_tx_output_hash)
                });
                // if we have found such transaction, unmark all of its UTXOs
                if let Some((idx, (_, referencing_tx))) = referencing_tx {
                    for input in &referencing_tx.inputs {
                        // set all UTXOs from this transaction to false
                        self.utxo_set
                            .entry(input.prev_tx_output_hash)
                            .and_modify(|(marked, _)| {
                                *marked = false;
                            });
                    }
                    // remove the transaction from the mempool
                    self.mempool.remove(idx);
                } else {
                    // if, somehow, there is no matching transaction in the mempool, set this UTXO to false (this should not happen)
                    self.utxo_set
                        .entry(input.prev_tx_output_hash)
                        .and_modify(|(marked, _)| {
                            *marked = false;
                        });
                }
            }
        }

        // all inputs must be lower than all outputs
        let tx_inputs: u64 = tx
            .inputs
            .iter()
            .map(|input| {
                self.utxo_set
                    .get(&input.prev_tx_output_hash)
                    .expect("prevout hash should be present")
                    .1
                    .amount
            })
            .sum();
        let tx_outputs = tx.outputs.iter().map(|output| output.amount).sum();
        if tx_inputs < tx_outputs {
            return Err(BtcError::InvalidTransaction(
                "Mempool: Spending more than available".to_string(),
            ));
        }

        // mark the UTXOs as used
        for input in &tx.inputs {
            self.utxo_set
                .entry(input.prev_tx_output_hash)
                .and_modify(|(marked, _)| {
                    *marked = true;
                });
        }

        // insert tx in the mempool
        self.mempool.push((Utc::now(), tx));

        // sort mempool by fee size
        self.mempool.sort_by_key(|(_, tx)| {
            let tx_inputs: u64 = tx
                .inputs
                .iter()
                .map(|input| {
                    self.utxo_set
                        .get(&input.prev_tx_output_hash)
                        .expect("prevout hash should be present")
                        .1
                        .amount
                })
                .sum();
            let tx_outputs: u64 = tx.outputs.iter().map(|output| output.amount).sum();
            tx_inputs - tx_outputs // miner fee
        });

        Ok(())
    }

    /// Calculates block subsidy
    pub fn calculate_block_reward(&self) -> u64 {
        let block_height = self.block_height();
        let halvings = block_height / HALVING_INTERVAL;
        crate::btc_to_sats(INITIAL_REWARD) >> halvings
    }

    /// Removes transactions older than `MAX_MEMPOOL_TRANSACTION_AGE`
    pub fn cleanup_mempool(&mut self) {
        let now = Utc::now();
        let mut utxos_to_unmark: Vec<Hash> = vec![];
        // retain only transactions that are not too old
        self.mempool.retain(|(timestamp, tx)| {
            if now - *timestamp > chrono::Duration::seconds(MAX_MEMPOOL_TRANSACTION_AGE as i64) {
                // push all UTXOs to unmark to the vector so we can unmark them later
                utxos_to_unmark.extend(tx.inputs.iter().map(|input| input.prev_tx_output_hash));
                false
            } else {
                true
            }
        });
        // unmark all of the UTXOs
        for hash in utxos_to_unmark {
            self.utxo_set.entry(hash).and_modify(|(marked, _)| {
                *marked = false;
            });
        }
    }
}

/// Save and load expecting CBOR from ciborium as format
impl Saveable for Blockchain {
    fn load<R: std::io::Read>(reader: R) -> std::io::Result<Self> {
        ciborium::de::from_reader(reader).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Failed to deserialize Blockchain",
            )
        })
    }

    fn save<W: std::io::Write>(&self, writer: W) -> std::io::Result<()> {
        ciborium::ser::into_writer(self, writer).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Failed to serialize Blockchain",
            )
        })
    }
}
