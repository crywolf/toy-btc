use anyhow::{Context, Result};
use btclib::blockchain::Blockchain;
use btclib::Saveable;
use static_init::dynamic;
use tokio::sync::RwLock;
use tokio::time;

#[dynamic]
pub static BLOCKCHAIN: RwLock<Blockchain> = RwLock::new(Blockchain::new());

pub async fn load_from_file(blockchain_file: &str) -> Result<()> {
    let new_blockchain =
        Blockchain::load_from_file(blockchain_file).context("load blockchain from file")?;
    println!(
        "blockchain loaded (height={})",
        new_blockchain.block_height()
    );

    let mut blockchain = BLOCKCHAIN.write().await;
    *blockchain = new_blockchain;

    println!("rebuilding utxo set...");
    blockchain.rebuild_utxo_set();
    println!("utxo set rebuilt");

    println!("checking if target needs to be adjusted...");
    println!("current target: {}", blockchain.target());
    blockchain.try_adjust_target();
    println!("new target: {}", blockchain.target());

    println!("initialization complete");
    Ok(())
}

pub async fn cleanup() {
    let mut interval = time::interval(time::Duration::from_secs(30));
    loop {
        interval.tick().await;
        println!("cleaning the mempool from old transactions");
        let mut blockchain = BLOCKCHAIN.write().await;
        blockchain.cleanup_mempool();
    }
}

pub async fn save(name: String) {
    let mut interval = time::interval(time::Duration::from_secs(15));
    loop {
        interval.tick().await;
        let blockchain = BLOCKCHAIN.read().await;
        println!(
            "saving blockchain (height={}) to drive...",
            blockchain.block_height()
        );
        blockchain.save_to_file(name.clone()).unwrap();
    }
}
