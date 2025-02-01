pub mod config;
mod utxo;

use anyhow::{Context, Result};
use btclib::blockchain::{Tx, TxInput, TxOutput};
use btclib::crypto::{PrivateKey, PublicKey, Signature};
use btclib::network::Message;
use btclib::Saveable;
use config::{Config, FeeType};
use kanal::AsyncSender;
use serde::{Deserialize, Serialize};
use tokio::net::TcpStream;
use utxo::UtxoStore;

use std::path::PathBuf;

/// Key pair
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Key {
    public: PathBuf,
    private: PathBuf,
}

#[derive(Clone)]
struct LoadedKey {
    public: PublicKey,
    private: PrivateKey,
}

/// Pair of name and public key used in contact list
#[derive(Serialize, Deserialize, Clone)]
pub struct Recipient {
    /// Recipient's name
    pub name: String,
    /// Path to the file with public key
    pub key_path: PathBuf,
}

/// Recipient with public key loaded from file
#[derive(Clone)]
pub struct LoadedRecipient {
    #[allow(unused)]
    pub name: String,
    pub key: PublicKey,
}

impl Recipient {
    /// Load public key from file and initialize `Recipient`
    pub fn load(&self) -> Result<LoadedRecipient> {
        let key = PublicKey::load_from_file(&self.key_path)
            .with_context(|| format!("load PublicKey from file '{}'", &self.key_path.display()))?;

        Ok(LoadedRecipient {
            name: self.name.clone(),
            key,
        })
    }
}

pub struct Core {
    pub config: Config,
    utxos: UtxoStore,
    node_conn: tokio::sync::Mutex<TcpStream>,
    pub tx_sender: AsyncSender<Tx>,
}

impl Core {
    async fn new(config: Config, utxos: UtxoStore) -> Result<Self> {
        let (tx_sender, _) = kanal::bounded(10);

        let stream = TcpStream::connect(&config.default_node)
            .await
            .with_context(|| format!("connect to default node {}", config.default_node))?;

        Ok(Self {
            node_conn: tokio::sync::Mutex::new(stream),
            config,
            utxos,
            tx_sender: tx_sender.clone_async(),
        })
    }

    /// Loads config file and configures `Core` accordingly
    pub async fn load(config_path: PathBuf) -> Result<Self> {
        let config = Config::load_from_file(config_path).context("load config from file")?;

        let mut utxos = UtxoStore::new();

        // Load keys from config
        for key in &config.my_keys {
            let public = PublicKey::load_from_file(&key.public)
                .with_context(|| format!("load PublicKey from file '{}'", key.public.display()))?;

            let private = PrivateKey::load_from_file(&key.private).with_context(|| {
                format!("load PrivateKey from file '{}'", key.private.display())
            })?;

            utxos.add_key(LoadedKey { public, private });
        }

        Core::new(config, utxos).await
    }

    pub async fn fetch_utxos(&self) -> Result<()> {
        for key in &self.utxos.my_keys {
            let mut stream_lock = self.node_conn.lock().await;

            Message::FetchUTXOs(key.public.clone())
                .send_async(&mut *stream_lock)
                .await
                .context("send FetchUTXOs message")?;

            if let Message::UTXOs(utxos) = Message::receive_async(&mut *stream_lock)
                .await
                .context("receive message")?
            {
                // Replace the entire UTXO set for this key
                self.utxos.utxos.insert(
                    key.public.clone(),
                    utxos
                        .into_iter()
                        .map(|(output, marked)| (marked, output))
                        .collect(),
                );
            } else {
                anyhow::bail!("Unexpected response from node");
            }
        }
        Ok(())
    }

    pub async fn send_transaction(&self, transaction: Tx) -> Result<()> {
        let mut stream_lock = self.node_conn.lock().await;
        Message::SubmitTransaction(transaction)
            .send_async(&mut *stream_lock)
            .await
            .context("send SubmitTransaction message")?;
        Ok(())
    }

    pub async fn create_transaction(
        &self,
        recipient_pubkey: &PublicKey,
        amount: u64,
    ) -> Result<Tx> {
        let fee = self.calculate_fee(amount);
        let total_amount = amount + fee;

        let mut inputs = Vec::new();
        let mut input_sum = 0;

        // iterate over all my pubkeys
        for entry in self.utxos.utxos.iter() {
            let my_pubkey = entry.key();
            let utxos = entry.value();

            let my_privkey = &self
                .utxos
                .my_keys
                .iter()
                .find(|k| k.public == *my_pubkey)
                .unwrap()
                .private;

            // iterate over UTXOs for one pubkey
            for (marked, utxo) in utxos.iter() {
                if *marked {
                    continue; // skip marked UTXOs
                }
                inputs.push(TxInput {
                    prev_tx_output_hash: utxo.hash(),
                    signature: Signature::sign_output(&utxo.hash(), my_privkey),
                });
                input_sum += utxo.amount;

                if input_sum >= total_amount {
                    break;
                }
            }

            if input_sum >= total_amount {
                break;
            }
        }

        if input_sum < total_amount {
            anyhow::bail!("Insufficient funds");
        }

        let mut outputs = vec![TxOutput {
            amount,
            unique_id: uuid::Uuid::new_v4(),
            pubkey: recipient_pubkey.clone(),
        }];

        if input_sum > total_amount {
            // change
            outputs.push(TxOutput {
                amount: input_sum - total_amount,
                unique_id: uuid::Uuid::new_v4(),
                pubkey: self.utxos.my_keys[0].public.clone(),
            });
        }

        Ok(Tx::new(inputs, outputs))
    }

    pub fn get_balance(&self) -> u64 {
        self.utxos
            .utxos
            .iter()
            .flat_map(|entry| entry.value().clone())
            .map(|(_, output)| output.amount)
            .sum()
    }

    fn calculate_fee(&self, amount: u64) -> u64 {
        match self.config.fee_config.fee_type {
            FeeType::Fixed => self.config.fee_config.value as u64,
            FeeType::Percent => (amount as f64 * self.config.fee_config.value / 100.0) as u64,
        }
    }
}
