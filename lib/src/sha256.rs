use serde::{Deserialize, Serialize};

use crate::U256;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Hash(U256);

impl Hash {
    /// Hashes anything that can be serialized (into CBOR)
    #[allow(clippy::self_named_constructors)]
    pub fn hash<T: serde::Serialize>(data: &T) -> Self {
        let mut serialized = Vec::new();

        if let Err(e) = ciborium::into_writer(&data, &mut serialized) {
            panic!("Failed to serialize data {:?}", e);
        }

        let hash = sha256::digest(serialized);
        let hash_bytes = hex::decode(&hash).expect("SHA256 hash should be valid hex string");
        Self(U256::from_big_endian(&hash_bytes))
    }

    /// Checks if the hash matches a target
    pub fn matches_target(&self, target: U256) -> bool {
        self.0 <= target
    }

    /// Zero hash
    pub fn zero() -> Self {
        Self(U256::zero())
    }

    /// Converts to little-endian encoded 32 bytes
    pub fn as_bytes(&self) -> [u8; 32] {
        self.0.to_little_endian()
    }
}

impl std::fmt::Display for Hash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:x}", self)
    }
}

impl std::fmt::LowerHex for Hash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::LowerHex::fmt(&self.0, f)
    }
}
