pub mod config;
mod grpc;
mod utxo;

use anyhow::{Context, Result};
use btclib::blockchain::{Tx, TxInput, TxOutput};
use btclib::crypto::{PrivateKey, PublicKey, Signature};
use btclib::network::Message;
use btclib::{Saveable, Serializable};
use config::{Config, FeeType};
use grpc::pb;
use grpc::pb::wallet_api_client::WalletApiClient;
use kanal::Sender;
use serde::{Deserialize, Serialize};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tonic::transport::Channel;
use tonic::Request;
use tracing::{debug, error, info};
use utxo::UtxoStore;

use std::path::PathBuf;

/// Key pair with paths to public and private keys
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Key {
    pub public: PathBuf,
    pub private: PathBuf,
}

/// Loaded key pair with actual public and private keys
#[derive(Clone)]
struct LoadedKey {
    public: PublicKey,
    private: PrivateKey,
}

/// Pair of name and path to the public key used in contact list
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

enum Connection {
    TcpStream(Mutex<TcpStream>),
    GrpcClient(Mutex<WalletApiClient<Channel>>),
}

/// Core functionality of the wallet
pub struct Core {
    pub config: Config,
    utxos: UtxoStore,
    node_conn: Connection,
    pub tx_sender: Sender<Tx>,
}

impl Core {
    /// Create a new `Core` instance
    async fn new(config: Config, utxos: UtxoStore, grpc: bool) -> Result<Self> {
        let (tx_sender, _) = kanal::bounded(10);

        let connection = if grpc {
            println!("Using gRPC connection");
            let addr = format!("http://{}", config.default_node);
            let client = WalletApiClient::connect(addr)
                .await
                .with_context(|| format!("connecting to {}", config.default_node))?;

            Connection::GrpcClient(Mutex::new(client))
        } else {
            let stream = TcpStream::connect(&config.default_node)
                .await
                .with_context(|| format!("connect to node {}", config.default_node))?;

            Connection::TcpStream(Mutex::new(stream))
        };

        Ok(Self {
            node_conn: connection,
            config,
            utxos,
            tx_sender: tx_sender.clone(),
        })
    }

    /// Loads config file and configures `Core` accordingly
    pub async fn load(config_path: PathBuf, node: Option<String>, grpc: bool) -> Result<Self> {
        info!("Loading core from config: {}", config_path.display());
        let mut config = Config::load_from_file(config_path).context("load config from file")?;

        if let Some(node) = node {
            info!("Overriding default node with: {}", node);
            config.default_node = node;
        }

        let mut utxos = UtxoStore::new();

        // Load keys from config
        for key in &config.my_keys {
            debug!("Loading key pair: {:?}", key.public);
            let public = PublicKey::load_from_file(&key.public)
                .with_context(|| format!("load PublicKey from file '{}'", key.public.display()))?;

            let private = PrivateKey::load_from_file(&key.private).with_context(|| {
                format!("load PrivateKey from file '{}'", key.private.display())
            })?;

            utxos.add_key(LoadedKey { public, private });
        }

        Core::new(config, utxos, grpc).await
    }

    /// Returns sender's private key for specified public key
    pub fn my_privkey(&self, pubkey: &PublicKey) -> Result<&PrivateKey> {
        let my_privkey = &self
            .utxos
            .my_keys
            .iter()
            .find(|k| k.public == *pubkey)
            .ok_or(anyhow::anyhow!("Public key not found"))?
            .private;

        Ok(my_privkey)
    }

    /// Fetches UTXOs from the node for all loaded keys
    pub async fn fetch_utxos(&self) -> Result<()> {
        debug!("Fetching UTXOs from node: {}", self.config.default_node);
        for key in &self.utxos.my_keys {
            match &self.node_conn {
                // TCP
                Connection::TcpStream(stream) => {
                    let mut stream_lock = stream.lock().await;

                    Message::FetchUTXOs(key.public.clone())
                        .send_async(&mut *stream_lock)
                        .await
                        .context("send FetchUTXOs message")?;

                    if let Message::UTXOs(utxos) = Message::receive_async(&mut *stream_lock)
                        .await
                        .context("receive message")?
                    {
                        debug!("Received {} UTXOs for key: {:?}", utxos.len(), key.public);
                        // Replace the entire UTXO set for this key
                        self.utxos.utxos.insert(
                            key.public.clone(),
                            utxos
                                .into_iter()
                                .map(|(output, marked)| (marked, output))
                                .collect(),
                        );
                    } else {
                        error!("Unexpected response from node");
                        anyhow::bail!("Unexpected response from node");
                    }
                }
                // GRPC
                Connection::GrpcClient(client) => {
                    let mut pubkey_bytes = Vec::new();
                    key.public.save(&mut pubkey_bytes)?;

                    let response = client
                        .lock()
                        .await
                        .fetch_utxos(Request::new(pb::FetchUtxosRequest {
                            pubkey: pubkey_bytes,
                        }))
                        .await
                        .context("calling fetch_utxos RPC")?;

                    let utxos = response.into_inner().utxos;
                    debug!("Received {} UTXOs for key: {:?}", utxos.len(), key.public);
                    let utxos = utxos
                        .into_iter()
                        .map(|utxo| {
                            let tx_output = TxOutput::load(&utxo.tx_output[..])
                                .context("deserialize transaction output")?;
                            let in_mempool = utxo.in_mempool;
                            Ok::<(bool, TxOutput), anyhow::Error>((in_mempool, tx_output))
                        })
                        .collect::<Result<Vec<_>, _>>()?;

                    // Replace the entire UTXO set for this key
                    self.utxos.utxos.insert(key.public.clone(), utxos);
                }
            }
        }
        info!("UTXOs fetched successfully");
        Ok(())
    }

    /// Sends a transaction to the node
    pub async fn send_transaction(&self, transaction: Tx) -> Result<()> {
        debug!("Sending transaction to node: {}", self.config.default_node);

        match &self.node_conn {
            // TCP
            Connection::TcpStream(stream) => {
                let mut stream_lock = stream.lock().await;
                Message::SubmitTransaction(transaction)
                    .send_async(&mut *stream_lock)
                    .await
                    .context("send SubmitTransaction message")?;
            }
            // GRPC
            Connection::GrpcClient(client) => {
                let mut bytes = Vec::new();
                transaction.save(&mut bytes)?;
                client
                    .lock()
                    .await
                    .submit_transaction(pb::Transaction { cbor: bytes })
                    .await
                    .context("calling submit_transaction RPC")?;
            }
        }

        info!("Transaction sent successfully");
        Ok(())
    }

    /// Prepares and sends a transaction asynchronously.
    /// We are not actually sending a transaction via Rust’s async here, but rather,
    /// we are sending it into the asynchronous part of the application.
    pub fn send_transaction_async(&self, recipient: &str, amount: u64) -> Result<()> {
        info!("Preparing to send {} asats to {}", amount, recipient);
        let recipient_pubkey = self
            .config
            .recipient_pubkey(recipient)
            .context("get recipient's pubkey")?;

        let transaction = self.create_transaction(&recipient_pubkey, amount)?;
        debug!("Sending transaction asynchronously");
        self.tx_sender
            .send(transaction)
            .context("send transaction to async part of the app")?;
        Ok(())
    }

    /// Creates new transaction
    pub fn create_transaction(&self, recipient_pubkey: &PublicKey, amount: u64) -> Result<Tx> {
        debug!(
            "Creating transaction for {} sats to {:?}",
            amount, recipient_pubkey
        );
        let fee = self.calculate_fee(amount);
        let total_amount = amount + fee;

        let mut inputs = Vec::new();
        let mut input_sum = 0;

        // iterate over all my pubkeys
        for entry in self.utxos.utxos.iter() {
            let my_pubkey = entry.key();
            let utxos = entry.value();

            let my_privkey = &self
                .my_privkey(my_pubkey)
                .context("get sender's private key")?;

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
            error!(
                "Insufficient funds: have {} sats, need {} sats",
                input_sum, total_amount
            );
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

        info!("Transaction created successfully");
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
        let fee = match self.config.fee_config.fee_type {
            FeeType::Fixed => self.config.fee_config.value as u64,
            FeeType::Percent => (amount as f64 * self.config.fee_config.value / 100.0) as u64,
        };

        debug!("Calculated fee: {} sats", fee);
        fee
    }
}
