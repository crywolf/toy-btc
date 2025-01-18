use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    crypto::{PublicKey, Signature},
    sha256::Hash,
};

/// Transaction
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Tx {
    pub inputs: Vec<TxInput>,
    pub outputs: Vec<TxOutput>,
}

impl Tx {
    pub fn new(inputs: Vec<TxInput>, outputs: Vec<TxOutput>) -> Self {
        Self { inputs, outputs }
    }

    pub fn hash(&self) -> Hash {
        Hash::hash(&self)
    }
}

/// Transaction input
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TxInput {
    /// Hash of the transaction output, which we are linking into this transaction as input
    pub prev_tx_output_hash: Hash,
    /// ScriptSig - here just signature (w/o script)
    pub signature: Signature,
}

// Transaction output
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TxOutput {
    pub amount: u64,
    /// Generated identifier that helps us ensure that the hash of each transaction output is unique, and can be used to identify it
    pub unique_id: Uuid,
    /// ScriptPubKey - here just compressed public key
    pub pubkey: PublicKey,
}

impl TxOutput {
    pub fn hash(&self) -> Hash {
        Hash::hash(&self)
    }
}
