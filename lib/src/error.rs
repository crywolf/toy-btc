use thiserror::Error;

pub type Result<T> = std::result::Result<T, BtcError>;

#[derive(Error, Debug)]
pub enum BtcError {
    #[error("Invalid transaction: {0}")]
    InvalidTransaction(String),
    #[error("Invalid block: {0}")]
    InvalidBlock(String),
    #[error("Invalid block header")]
    InvalidBlockHeader,
    #[error("Invalid transaction input")]
    InvalidTxInput,
    #[error("Invalid transaction output")]
    InvalidTxOutput,
    #[error("Invalid Merkle root")]
    InvalidMerkleRoot,
    #[error("Invalid hash")]
    InvalidHash,
    #[error("Invalid signature")]
    InvalidSignature,
    #[error("Invalid public key")]
    InvalidPublicKey,
    #[error("Invalid private key")]
    InvalidPrivateKey,
    #[error("Transaction is already in mempool: {0}")]
    TxAlreadyInMempool(crate::sha256::Hash),
}
