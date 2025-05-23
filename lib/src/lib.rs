pub mod blockchain;
pub mod crypto;
pub mod error;
pub mod merkle_root;
pub mod network;
pub mod sha256;

use bigdecimal::{BigDecimal, FromPrimitive, ToPrimitive};
use uint::construct_uint;

// Unsigned 256-bit integer consisting of 4 x 64-bit words (little-endian)
construct_uint! {
    #[derive(serde::Serialize, serde::Deserialize)]
    pub struct U256(4);
}

/// Initial reward in bitcoin - multiply by 10^8 to get sats
pub const INITIAL_REWARD: u64 = 50;
/// Halving interval in blocks
pub const HALVING_INTERVAL: u64 = 210;
/// Ideal block time in seconds
pub const IDEAL_BLOCK_TIME: u64 = 10; // 600 s in real BTC
/// Minimum target (little-endian: the first 2 bytes are zero)
pub const MIN_TARGET: U256 = U256([
    0xFFFF_FFFF_FFFF_FFFF,
    0xFFFF_FFFF_FFFF_FFFF,
    0xFFFF_FFFF_FFFF_FFFF,
    0x0000_FFFF_FFFF_FFFF,
]);
/// Difficulty update interval in blocks
pub const DIFFICULTY_UPDATE_INTERVAL: u64 = 50; // 2016 blocks in real BTC
/// Maximum mempool transaction age in seconds
pub const MAX_MEMPOOL_TRANSACTION_AGE: u64 = 600; // 72 hours in real BTC
/// Maximum amount of transactions allowed in a block
pub const BLOCK_TRANSACTION_CAP: usize = 20;

/// Convert bitcoins to sats
pub fn btc_to_sats(btc: f64) -> u64 {
    (btc * (10u64.pow(8) as f64)) as u64
}

/// Convert sats to bitcoins
pub fn sats_to_btc(sats: u64) -> f64 {
    let sats = BigDecimal::from_u64(sats).unwrap();
    let sats_per_btc = BigDecimal::from_f64(100_000_000.0).unwrap();
    (sats / sats_per_btc).to_f64().unwrap()
}

/// Serializable trait
pub trait Serializable
where
    Self: Sized,
{
    /// Deserialize from reader
    fn deserialize<R: std::io::Read>(reader: R) -> std::io::Result<Self>;

    /// Serialize into writer
    fn serialize<W: std::io::Write>(&self, writer: W) -> std::io::Result<()>;
}

/// Saveable trait - save and load from file
pub trait Saveable: Serializable {
    // Default implementations:
    fn load_from_file<P: AsRef<std::path::Path>>(path: P) -> std::io::Result<Self> {
        let file = std::fs::File::open(&path)?;
        Self::deserialize(file)
    }
    fn save_to_file<P: AsRef<std::path::Path>>(&self, path: P) -> std::io::Result<()> {
        let file = std::fs::File::create(&path)?;
        self.serialize(file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_btc_to_sats() {
        assert_eq!(btc_to_sats(6.25), 625000000);
    }

    #[test]
    fn test_sats_to_btc() {
        assert_eq!(sats_to_btc(625000000), 6.25);
    }
}
