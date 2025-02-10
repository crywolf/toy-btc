use std::path::PathBuf;

use anyhow::{Context, Result};
use btclib::crypto::PublicKey;
use serde::{Deserialize, Serialize};

use super::{Key, Recipient};

pub fn generate_config_template(path: &PathBuf) -> Result<()> {
    let dummy_config = Config {
        my_keys: vec![],
        contacts: vec![
            Recipient {
                name: "Alice".to_string(),
                key_path: PathBuf::from("alice.pub.pem"),
            },
            Recipient {
                name: "Bob".to_string(),
                key_path: PathBuf::from("bob.pub.pem"),
            },
        ],
        default_node: "127.0.0.1:9000".to_string(),
        fee_config: FeeConfig {
            fee_type: FeeType::Percent,
            value: 0.1,
        },
    };

    let config_str = toml::to_string_pretty(&dummy_config)?;
    std::fs::write(path, config_str)?;

    println!("Config template generated at: {}", path.display());

    Ok(())
}

/// Type of the miner fee
#[derive(Serialize, Deserialize, Clone)]
pub enum FeeType {
    /// Flat value
    Fixed,
    /// Percentage of the sent amount
    Percent,
}

/// Fee configuration
#[derive(Serialize, Deserialize, Clone)]
pub struct FeeConfig {
    pub fee_type: FeeType,
    pub value: f64,
}

/// Wallet configuration
#[derive(Serialize, Deserialize, Clone)]
pub struct Config {
    /// What are my private and public keys?
    pub my_keys: Vec<Key>,
    /// My contacts - pairs of names and public keys
    pub contacts: Vec<Recipient>,
    /// Default node we want to connect to
    pub default_node: String,
    /// Fee configuration
    pub fee_config: FeeConfig,
}

impl Config {
    /// Loads config from file
    pub fn load_from_file(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let config_str = std::fs::read_to_string(&path)
            .with_context(|| format!("read config file '{}'", path.as_ref().display()))?;

        let config: Config = toml::from_str(&config_str).context("deserialize config from TOML")?;

        if config.my_keys.is_empty() {
            anyhow::bail!("No private keys found in config");
        }

        Ok(config)
    }

    /// Returns recipient's public key
    pub fn recipient_pubkey(&self, name: &str) -> Result<PublicKey> {
        let key = self
            .contacts
            .iter()
            .find(|r| r.name == name)
            .ok_or_else(|| anyhow::anyhow!("Recipient not found"))?
            .load()
            .context("load recipient key")?
            .key;

        Ok(key)
    }
}
