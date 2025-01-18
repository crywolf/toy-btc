use serde::{Deserialize, Serialize};

use crate::{blockchain::Tx, sha256::Hash};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct MerkleRoot(Hash);

impl MerkleRoot {
    /// Calculate the Merkle root of a block's transactions
    pub fn calculate(transactions: &[Tx]) -> Self {
        let mut layer = Vec::new();
        for tx in transactions {
            layer.push(Hash::hash(tx));
        }

        if layer.is_empty() {
            return Self(Hash::zero()); // or error?
        }

        while layer.len() > 1 {
            let mut new_layer = Vec::new();
            for pair in layer.chunks(2) {
                let left = pair[0];
                // if there is no right, use the left hash again
                let right = *pair.get(1).unwrap_or(&left);
                new_layer.push(Hash::hash(&[left, right]));
            }
            layer = new_layer;
        }

        Self(layer[0])
    }
}
