use btclib::blockchain::{Block, BlockHeader, Tx, TxOutput};
use btclib::crypto::PrivateKey;
use btclib::merkle_root::MerkleRoot;
use btclib::sha256::Hash;
use btclib::Saveable;
use chrono::Utc;
use uuid::Uuid;

fn main() {
    let path = if let Some(arg) = std::env::args().nth(1) {
        arg
    } else {
        eprintln!("Usage: block_gen <block_file>");
        std::process::exit(1);
    };

    let private_key = PrivateKey::new_key();

    let transactions = vec![Tx::new(
        vec![],
        vec![TxOutput {
            unique_id: Uuid::new_v4(),
            amount: btclib::btc_to_sats(btclib::INITIAL_REWARD),
            pubkey: private_key.public_key(),
        }],
    )];

    let merkle_root = MerkleRoot::calculate(&transactions);
    let header = BlockHeader::new(Utc::now(), 0, Hash::zero(), merkle_root, btclib::MIN_TARGET);

    let block = Block::new(header, transactions);

    block.save_to_file(path).expect("Failed to save block");
}
