use std::sync::Arc;

use btclib::{blockchain::TxOutput, crypto::PublicKey};
use crossbeam_skiplist::SkipMap;

use super::LoadedKey;

#[derive(Clone, Default)]
pub struct UtxoStore {
    /// My pub and priv key pair
    pub my_keys: Vec<LoadedKey>,
    /// My utxos
    pub utxos: Arc<SkipMap<PublicKey, Vec<(bool, TxOutput)>>>,
}

impl UtxoStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_key(&mut self, key: LoadedKey) {
        self.my_keys.push(key);
    }
}
