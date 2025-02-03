pub mod blockchain;
pub mod crypto;
pub mod error;
pub mod merkle_root;
pub mod network;
pub mod sha256;

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
pub fn btc_to_sats(btc: u64) -> u64 {
    btc * 10u64.pow(8)
}

/// Convert sats to bitcoins
pub fn sats_to_btc(sats: u64) -> f64 {
    sats as f64 / 100_000_000.0
}

/// Saveable trait - save and load from file
pub trait Saveable
where
    Self: Sized,
{
    fn load<R: std::io::Read>(reader: R) -> std::io::Result<Self>;
    fn save<W: std::io::Write>(&self, writer: W) -> std::io::Result<()>;
    // Default implementations:
    fn load_from_file<P: AsRef<std::path::Path>>(path: P) -> std::io::Result<Self> {
        let file = std::fs::File::open(&path)?;
        Self::load(file)
    }
    fn save_to_file<P: AsRef<std::path::Path>>(&self, path: P) -> std::io::Result<()> {
        let file = std::fs::File::create(&path)?;
        self.save(file)
    }
}
