use std::collections::HashSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::{BtcError, Result};
use crate::merkle_root::MerkleRoot;
use crate::sha256::Hash;
use crate::{Saveable, Serializable, HALVING_INTERVAL, INITIAL_REWARD, U256};

use super::transaction::Tx;
use super::UtxoSet;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Block {
    pub header: BlockHeader,
    pub transactions: Vec<Tx>,
}

impl Block {
    pub fn new(header: BlockHeader, transactions: Vec<Tx>) -> Self {
        Self {
            header,
            transactions,
        }
    }

    pub fn hash(&self) -> Hash {
        Hash::hash(&self)
    }

    /// Verifies all transactions in the block
    pub fn verify_transactions(
        &self,
        predicted_block_height: u64,
        utxo_set: &UtxoSet,
    ) -> Result<()> {
        // reject completely empty blocks (w/o even coinbase transaction)
        if self.transactions.is_empty() {
            return Err(BtcError::InvalidBlock("No transactions".to_string()));
        }

        let mut miner_fees = 0;
        let mut spent_outputs = HashSet::new(); // for double-spend check

        // skip the coinbase transaction and verify each transaction's inputs and outputs
        for tx in self.transactions.iter().skip(1) {
            let mut input_amount = 0;
            let mut output_amount = 0;

            for input in &tx.inputs {
                let prev_output = if let Some(prev_output) = utxo_set
                    .get(&input.prev_tx_output_hash)
                    .map(|(_, output)| output)
                {
                    prev_output
                } else {
                    return Err(BtcError::InvalidTransaction(format!(
                        "Spending output that does not exist: {}",
                        input.prev_tx_output_hash
                    )));
                };

                // prevent same-block double-spending
                if spent_outputs.contains(&input.prev_tx_output_hash) {
                    return Err(BtcError::InvalidTransaction(format!(
                        "Double spend of the output: {}",
                        prev_output.unique_id
                    )));
                }

                // check if the signature is valid
                if !input
                    .signature
                    .verify(&input.prev_tx_output_hash, &prev_output.pubkey)
                {
                    return Err(BtcError::InvalidSignature);
                }

                input_amount += prev_output.amount;
                spent_outputs.insert(input.prev_tx_output_hash);
            }

            for output in &tx.outputs {
                output_amount += output.amount;
            }

            if input_amount < output_amount {
                // difference between input and output value is the mining fee
                return Err(BtcError::InvalidTransaction(
                    "Spending more than available".to_string(),
                ));
            }

            miner_fees += input_amount - output_amount;
        }

        self.verify_coinbase_transaction(predicted_block_height, miner_fees)?;

        Ok(())
    }

    /// Verifies coinbase transaction
    pub fn verify_coinbase_transaction(
        &self,
        predicted_block_height: u64,
        miner_fees: u64,
    ) -> Result<()> {
        let coinbase_tx = &self
            .transactions
            .first()
            .expect("coinbase tx is the first transaction in the block");

        if !coinbase_tx.inputs.is_empty() {
            return Err(BtcError::InvalidTransaction(
                "Coinbase transaction cannot have inputs".to_string(),
            ));
        }
        if coinbase_tx.outputs.is_empty() {
            return Err(BtcError::InvalidTransaction(
                "Coinbase transaction must have outputs".to_string(),
            ));
        }

        let block_reward = crate::btc_to_sats(INITIAL_REWARD as f64)
            / 2u64.pow((predicted_block_height / HALVING_INTERVAL) as u32);

        let total_coinbase_amount: u64 = coinbase_tx.outputs.iter().map(|o| o.amount).sum();

        if total_coinbase_amount != block_reward + miner_fees {
            return Err(BtcError::InvalidTransaction(
                "Outputs in coinbase tx do not equal to block reward + fees".to_string(),
            ));
        }

        Ok(())
    }

    /// Returns miner fees. It assumes that transactions are valid. (Used when creating new block template.)
    pub fn calculate_miner_fees(&self, utxo_set: &UtxoSet) -> Result<u64> {
        let mut miner_fees = 0;

        // skip the coinbase transaction
        for tx in self.transactions.iter().skip(1) {
            let mut input_amount = 0;
            let mut output_amount = 0;

            for input in &tx.inputs {
                let prev_output = if let Some(prev_output) = utxo_set
                    .get(&input.prev_tx_output_hash)
                    .map(|(_, output)| output)
                {
                    prev_output
                } else {
                    return Err(BtcError::InvalidTransaction(format!(
                        "Spending output that does not exist: {}",
                        input.prev_tx_output_hash
                    )));
                };

                input_amount += prev_output.amount;
            }

            for output in &tx.outputs {
                output_amount += output.amount;
            }

            miner_fees += input_amount - output_amount;
        }

        Ok(miner_fees)
    }
}

/// Save and load expecting CBOR from ciborium as format
impl Serializable for Block {
    fn deserialize<R: std::io::Read>(reader: R) -> std::io::Result<Self> {
        ciborium::de::from_reader(reader).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Failed to deserialize Block",
            )
        })
    }

    fn serialize<W: std::io::Write>(&self, writer: W) -> std::io::Result<()> {
        ciborium::ser::into_writer(self, writer).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "Failed to serialize Block")
        })
    }
}

impl Saveable for Block {}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BlockHeader {
    /// Timestamp of the block
    pub timestamp: DateTime<Utc>,
    /// Nonce used to mine the block
    pub nonce: u64,
    /// Hash of the previous block
    pub prev_block_hash: Hash,
    /// Merkle root of the block's transactions
    pub merkle_root: MerkleRoot,
    /// A number which has to be higher than the hash of this block for it to be considered valid
    pub target: U256,
}

impl BlockHeader {
    pub fn new(
        timestamp: DateTime<Utc>,
        nonce: u64,
        prev_block_hash: Hash,
        merkle_root: MerkleRoot,
        target: U256,
    ) -> Self {
        Self {
            timestamp,
            nonce,
            prev_block_hash,
            merkle_root,
            target,
        }
    }

    pub fn mine(&mut self, steps: usize) -> bool {
        // if the block already matches the target, return early
        if self.hash().matches_target(self.target) {
            return true;
        }

        // The reason why we only do a finite number of steps at a time is because we may want to interrupt the mining if we receive an update
        // from the network that we should work on a new block (because a new block has been found in the meantime)
        for _ in 0..steps {
            if let Some(new_nonce) = self.nonce.checked_add(1) {
                self.nonce = new_nonce;
            } else {
                // we ran out of the available nonces -> update the timestamp
                self.nonce = 0;
                self.timestamp = Utc::now()
            }
            if self.hash().matches_target(self.target) {
                // we found the correct nonce!
                return true;
            }
        }
        false
    }

    pub fn hash(&self) -> Hash {
        Hash::hash(&self)
    }
}
