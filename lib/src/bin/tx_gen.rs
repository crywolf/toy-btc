use btclib::{
    blockchain::{Tx, TxOutput},
    crypto::PrivateKey,
    Saveable,
};
use uuid::Uuid;

fn main() {
    let path = if let Some(arg) = std::env::args().nth(1) {
        arg
    } else {
        eprintln!("Usage: tx_gen <tx_file>");
        std::process::exit(1);
    };

    let private_key = PrivateKey::new_key();

    let inputs = vec![];
    let outputs = vec![TxOutput {
        unique_id: Uuid::new_v4(),
        amount: btclib::INITIAL_REWARD * 10u64.pow(8),
        pubkey: private_key.public_key(),
    }];

    let tx = Tx::new(inputs, outputs);

    tx.save_to_file(path).expect("Failed to save transaction");
}
