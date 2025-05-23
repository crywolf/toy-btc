mod cli;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use anyhow::{anyhow, bail, Context, Result};
use btclib::blockchain::Block;
use btclib::crypto::PublicKey;
use btclib::network::Message;
use btclib::Saveable;
use tokio::net::TcpStream;
use tokio::signal::unix::{signal, SignalKind};
use tokio::time::{interval, Duration};
use tokio_util::sync::CancellationToken;

struct Miner {
    public_key: PublicKey,
    stream: tokio::sync::Mutex<TcpStream>,
    current_template: Arc<Mutex<Option<Block>>>,
    mining: Arc<AtomicBool>,
    mined_block_sender: flume::Sender<Block>,
    mined_block_receiver: flume::Receiver<Block>,
}

impl Miner {
    async fn new(address: String, public_key: PublicKey) -> Result<Self> {
        println!("Connecting to {} to mine with {public_key:?}", address);

        let stream = TcpStream::connect(&address)
            .await
            .context(format!("failed to connect to node: {}", address))?;

        let (mined_block_sender, mined_block_receiver) = flume::unbounded();

        Ok(Self {
            public_key,
            stream: tokio::sync::Mutex::new(stream),
            current_template: Arc::new(Mutex::new(None)),
            mining: Arc::new(AtomicBool::new(false)),
            mined_block_sender,
            mined_block_receiver,
        })
    }

    async fn run(&self, cancel_token: CancellationToken) -> Result<()> {
        let handle = self.spawn_mining_thread(cancel_token.clone());

        let mut template_interval = interval(Duration::from_secs(5));
        tokio::select! {
            _ = cancel_token.cancelled() => {}
            res = async {
                loop {
                    tokio::select! {
                        _ = template_interval.tick() => {
                            self.fetch_or_validate_template().await?;
                        }
                        Ok(mined_block) = self.mined_block_receiver.recv_async() => {
                            self.submit_mined_block(mined_block).await?;
                        }
                    }
                }
                // help the rust type inferencer out
                #[allow(unreachable_code)]
                Ok::<_, anyhow::Error>(())
            } => {
                res?;
            }
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

    async fn fetch_or_validate_template(&self) -> Result<()> {
        if !self.mining.load(Ordering::Relaxed) {
            self.fetch_template().await.context("fetch template")?;
        } else {
            self.validate_template()
                .await
                .context("validate template")?;
        }
        Ok(())
    }

    async fn fetch_template(&self) -> Result<()> {
        println!("Fetching new template");

        let message = Message::FetchTemplate(self.public_key.clone());
        let mut stream_lock = self.stream.lock().await;
        message
            .send_async(&mut *stream_lock)
            .await
            .context("send FetchTemplate message")?;

        match Message::receive_async(&mut *stream_lock)
            .await
            .context("receive message")?
        {
            Message::Template(template) => {
                drop(stream_lock);
                println!(
                    "Received new template with target: {}",
                    template.header.target
                );
                *self.current_template.lock().unwrap() = Some(template);
                self.mining.store(true, Ordering::Relaxed);
                Ok(())
            }
            _ => bail!("Unexpected message received when fetching template"),
        }
    }

    async fn validate_template(&self) -> Result<()> {
        let current_template = self.current_template.lock().unwrap().clone();

        if let Some(template) = current_template {
            let message = Message::ValidateTemplate(template);
            let mut stream_lock = self.stream.lock().await;
            message
                .send_async(&mut *stream_lock)
                .await
                .context("send ValidateTemplate message")?;

            match Message::receive_async(&mut *stream_lock)
                .await
                .context("receive message")?
            {
                Message::TemplateValidity(valid) => {
                    drop(stream_lock);
                    if !valid {
                        println!("Current template is no longer valid");
                        self.mining.store(false, Ordering::Relaxed);
                    } else {
                        println!("Current template is still valid");
                    }
                    Ok(())
                }
                _ => bail!("Unexpected message received when validating template"),
            }
        } else {
            println!("No template to validate");
            Ok(())
        }
    }

    async fn submit_mined_block(&self, block: Block) -> Result<()> {
        println!("Submitting mined block");

        let message = Message::SubmitTemplate(block);
        let mut stream_lock = self.stream.lock().await;
        message
            .send_async(&mut *stream_lock)
            .await
            .context("send SubmitTemplate message")?;

        self.mining.store(false, Ordering::Relaxed);

        Ok(())
    }
}

#[tokio::main]
pub async fn main() -> Result<()> {
    let cli = cli::parse();

    let public_key = PublicKey::load_from_file(&cli.public_key_file)
        .map_err(|e| anyhow!("Error reading public key: {}", e))?;

    // Graceful shutdown
    let token = CancellationToken::new();
    let cancel_token = token.clone();
    tokio::spawn(async move {
        let mut hup = signal(SignalKind::hangup()).unwrap();
        let mut term = signal(SignalKind::terminate()).unwrap();
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                println!("\n>>> ctrl-c received!");
                token.cancel();
            }
           _= hup.recv() => {
                println!(">>> got signal HUP");
                token.cancel();
            }
            _= term.recv() => {
                println!(">>> got signal TERM");
                token.cancel();
            }
        }
    });

    let miner = Miner::new(cli.address, public_key).await?;
    miner.run(cancel_token).await
}
