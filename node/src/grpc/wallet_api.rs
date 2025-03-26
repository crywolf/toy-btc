use std::sync::Arc;

use anyhow::{Context, Result};
use btclib::blockchain::Tx;
use btclib::crypto::PublicKey;
use btclib::Serializable;

use tonic::{Request, Response, Status};

use super::log_error;
use super::node_api::SubscriptionItem;
use super::peers::Peers;
use super::wallet::pb;
use super::wallet::pb::wallet_api_server::{WalletApi, WalletApiServer};
use crate::blockchain::BLOCKCHAIN;

pub struct WalletSvc {
    peers: Arc<Peers>,
}

impl WalletSvc {
    pub fn new(peers: Arc<Peers>) -> Self {
        Self { peers }
    }
}

#[tonic::async_trait]
impl WalletApi for WalletSvc {
    /// Fetch all UTXOs belonging to a public key
    async fn fetch_utxos(
        &self,
        request: Request<pb::FetchUtxosRequest>,
    ) -> Result<Response<pb::FetchUtxosResponse>, Status> {
        println!("gRPC: received request to fetch UTXOs");
        let pubkey_bytes = request.into_inner().pubkey;
        let pubkey = PublicKey::deserialize(&pubkey_bytes[..])?;

        let blockchain = BLOCKCHAIN.read().await;
        let utxos = blockchain
            .utxo_set_for_pubkey(pubkey)
            .map(|(_, (marked, txout))| (txout, *marked))
            .map(|(txout, marked)| {
                let mut txout_bytes = Vec::new();
                if let Err(e) = txout
                    .serialize(&mut txout_bytes)
                    .context("serialize transaction output")
                {
                    log_error(&e);
                }

                pb::Utxo {
                    tx_output: txout_bytes,
                    in_mempool: marked,
                }
            })
            .collect::<Vec<_>>();

        Ok(Response::new(pb::FetchUtxosResponse { utxos }))
    }

    /// Send a transaction to the network
    async fn submit_transaction(
        &self,
        request: Request<pb::Transaction>,
    ) -> Result<Response<pb::Empty>, Status> {
        let bytes = request.into_inner().cbor;
        let transaction = Tx::deserialize(&bytes[..])?;

        println!(
            "gRPC: new transaction was submitted, {}",
            transaction.hash()
        );

        let mut blockchain = BLOCKCHAIN.write().await;

        if let Err(e) = blockchain.add_to_mempool(transaction.clone()) {
            log_error(&e);
            return Err(Status::unknown(e.to_string()));
        }
        println!("new transaction was added to mempool");

        // broadcast the transaction to all known nodes
        println!("broadcasting new transaction to all subscribers");
        if let Err(e) = self
            .peers
            .broadcast(SubscriptionItem::Transaction(transaction), None)
            .await
            .context("broadcasting new block")
        {
            log_error(e);
        }

        Ok(Response::new(pb::Empty {}))
    }
}

pub fn create_server(peers: Arc<Peers>) -> WalletApiServer<WalletSvc> {
    WalletApiServer::new(WalletSvc::new(peers))
}
