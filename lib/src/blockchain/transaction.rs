use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    crypto::{PublicKey, Signature},
    sha256::Hash,
    Saveable,
};

/// Transaction
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
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

/// Save and load expecting CBOR from ciborium as format
impl Saveable for Tx {
    fn load<R: std::io::Read>(reader: R) -> std::io::Result<Self> {
        ciborium::de::from_reader(reader).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Failed to deserialize Transaction",
            )
        })
    }

    fn save<W: std::io::Write>(&self, writer: W) -> std::io::Result<()> {
        ciborium::ser::into_writer(self, writer).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Failed to serialize Transaction",
            )
        })
    }
}
/// Transaction input
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TxInput {
    /// Hash of the transaction output, which we are linking into this transaction as input
    pub prev_tx_output_hash: Hash,
    /// ScriptSig - here just signature (w/o script)
    pub signature: Signature,
}

// Transaction output
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
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
