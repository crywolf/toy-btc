use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use anyhow::{Context, Result};
use btclib::blockchain::Block;
use btclib::crypto::PublicKey;
use btclib::Serializable;
use pb::miner_api_client::MinerApiClient;
use tokio::time::{interval, Duration};
use tokio_util::sync::CancellationToken;
use tonic::transport::Channel;
use tonic::Request;

pub mod pb {
    tonic::include_proto!("miner");
}

pub struct Miner {
    public_key: PublicKey,
    client: MinerApiClient<Channel>,
    current_template: Arc<Mutex<Option<Block>>>,
    mining: Arc<AtomicBool>,
    mined_block_sender: flume::Sender<Block>,
    mined_block_receiver: flume::Receiver<Block>,
}

impl Miner {
    pub async fn new(address: String, public_key: PublicKey) -> Result<Self> {
        println!("Connecting to {} to mine with {public_key:?}", address);

        let address = format!("http://{}", address);
        let client = MinerApiClient::connect(address.clone())
            .await
            .with_context(|| format!("failed to connect to server: {}", address))?;

        let (mined_block_sender, mined_block_receiver) = flume::unbounded();

        Ok(Self {
            public_key,
            client,
            current_template: Arc::new(Mutex::new(None)),
            mining: Arc::new(AtomicBool::new(false)),
            mined_block_sender,
            mined_block_receiver,
        })
    }

    pub async fn run(&mut self, cancel_token: CancellationToken) -> Result<()> {
        let handle = self.spawn_mining_thread(cancel_token.clone());

        let mut template_interval = interval(Duration::from_secs(5));
        tokio::select! {
            _ = cancel_token.cancelled() => {}
            _ = async {
                loop {
                    tokio::select! {
                        _ = template_interval.tick() => {
                            _= self.fetch_or_validate_template().await
                                .context("fetching new template").map_err(log_error);
                        }
                        Ok(mined_block) = self.mined_block_receiver.recv_async() => {
                            _= self.submit_mined_block(mined_block).await
                                .context("submit_mined_block").map_err(log_error);
                        }
                    }
                }
            } => {}
        }

        handle.join().unwrap();
        println!("miner stopped");
        Ok(())
    }

    fn spawn_mining_thread(&self, cancel_token: CancellationToken) -> thread::JoinHandle<()> {
        let template = Arc::clone(&self.current_template);
        let mining = Arc::clone(&self.mining);
        let sender = self.mined_block_sender.clone();

        thread::spawn(move || loop {
            if cancel_token.is_cancelled() {
                println!("mining thread terminated");
                return;
            }
            if mining.load(Ordering::Relaxed) {
                if let Some(mut block) = template.lock().unwrap().clone() {
                    println!("Mining block with target: {}", block.header.target);

                    if block.header.mine(2_000_000) {
                        println!("Block mined: {}", block.header.hash());
                        sender.send(block).expect("Failed to send mined block");
                        mining.store(false, Ordering::Relaxed);
                    }
                }
            }
            thread::yield_now();
        })
    }

    async fn fetch_or_validate_template(&mut self) -> Result<()> {
        if !self.mining.load(Ordering::Relaxed) {
            self.fetch_template().await.context("fetch template")?;
        } else {
            self.validate_template()
                .await
                .context("validate template")?;
        }
        Ok(())
    }

    async fn fetch_template(&mut self) -> Result<()> {
        println!("Fetching new template");

        let mut bytes = Vec::new();
        self.public_key
            .save(&mut bytes)
            .context("serialize public key")?;

        let response = self
            .client
            .fetch_template(Request::new(pb::FetchTemplateRequest { pubkey: bytes }))
            .await
            .context("calling fetch_template RPC")?;

        let template_bytes = response.into_inner().cbor;
        let template = Block::load(&template_bytes[..]).context("deserialize block template")?;

        println!(
            "Received new template with target: {}",
            template.header.target
        );
        *self.current_template.lock().unwrap() = Some(template);
        self.mining.store(true, Ordering::Relaxed);

        Ok(())
    }

    async fn validate_template(&mut self) -> Result<()> {
        let current_template = self.current_template.lock().unwrap().clone();

        if let Some(template) = current_template {
            let mut template_bytes = Vec::new();
            template
                .save(&mut template_bytes)
                .context("serializing block template")?;

            let response = self
                .client
                .validate_template(Request::new(pb::Template {
                    cbor: template_bytes,
                }))
                .await
                .context("calling validate_template RPC")?;

            let is_valid = response.into_inner().is_valid;

            if !is_valid {
                println!("Current template is no longer valid");
                self.mining.store(false, Ordering::Relaxed);
            } else {
                println!("Current template is still valid");
            }
        } else {
            println!("No template to validate");
        }

        Ok(())
    }

    async fn submit_mined_block(&mut self, block: Block) -> Result<()> {
        println!("Submitting mined block");

        let mut template_bytes = Vec::new();
        block
            .save(&mut template_bytes)
            .context("serializing block template")?;

        let _response = self
            .client
            .submit_template(Request::new(pb::Template {
                cbor: template_bytes,
            }))
            .await
            .context("calling submit_template RPC")?;

        self.mining.store(false, Ordering::Relaxed);

        Ok(())
    }
}

fn log_error(e: impl std::fmt::Debug) {
    eprintln!("Error: {:?}", e);
}
